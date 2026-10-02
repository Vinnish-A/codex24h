//! Optional native completion notifications; no SMTP work runs in the PTY loop.
use std::{
    env,
    ffi::OsString,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

pub struct Watcher {
    child: Child,
    pub path: PathBuf,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGTERM);
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
        if daemon
            && !crate::launch::has_option(&args, "--no-daemon")
            && !crate::launch::has_option(&args, "--remote")
        {
            let mut command = Command::new(&helper);
            command
                .arg("watch")
                .arg("--config")
                .arg(&config)
                .arg("--codex-home")
                .arg(&home)
                .stdout(Stdio::piped());
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
        // Last config override wins, but it must remain before a literal prompt delimiter.
        let index = args
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(args.len());
        args.splice(
            index..index,
            [OsString::from("-c"), OsString::from(notification)],
        );
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
