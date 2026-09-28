use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

/// Find the real Codex binary without accidentally launching this wrapper again.
/// An explicit override is authoritative; otherwise PATH takes precedence over
/// the usual user-local installation.
pub fn resolve_codex() -> io::Result<PathBuf> {
    let current = env::current_exe()?.canonicalize()?;
    if let Some(override_path) = env::var_os("CODEX24H_CODEX") {
        return executable(&override_path, &current)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "CODEX24H_CODEX does not name an executable Codex binary",
            )
        });
    }

    if let Some(found) = executable(OsStr::new("codex"), &current)? {
        return Ok(found);
    }
    if let Some(home) = env::var_os("HOME") {
        let path = PathBuf::from(home).join(".local/bin/codex");
        if let Some(found) = usable_path(&path, &current) {
            return Ok(found);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Codex executable not found; set CODEX24H_CODEX to its path",
    ))
}

fn executable(name: &OsStr, current: &Path) -> io::Result<Option<PathBuf>> {
    let path = Path::new(name);
    if path.components().count() > 1 || path.is_absolute() {
        return Ok(usable_path(path, current));
    }
    let Some(search_path) = env::var_os("PATH") else {
        return Ok(None);
    };
    for directory in env::split_paths(&search_path) {
        if let Some(found) = usable_path(&directory.join(name), current) {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn usable_path(path: &Path, current: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let resolved = path.canonicalize().ok()?;
    let metadata = resolved.metadata().ok()?;
    (resolved != current && metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .then_some(resolved)
}

/// Whether these Codex arguments would enter its interactive TUI. This parser
/// only needs to identify the root command; Codex itself remains responsible
/// for validating every argument.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Invocation {
    Interactive,
    Batch,
    Other,
}

pub fn interactive(args: &[OsString]) -> bool {
    invocation(args) == Invocation::Interactive
}

pub fn notifying(args: &[OsString]) -> bool {
    invocation(args) != Invocation::Other
}

fn invocation(args: &[OsString]) -> Invocation {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            return Invocation::Interactive; // Everything afterward is the root prompt.
        }
        if arg == "--help" || arg == "-h" || arg == "--version" || arg == "-V" {
            return Invocation::Other;
        }
        if arg == "--no-alt-screen" {
            index += 1;
            continue;
        }
        if arg == "-i" || arg == "--image" {
            index += 2;
            while index < args.len()
                && args[index] != "--"
                && !args[index].to_string_lossy().starts_with('-')
            {
                index += 1;
            }
            continue;
        }
        if takes_value(arg) {
            index += 2;
            continue;
        }
        if has_attached_value(arg) || is_flag(arg) {
            index += 1;
            continue;
        }
        if arg.to_string_lossy().starts_with('-') {
            // Unknown future options are passed through unchanged. Avoid
            // guessing whether their following argument is a command.
            return Invocation::Other;
        }
        if arg == "resume" || arg == "fork" {
            return if requests_help(&args[index + 1..]) {
                Invocation::Other
            } else {
                Invocation::Interactive
            };
        }
        return if requests_help(&args[index + 1..]) {
            Invocation::Other
        } else if matches!(arg.to_str(), Some("exec" | "e" | "review")) {
            Invocation::Batch
        } else if is_command(arg) {
            Invocation::Other
        } else {
            Invocation::Interactive
        };
    }
    Invocation::Interactive
}

fn requests_help(args: &[OsString]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            break;
        }
        if arg == "--help" || arg == "-h" || arg == "--version" || arg == "-V" {
            return true;
        }
        index += if takes_value(arg) { 2 } else { 1 };
    }
    false
}

fn has_no_alt_screen(args: &[OsString]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            break;
        }
        if arg == "--no-alt-screen" {
            return true;
        }
        index += if takes_value(arg) { 2 } else { 1 };
    }
    false
}

fn takes_value(arg: &OsStr) -> bool {
    matches!(
        arg.to_str(),
        Some(
            "-c" | "--config"
                | "--enable"
                | "--disable"
                | "--remote"
                | "--remote-auth-token-env"
                | "-i"
                | "--image"
                | "-m"
                | "--model"
                | "--local-provider"
                | "-p"
                | "--profile"
                | "-s"
                | "--sandbox"
                | "-C"
                | "--cd"
                | "--add-dir"
                | "-a"
                | "--ask-for-approval"
        )
    )
}

