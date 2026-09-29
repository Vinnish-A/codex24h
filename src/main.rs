use codex24h::{
    core::{Core, Mode, Size},
    input::{Action, Router},
    launch,
    render::Renderer,
    terminal,
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    env, io,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("codex24h: {e}");
            1
        }
    };
    std::process::exit(code);
}
fn number(name: &str, default: usize, min: usize, max: usize) -> Result<usize> {
    match env::var(name) {
        Ok(v) => {
            let n: usize = v
                .parse()
                .map_err(|_| format!("{name} must be an integer"))?;
            if !(min..=max).contains(&n) {
                return Err(format!("{name} must be between {min} and {max}").into());
            }
            Ok(n)
        }
        Err(env::VarError::NotPresent) => Ok(default),
        Err(e) => Err(e.into()),
    }
}
fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows as u16,
        cols: size.cols as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}
fn run() -> Result<i32> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg == "attach" || arg == "session")
    {
        let name = if args[0] == "session" {
            "codex24h-session"
        } else {
            "codex24h-attach"
        };
        let binary = env::current_exe()?;
        let mut helper = binary.with_file_name(name);
        if !helper.is_file() {
            helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("scripts")
                .join(name);
        }
        return Err(Command::new("python3")
            .arg(helper)
            .arg("--wrapper")
            .arg(binary)
            .args(&args[1..])
            .exec()
            .into());
    }
    let attached = env::var_os("CODEX24H_TMUX_CLIENT").is_some();
    let attached_full_screen = env::var_os("CODEX24H_ATTACH_FULL_SCREEN").is_some_and(|v| v == "1");
    let mut tmux_input = codex24h::tmux_input::TmuxInput::from_env()?;
    let args = codex24h::mail::arguments(args)?;
    let exe = launch::resolve_codex()?;
    if !terminal::is_tty(0) || !terminal::is_tty(1) || !launch::interactive(&args) {
        return Err(Command::new(exe).args(args).exec().into());
    }
    let history = number("CODEX24H_HISTORY", 10_000, 1, 1_000_000)?;
    let pin_rows = number("CODEX24H_PIN_ROWS", 6, 0, 100)?;
    let escape_ms = number("CODEX24H_ESCAPE_MS", 100, 10, 2000)?;
    let mut outer = terminal::Terminal::raw()?;
    let caps = outer.discover()?;
    let size = terminal::size()?;
    let pair = native_pty_system().openpty(pty_size(size))?;
    let mut cmd = CommandBuilder::new(exe);
    cmd.args(launch::wrapped_args(&args));
    cmd.cwd(env::current_dir()?);
    cmd.env("TERM", "xterm-256color");
    let fd = pair
        .master
        .as_raw_fd()
        .ok_or("this build requires a Unix PTY")?;
    terminal::set_nonblocking(fd)?;
    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);
    let mut final_screen = None;
    let result = (|| -> Result<i32> {
        outer.enter(caps.kitty)?;
        let mut core = Core::new(size, history, caps.kitty, caps.foreground, caps.background);
        core.pin_rows = pin_rows;
        let mut router = Router::new();
        router.set_detachable(attached);
        let mut renderer = Renderer::new();
        let mut router_mode = core.input_mode();
        let mut child_write = Vec::new();
        let mut child_pos = 0usize;
        let mut display = Vec::new();
        let mut display_pos = 0usize;
        let mut last_input = Instant::now();
        let mut last_frame = Instant::now() - Duration::from_secs(1);
        let mut last_draw = last_frame;
        let mut urgent = true;
        let terminate = Arc::new(AtomicUsize::new(0));
        let resize = Arc::new(AtomicBool::new(false));
        let suspend = Arc::new(AtomicBool::new(false));
        let mut registrations = Vec::new();
        for signal in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT] {
            registrations.push(signal_hook::flag::register_usize(
                signal,
                terminate.clone(),
                signal as usize,
            )?);
        }
        registrations.push(signal_hook::flag::register(libc::SIGWINCH, resize.clone())?);
        registrations.push(signal_hook::flag::register(libc::SIGTSTP, suspend.clone())?);
        struct Signals(Vec<signal_hook::SigId>);
        impl Drop for Signals {
            fn drop(&mut self) {
                for id in self.0.drain(..) {
                    signal_hook::low_level::unregister(id);
                }
            }
        }
        let _signals = Signals(registrations);
        let mut eof = false;
        let mut exit_code = None;
        let mut exit_at = None;
        route(
            &mut router,
            &mut core,
            &mut router_mode,
            &caps.pending,
            None,
            &mut tmux_input,
        );
        let mut buf = [0u8; 65536];
        loop {
            if router.detach_requested() {
                return Ok(0);
            }
            let signal = terminate.load(Ordering::Relaxed);
            if signal != 0 {
                return Ok(128 + signal as i32);
            }
            if suspend.swap(false, Ordering::Relaxed) {
                if let Some(pid) = child.process_id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGSTOP);
                    }
                }
                outer.suspend()?;
                display.clear();
                display_pos = 0;
                unsafe {
                    libc::raise(libc::SIGSTOP);
                }
                outer.resume()?;
                if let Some(pid) = child.process_id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGCONT);
                    }
                }
                core.invalidate_modes();
                renderer.invalidate();
                resize.store(true, Ordering::Relaxed);
                urgent = true;
            }
            if resize.swap(false, Ordering::Relaxed) {
                match terminal::size() {
                    Ok(new) => {
                        pair.master.resize(pty_size(new))?;
                        core.resize(new);
                        renderer.invalidate();
                        urgent = true;
                    }
                    // A remote client can briefly report no usable cells while
                    // resizing. Keep the child and its last valid size alive.
                    Err(e) if e.kind() == io::ErrorKind::InvalidInput => {}
                    Err(e) => return Err(e.into()),
                }
            }
            let now = Instant::now();
            if router.has_pending()
                && now.duration_since(last_input) >= Duration::from_millis(escape_ms as u64)
            {
                for action in router.flush_timeout() {
                    apply_input(&mut core, &mut tmux_input, action);
                }
                urgent = true;
            }
            if core.input_mode() != router_mode {
                router_mode = core.input_mode();
                router.set_mode(router_mode);
            }
            let frame_due = urgent || now.duration_since(last_frame) >= Duration::from_millis(33);
            if frame_due {
                core.tick();
                last_frame = now;
            }
            if child_pos == child_write.len() {
                child_write.clear();
                child_pos = 0;
            }
            child_write.append(&mut core.pty_out);
            if display_pos == display.len() {
                display = core.controls();
                display_pos = 0;
                if core.dirty
                    && frame_due
                    && (urgent
                        || core.mode == Mode::Follow
                        || (core.mode == Mode::Browse && core.pin_rows > 0)
                        || now.duration_since(last_draw) >= Duration::from_millis(250))
                {
                    let mut frame = core.frame();
                    if attached && core.mode == Mode::Follow {
                        frame.status = if attached_full_screen {
                            "codex24h attached · full-screen: history limited · Ctrl+] d: detach"
                        } else {
                            "codex24h attached · wheel: history · Ctrl+] d: detach"
                        }
                        .into();
                    }
                    if let Some(code) = exit_code {
                        frame.status = format!(
                            "Codex exited ({code}) · q / Enter: close · Ctrl+] b: final screen"
                        );
                    }
                    display.extend(renderer.render(&frame));
                    core.dirty = false;
                    last_draw = now;
                    urgent = false;
                }
            }
            if let Some(status) = child.try_wait()? {
                if exit_code.is_none() {
                    exit_code = Some(child_exit_code(&status));
                    exit_at = Some(Instant::now());
                    core.dirty = true;
                    urgent = true;
                }
            }
            if let Some(code) = exit_code {
                if core.mode == Mode::Follow
                    && ((eof && !core.dirty && display_pos == display.len())
                        || exit_at.unwrap().elapsed() > Duration::from_secs(2))
                {
                    final_screen = Some(core.exit_text());
                    return Ok(code);
                }
            }
            let mut fds = [
                libc::pollfd {
                    fd: 0,
                    events: if child_write.len() - child_pos < 4 * 1024 * 1024
                        && tmux_input
                            .as_ref()
                            .is_none_or(|input| input.pending() < 4 * 1024 * 1024)
                    {
                        libc::POLLIN
                    } else {
                        0
                    },
                    revents: 0,
                },
                libc::pollfd {
                    fd: if eof { -1 } else { fd },
                    events: if eof { 0 } else { libc::POLLIN }
                        | if child_pos < child_write.len() && exit_code.is_none() {
                            libc::POLLOUT
                        } else {
                            0
                        },
                    revents: 0,
                },
                libc::pollfd {
                    fd: 1,
                    events: if display_pos < display.len() {
                        libc::POLLOUT
                    } else {
                        0
                    },
                    revents: 0,
                },
                tmux_input.as_ref().map_or(
                    libc::pollfd {
                        fd: -1,
                        events: 0,
                        revents: 0,
                    },
                    |input| input.pollfd(),
                ),
            ];
            terminal::poll(&mut fds, 10)?;
            if fds[3].revents & (libc::POLLERR | libc::POLLHUP) != 0 && exit_code.is_none() {
                return Err("tmux input connection closed".into());
            }
            if fds[3].revents & libc::POLLOUT != 0 {
                tmux_input.as_mut().unwrap().flush()?;
            }
            if fds[0].revents & libc::POLLHUP != 0
                || fds[2].revents & (libc::POLLERR | libc::POLLHUP) != 0
            {
                return Ok(exit_code.unwrap_or(129));
            }
            if fds[0].revents & libc::POLLIN != 0 {
                match terminal::read(0, &mut buf) {
                    Ok(0) => return Ok(exit_code.unwrap_or(0)),
                    Ok(n) => {
                        last_input = Instant::now();
                        urgent = true;
                        if route(
                            &mut router,
                            &mut core,
                            &mut router_mode,
                            &buf[..n],
                            exit_code,
                            &mut tmux_input,
                        ) {
                            final_screen = Some(core.exit_text());
                            return Ok(exit_code.unwrap_or(0));
                        }
                    }
                    Err(e) if transient(&e) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 && !eof {
                for _ in 0..4 {
                    match terminal::read(fd, &mut buf) {
                        Ok(0) => {
                            eof = true;
                            break;
                        }
                        Ok(n) => core.process(&buf[..n]),
                        Err(e) if e.raw_os_error() == Some(libc::EIO) => {
                            eof = true;
                            break;
                        }
                        Err(e) if transient(&e) => break,
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            if fds[1].revents & libc::POLLOUT != 0 && child_pos < child_write.len() {
                match terminal::write(fd, &child_write[child_pos..]) {
                    Ok(n) => child_pos += n,
                    Err(e) if transient(&e) => {}
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            if fds[2].revents & libc::POLLOUT != 0 && display_pos < display.len() {
                match terminal::write(1, &display[display_pos..]) {
                    Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
                    Ok(n) => display_pos += n,
                    Err(e) if transient(&e) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    })();
    if child.try_wait()?.is_none() {
        if let Some(pid) = child.process_id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGHUP);
            }
        } else {
            let _ = child.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while child.try_wait()?.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if child.try_wait()?.is_none() {
            if let Some(pid) = child.process_id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.kill();
        }
        let _ = child.wait();
    }
    drop(outer);
    if let Some(screen) = final_screen.filter(|s| !s.is_empty()) {
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "{screen}");
    }
    result
}
fn transient(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

fn route(
    router: &mut Router,
    core: &mut Core,
    mode: &mut codex24h::input::InputMode,
    bytes: &[u8],
    exit: Option<i32>,
    tmux_input: &mut Option<codex24h::tmux_input::TmuxInput>,
) -> bool {
    // Apply mode transitions between keys even when an SSH packet includes the
    // prefix, search command and search text in a single read.
    let mut actions = router.feed_step(bytes);
    while !actions.is_empty() {
        for action in actions {
            if exit.is_some()
                && matches!(&action,Action::Forward(v)if v==b"q"||v==b"\r"||v==b"\x03")
            {
                return true;
            }
            apply_input(core, tmux_input, action);
            let next = core.input_mode();
            if next != *mode {
                *mode = next;
                router.set_mode(next);
            }
        }
        actions = router.drain_step();
    }
    router.detach_requested()
}
fn apply_input(
    core: &mut Core,
    tmux_input: &mut Option<codex24h::tmux_input::TmuxInput>,
    action: Action,
) {
    if let (Some(input), Action::Forward(bytes)) = (tmux_input.as_mut(), &action) {
        input.enqueue(bytes);
    } else {
        core.action(action);
    }
}
fn child_exit_code(status: &portable_pty::ExitStatus) -> i32 {
    if let Some(name) = status.signal() {
        for signal in 1..=64 {
            let ptr = unsafe { libc::strsignal(signal) };
            if !ptr.is_null() && unsafe { std::ffi::CStr::from_ptr(ptr) }.to_string_lossy() == name
            {
                return 128 + signal;
            }
        }
    }
    status.exit_code() as i32
}
