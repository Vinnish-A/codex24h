//! Optional native completion notifications; no SMTP work runs in the PTY loop.
use std::{env, ffi::OsString, path::PathBuf, process::Command};

pub fn arguments(mut args: Vec<OsString>) -> Result<Vec<OsString>, Box<dyn std::error::Error>> {
    if env::var_os("CODEX24H_MAIL").is_some_and(|v| v == "0") || !crate::launch::notifying(&args) {
        return Ok(args);
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
        return Ok(args);
    }
    let mut helper = env::current_exe()?.with_file_name("codex24h-mail");
    if !helper.is_file() {
        helper = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/codex24h-mail");
    }
    let home = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".codex"));
    let result = Command::new("python3")
        .arg(helper)
        .arg("prepare")
        .arg("--config")
        .arg(config)
        .arg("--codex-home")
        .arg(home)
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
    Ok(args)
}
