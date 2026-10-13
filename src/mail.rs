//! Optional native completion notifications; no SMTP work runs in the PTY loop.
use std::{
    env,
    ffi::OsString,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Watcher {
    child: Child,
    pub path: PathBuf,
}

impl Watcher {
    pub fn running(&mut self) -> std::io::Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }

    fn write_control(&self, name: &str, value: serde_json::Value) -> std::io::Result<()> {
        let path = self.path.with_file_name(name);
        let pending = path.with_extension("next");
        fs::write(&pending, serde_json::to_vec(&value)?)?;
        fs::rename(pending, path)
    }

    pub fn capacity(&self, pid: u32, generation: u64) -> std::io::Result<()> {
        self.write_control(
            "native-capacity.json",
            serde_json::json!({
                "pid": pid, "generation": generation,
                "at": SystemTime::now().duration_since(UNIX_EPOCH)
                    .map_err(std::io::Error::other)?.as_nanos().to_string(),
            }),
        )
    }

    pub fn frontend(&self, generation: u64, input_at_ms: u128, ready: bool) -> std::io::Result<()> {
        self.write_control(
            "frontend.json",
            serde_json::json!({
                "generation": generation, "input_at_ms": input_at_ms, "ready": ready,
            }),
        )
    }

    /// Called after reading human input and native output. Consume each command
    /// once; the acknowledgment is written only after the PTY accepted its bytes.
    pub fn recovery(
        &self,
        generation: u64,
        ready: bool,
        fd: i32,
    ) -> std::io::Result<Option<(String, String)>> {
        let path = self.path.with_file_name("recovery-request.json");
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let request: serde_json::Value = serde_json::from_slice(&bytes)?;
        fs::remove_file(path)?;
        let command = request["command"].as_str().unwrap_or_default();
        let status = if ready
            && request["generation"].as_u64() == Some(generation)
            && matches!(command, "go on" | "/goal resume" | "/status")
        {
            let input = format!("\x1b[200~{command}\x1b[201~\r");
            match crate::terminal::write(fd, input.as_bytes()) {
                Ok(n) if n == input.len() => "written",
                // Never replay an uncertain/partial submission.
                _ => "unconfirmed",
            }
        } else {
            "cancelled"
        };
        self.write_control(
            "recovery-ack.json",
            serde_json::json!({
                "key": request["key"], "status": status,
            }),
        )?;
        Ok((status == "written").then(|| {
            (
                request["key"].as_str().unwrap_or_default().to_owned(),
                command.to_owned(),
            )
        }))
    }

    /// Follow a session explicitly confirmed in the native resume picker.
    pub fn bind_session(
        &self,
        session: &str,
        generation: u64,
        identity: bool,
    ) -> std::io::Result<()> {
        let record = serde_json::to_vec(&serde_json::json!({
            "dir": "codex24h", "kind": "resume", "session": session,
            "generation": generation,
            "since": if identity { 0 } else { SystemTime::now().duration_since(UNIX_EPOCH)
                .map_err(std::io::Error::other)?.as_secs() },
        }))?;
        // Keep control records separate from Codex's potentially large FIFO
        // writes: two pipe writers could interleave a JSON event mid-line.
        let pending = self.path.with_file_name("scope.next");
        fs::write(&pending, record)?;
        fs::rename(pending, self.path.with_file_name("scope.json"))
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            unsafe {
                libc::kill(self.child.id() as i32, libc::SIGTERM);
            }
        }
        let _ = self.child.wait();
    }
}

pub fn arguments(
    mut args: Vec<OsString>,
    exe: &Path,
    pty: bool,
) -> Result<(Vec<OsString>, Option<Watcher>), Box<dyn std::error::Error>> {
    if env::var_os("CODEX24H_MAIL").is_some_and(|v| v == "0") || !crate::launch::notifying(&args) {
        return Ok((args, None));
    }
    let config = env::var_os("CODEX24H_MAIL_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".config")
                })
                .join("codex24h/mail.toml")
        });
    if !config.try_exists()? {
        if env::var_os("CODEX24H_MAIL_CONFIG").is_some() {
            return Err(format!("mail config not found: {}", config.display()).into());
        }
        return Ok((args, None));
    }
    let mut helper = env::current_exe()?.with_file_name("codex24h-mail");
    if !helper.is_file() {
        helper = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/codex24h-mail");
    }
    let home = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".codex"));
    let result = Command::new(&helper)
        .arg("prepare")
        .arg("--config")
        .arg(&config)
        .arg("--codex-home")
        .arg(&home)
        .arg("--")
        .args(&args)
        .output()?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr)
            .trim()
            .to_owned()
            .into());
    }
    let notification = String::from_utf8(result.stdout)?.trim().to_owned();
    if !notification.is_empty() {
        // CLI config overrides select an in-process server in daemon-era Codex.
        // Observe this TUI instead of changing its default connection or agents UI.
        let daemon = pty
            && String::from_utf8_lossy(&Command::new(exe).arg("--help").output()?.stdout)
                .contains("--no-daemon");
        let shared = daemon
            && !crate::launch::has_option(&args, "--no-daemon")
            && !crate::launch::has_option(&args, "--remote");
        if !shared {
            let index = args
                .iter()
                .position(|arg| arg == "--")
                .unwrap_or(args.len());
            args.splice(
                index..index,
                [OsString::from("-c"), OsString::from(notification)],
            );
        }
        if pty && !crate::launch::has_option(&args, "--remote") {
            let mut command = Command::new(&helper);
            command
                .arg("watch")
                .arg("--config")
                .arg(&config)
                .arg("--codex-home")
                .arg(&home)
                .stdout(Stdio::piped());
            if !shared {
                command.arg("--standalone");
            }
            if let Some(session) = crate::launch::resume_target(&args) {
                command.arg("--session").arg(session);
            }
            let mut child = command.spawn()?;
            let mut path = String::new();
            BufReader::new(child.stdout.take().unwrap()).read_line(&mut path)?;
            if path.trim().is_empty() {
                child.wait()?;
                return Err("mail watcher did not provide its event pipe".into());
            }
            return Ok((
                args,
                Some(Watcher {
                    child,
                    path: PathBuf::from(path.trim()),
                }),
            ));
        }
    }
    Ok((args, None))
}

/// SMTP and session lookup run off the PTY loop, including waiting/reaping the helper.
pub fn capacity(pid: u32) {
    if env::var_os("CODEX24H_MAIL").is_some_and(|v| v == "0") {
        return;
    }
    std::thread::spawn(move || {
        let Ok(binary) = env::current_exe() else {
            return;
        };
        let mut helper = binary.with_file_name("codex24h-mail");
        if !helper.is_file() {
            helper = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/codex24h-mail");
        }
        let result = Command::new(helper)
            .arg("capacity")
            .arg("--pid")
            .arg(pid.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .output();
        if let Ok(output) = result {
            if !output.status.success() {
                eprintln!("{}", String::from_utf8_lossy(&output.stderr).trim());
            }
        }
    });
}