fn has_attached_value(arg: &OsStr) -> bool {
    let Some(value) = arg.to_str() else {
        return false;
    };
    [
        "--config=",
        "--enable=",
        "--disable=",
        "--remote=",
        "--remote-auth-token-env=",
        "--image=",
        "--model=",
        "--local-provider=",
        "--profile=",
        "--sandbox=",
        "--cd=",
        "--add-dir=",
        "--ask-for-approval=",
    ]
    .iter()
    .any(|prefix| value.starts_with(prefix))
        || ["-c", "-i", "-m", "-p", "-s", "-C", "-a"]
            .iter()
            .any(|prefix| value.starts_with(prefix) && value.len() > prefix.len())
}

fn is_flag(arg: &OsStr) -> bool {
    matches!(
        arg.to_str(),
        Some(
            "--strict-config"
                | "--oss"
                | "--approve-for-me"
                | "--dangerously-bypass-approvals-and-sandbox"
                | "--yolo"
                | "--dangerously-bypass-hook-trust"
                | "--worktree"
                | "--search"
                | "--no-daemon"
        )
    )
}

fn is_command(arg: &OsStr) -> bool {
    matches!(
        arg.to_str(),
        Some(
            "agents"
                | "exec"
                | "e"
                | "review"
                | "login"
                | "logout"
                | "mcp"
                | "plugin"
                | "app-server"
                | "remote-control"
                | "completion"
                | "update"
                | "doctor"
                | "sandbox"
                | "debug"
                | "apply"
                | "a"
                | "queue"
                | "archive"
                | "delete"
                | "migrate-rollouts"
                | "unarchive"
                | "cloud"
                | "exec-server"
                | "features"
                | "help"
        )
    )
}

/// Preserve the caller's argument bytes and order, adding inline mode only for
/// an interactive Codex invocation that has not already requested it.
pub fn wrapped_args(args: &[OsString]) -> Vec<OsString> {
    if !interactive(args) || has_no_alt_screen(args) {
        return args.to_vec();
    }
    let mut result = Vec::with_capacity(args.len() + 1);
    result.push(OsString::from("--no-alt-screen"));
    result.extend_from_slice(args);
    result
}

#[cfg(test)]
mod tests {
    use super::{interactive, wrapped_args};
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn classifies_root_options_and_commands() {
        assert!(interactive(&args(&[])));
        assert!(interactive(&args(&[
            "-c",
            "model=exec",
            "-C",
            "exec",
            "hello"
        ])));
        assert!(interactive(&args(&[
            "--config=foo=exec",
            "resume",
            "--last"
        ])));
        assert!(interactive(&args(&["-i", "photo.png", "exec"])));
        assert!(interactive(&args(&["--", "exec"])));
        assert!(interactive(&args(&["a prompt", "--", "--help"])));
        assert!(!interactive(&args(&["a prompt", "--help"])));
        assert!(!interactive(&args(&["--remote", "exec", "exec", "--json"])));
        assert!(!interactive(&args(&["-mexec", "review"])));
        assert!(!interactive(&args(&["exec", "--json"])));
        assert!(!interactive(&args(&["--yolo", "exec", "--json"])));
        assert!(interactive(&args(&["--yolo", "resume", "--last"])));
        assert!(!interactive(&args(&["resume", "--help"])));
        assert!(!interactive(&args(&["-h"])));
        assert!(!interactive(&args(&["--version"])));
    }

    #[test]
    fn injects_only_for_interactive_calls() {
        assert_eq!(
            wrapped_args(&args(&["resume", "--last"])),
            args(&["--no-alt-screen", "resume", "--last"])
        );
        assert_eq!(
            wrapped_args(&args(&["--no-alt-screen", "fork"])),
            args(&["--no-alt-screen", "fork"])
        );
        assert_eq!(
            wrapped_args(&args(&["--", "--no-alt-screen"])),
            args(&["--no-alt-screen", "--", "--no-alt-screen"])
        );
        assert_eq!(
            wrapped_args(&args(&["exec", "--json"])),
            args(&["exec", "--json"])
        );
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_prompt() {
        use std::os::unix::ffi::OsStringExt;

        let prompt = OsString::from_vec(vec![b'p', 0xff]);
        assert_eq!(
            wrapped_args(&[prompt.clone()]),
            vec![OsString::from("--no-alt-screen"), prompt]
        );
    }
}
