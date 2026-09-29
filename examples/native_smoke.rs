//! Real Codex smoke check: `cargo build && cargo run --example native_smoke`.
//! This uses the caller's Codex installation and credentials. It sends only
//! native slash commands and exits the resume picker without opening a session.
//! Set CODEX24H_SMOKE_QUESTION=1 to opt in to one real model question in a
//! new test session; the default run sends no model request.
use std::{
    cell::RefCell,
    env,
    error::Error,
    io::{Read, Write},
    path::PathBuf,
    rc::Rc,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use alacritty_terminal::{
    event::{Event, EventListener},
    grid::Dimensions,
    index::{Column, Line},
    term::{Config, Term, cell::Flags},
    vte::ansi::Processor,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const ROWS: usize = 34;
const COLS: usize = 110;
const STARTUP: Duration = Duration::from_secs(90);
const STEP: Duration = Duration::from_secs(20);

#[derive(Clone, Copy)]
struct Size;
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        ROWS
    }
    fn screen_lines(&self) -> usize {
        ROWS
    }
    fn columns(&self) -> usize {
        COLS
    }
}
#[derive(Clone)]
struct Events(Rc<RefCell<Vec<Event>>>);
impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.borrow_mut().push(event);
    }
}

struct Session {
    master: Box<dyn MasterPty>,
    child: Box<dyn Child>,
    reader: Receiver<Vec<u8>>,
    writer: Box<dyn Write + Send>,
    terminal: Term<Events>,
    parser: Processor,
    events: Events,
    query_tail: Vec<u8>,
    debug: bool,
    kitty: bool,
}

impl Session {
    fn start(args: &[&str], debug: bool) -> Result<Self> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let binary = env::var_os("CODEX24H_TEST_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("target/debug/codex24h"));
        if !binary.is_file() {
            return Err(format!(
                "build the wrapper first: cargo build ({})",
                binary.display()
            )
            .into());
        }
        let pair = native_pty_system().openpty(PtySize {
            rows: ROWS as u16,
            cols: COLS as u16,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut command = CommandBuilder::new(binary);
        command.args(args);
        command.cwd(root);
        command.env("TERM", "xterm-256color");
        command.env("CODEX24H_PIN_ROWS", "0"); // Original full-frame freeze suite.
        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let mut pty_reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (tx, reader) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0u8; 65536];
            loop {
                match pty_reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buffer[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let events = Events(Rc::new(RefCell::new(Vec::new())));
        let terminal = Term::new(Config::default(), &Size, events.clone());
        Ok(Self {
            master: pair.master,
            child,
            reader,
            writer,
            terminal,
            parser: Processor::new(),
            events,
            query_tail: Vec::new(),
            debug,
            kitty: env::var("CODEX24H_SMOKE_KITTY").is_ok_and(|v| v == "1"),
        })
    }

    fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    fn pump(&mut self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self
                .reader
                .recv_timeout(remaining.min(Duration::from_millis(50)))
            {
                Ok(bytes) => {
                    self.parser.advance(&mut self.terminal, &bytes);
                    self.reply_to_queries(&bytes)?;
                    self.events.0.borrow_mut().clear();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    }

    fn reply_to_queries(&mut self, bytes: &[u8]) -> Result<()> {
        for &byte in bytes {
            self.query_tail.push(byte);
            if self.query_tail.len() > 80 {
                self.query_tail.remove(0);
            }
            let t = &self.query_tail;
            let reply: Option<&[u8]> = if t.ends_with(b"\x1b[?u") {
                self.kitty.then_some(b"\x1b[?0u".as_slice())
            } else if t.ends_with(b"\x1b]10;?\x1b\\") || t.ends_with(b"\x1b]10;?\x07") {
                Some(b"\x1b]10;rgb:eeee/eeee/eeee\x1b\\")
            } else if t.ends_with(b"\x1b]11;?\x1b\\") || t.ends_with(b"\x1b]11;?\x07") {
                Some(b"\x1b]11;rgb:1111/1111/1111\x1b\\")
            } else if t.ends_with(b"\x1b[6n") {
                Some(b"\x1b[1;1R")
            } else if t.ends_with(b"\x1b[0c") || t.ends_with(b"\x1b[c") {
                Some(b"\x1b[?62;4c")
            } else if t.ends_with(b"\x1b]52;c;?\x1b\\") || t.ends_with(b"\x1b]52;c;?\x07") {
                Some(b"\x1b]52;c;\x1b\\")
            } else {
                None
            };
            if let Some(reply) = reply {
                self.writer.write_all(reply)?;
                self.writer.flush()?;
                self.query_tail.clear();
            }
        }
        Ok(())
    }

    fn lines(&self) -> Vec<String> {
        (0..ROWS)
            .map(|row| {
                let mut text = String::new();
                for col in 0..COLS {
                    let cell = &self.terminal.grid()[Line(row as i32)][Column(col)];
                    if !cell
                        .flags
                        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                    {
                        text.push(cell.c);
                        if let Some(extra) = cell.zerowidth() {
                            text.extend(extra);
                        }
                    }
                }
                text.trim_end().to_owned()
            })
            .collect()
    }

    fn body(&self) -> Vec<String> {
        self.lines()[..ROWS - 1].to_vec()
    }
    fn screen(&self) -> String {
        self.lines().join("\n")
    }

    fn wait_for(
        &mut self,
        label: &str,
        duration: Duration,
        check: impl Fn(&str) -> bool,
    ) -> Result<String> {
        let deadline = Instant::now() + duration;
        loop {
            self.pump(Duration::from_millis(80))?;
            let screen = self.screen();
            if check(&screen) {
                return Ok(screen);
            }
            if self.child.try_wait()?.is_some() {
                return Err(format!("{label}: wrapper exited before expected screen").into());
            }
            if Instant::now() >= deadline {
                if self.debug {
                    eprintln!("DEBUG {label}:\n{screen}");
                }
                return Err(
                    format!("{label}: timed out after {} seconds", duration.as_secs()).into(),
                );
            }
        }
    }

    fn model_menu(&mut self, label: &str) -> Result<()> {
        let deadline = Instant::now() + STARTUP;
        let mut waiting_reported = false;
        while Instant::now() < deadline {
            self.slash("/model")?;
            self.pump(Duration::from_secs(2))?;
            let screen = self.screen();
            if menu(&screen) {
                println!("ok: {label}");
                return Ok(());
            }
            if screen.contains("Model selection is disabled until startup completes") {
                if !waiting_reported {
                    println!("waiting: native model selection startup");
                    waiting_reported = true;
                }
                self.pump(Duration::from_secs(3))?;
                continue;
            }
            if screen.contains("› /model") {
                self.send(b"\r")?;
                self.pump(Duration::from_secs(2))?;
                if menu(&self.screen()) {
                    println!("ok: {label}");
                    return Ok(());
                }
            }
            if self.debug {
                eprintln!("DEBUG {label}:\n{}", self.screen());
            }
            return Err(format!("{label}: native Codex did not open model menu").into());
        }
        Err(format!("{label}: native model selection remained disabled for 90 seconds").into())
    }

    fn slash(&mut self, command: &str) -> Result<()> {
        self.send(command.as_bytes())?;
        self.pump(Duration::from_millis(250))?;
        self.send(b"\r")
    }

    fn step_slash(
        &mut self,
        label: &str,
        command: &str,
        check: impl Fn(&str) -> bool,
    ) -> Result<String> {
        self.slash(command)?;
        let screen = self.wait_for(label, STEP, check)?;
        println!("ok: {label}");
        Ok(screen)
    }

    fn step(&mut self, label: &str, bytes: &[u8], check: impl Fn(&str) -> bool) -> Result<String> {
        self.send(bytes)?;
        let screen = self.wait_for(label, STEP, check)?;
        println!("ok: {label}");
        Ok(screen)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = &self.master;
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        if let Some(pid) = self.child.process_id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGHUP);
            }
        } else {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if let Some(pid) = self.child.process_id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn ready(screen: &str) -> bool {
    let lower = screen.to_ascii_lowercase();
    (lower.contains("codex")
        && (lower.contains("model") || lower.contains("message") || lower.contains("ask")))
        && !lower.contains("trust this directory")
        && !lower.contains("enter continue · esc skip")
}

fn start_ready(args: &[&str], debug: bool) -> Result<Session> {
    let mut session = Session::start(args, debug)?;
    let deadline = Instant::now() + STARTUP;
    let mut trust_answered = false;
    let mut update_dismissed = false;
    let mut ready_since = None;
    loop {
        session.pump(Duration::from_millis(120))?;
        let screen = session.screen();
        let lower = screen.to_ascii_lowercase();
        if !trust_answered
            && (lower.contains("trust this directory") || lower.contains("trust the contents"))
        {
            session.send(b"\r")?;
            trust_answered = true;
            ready_since = None;
            println!("ok: native project trust prompt accepted");
        } else if !update_dismissed
            && lower.contains("enter continue")
            && lower.contains("esc skip")
        {
            session.pump(Duration::from_secs(3))?;
            session.send(b"\x1b")?;
            update_dismissed = true;
            ready_since = None;
            println!("ok: native update prompt dismissed");
        } else if ready(&screen) {
            let since = ready_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(1500) {
                println!("ok: native Codex ready");
                return Ok(session);
            }
        } else {
            ready_since = None;
        }
        if session.child.try_wait()?.is_some() {
            return Err("wrapper exited during native startup".into());
        }
        if Instant::now() >= deadline {
            if debug {
                eprintln!("DEBUG startup:\n{screen}");
            }
            return Err("native Codex startup timed out".into());
        }
    }
}

fn menu(screen: &str) -> bool {
    let lower = screen.to_ascii_lowercase();
    lower.contains("select model")
        || lower.contains("choose a model")
        || lower.contains("available models")
}

fn run(debug: bool) -> Result<()> {
    if env::var("CODEX24H_SMOKE_PICKER_ONLY").is_ok_and(|v| v == "1") {
        return run_picker(debug);
    }
    if env::var("CODEX24H_SMOKE_QUESTION_ONLY").is_ok_and(|v| v == "1") {
        return run_question(debug);
    }
    println!("native smoke: opening main session");
    let mut session = start_ready(&[], debug)?;
    session.model_menu("/model menu")?;
    session.step("/model Escape", b"\x1b", |screen| {
        !menu(screen) && ready(screen)
    })?;

    let before = session.step("wrapper PageUp browse", b"\x1b[5~", |screen| {
        screen.contains("new updates")
    })?;
    let frozen_body = session.body();
    session.slash("/model")?;
    session.wait_for("frozen menu output", STEP, |screen| {
        screen.contains("new updates") && !menu(screen)
    })?;
    if session.body() != frozen_body {
        return Err("browsing body changed while native /model output arrived".into());
    }
    if !before.contains("new updates") {
        return Err("wrapper did not enter Browse".into());
    }
    println!("ok: frozen body survives native /model output");
    session.step("Ctrl+] b restores native menu", b"\x1db", menu)?;
    session.step("native menu Escape", b"\x1b", |screen| {
        !menu(screen) && ready(screen)
    })?;

    session.step_slash("/mcp view", "/mcp", |screen| {
        screen.to_ascii_lowercase().contains("mcp")
    })?;
    session.send(b"\x1b")?;
    session.pump(Duration::from_millis(400))?;
    session.step_slash("/diff no-repo response", "/diff", |screen| {
        let lower = screen.to_ascii_lowercase();
        lower.contains("diff")
            || lower.contains("not a git repository")
            || lower.contains("no changes")
    })?;
    session.send(b"\x1b")?;
    session.pump(Duration::from_millis(400))?;
    session.step("wrapper Help", b"\x1d?", |screen| {
        screen.contains("codex24h — original Codex")
    })?;
    session.step("wrapper Help Escape", b"\x1b", |screen| {
        !screen.contains("codex24h — original Codex")
    })?;
    drop(session);

    if env::var("CODEX24H_SMOKE_QUESTION").is_ok_and(|v| v == "1") {
        run_question(debug)?;
    }
    run_picker(debug)?;
    Ok(())
}

fn run_question(debug: bool) -> Result<()> {
    println!("native smoke: opening new question test session");
    let mut session = start_ready(&[], debug)?;
    session.model_menu("native model ready for question")?;
    session.step("model menu Escape", b"\x1b", |screen| {
        !menu(screen) && ready(screen)
    })?;
    session.slash("/plan")?;
    session.pump(Duration::from_millis(600))?;
    println!("ok: requested native Plan mode");
    let prompt = "请调用 request_user_input 提出一个单选问题：本次测试选择哪一种水果？选项依次为 苹果、香蕉、梨。收到我的选择后，仅输出“已选择：”加上我实际选择的水果名。不要调用其他工具，不要读写文件，不要运行命令。";
    session.send(prompt.as_bytes())?;
    session.pump(Duration::from_millis(500))?;
    session.send(b"\r")?;
    session.pump(Duration::from_millis(300))?;
    session.step("browse during model response", b"\x1b[5~", |screen| {
        screen.contains("new updates")
    })?;
    let frozen = session.body();
    session.wait_for(
        "model output while frozen",
        Duration::from_secs(60),
        |screen| screen.contains("new updates") && !screen.contains("↓ 0 new updates"),
    )?;
    session.pump(Duration::from_secs(15))?;
    if session.body() != frozen {
        return Err("question output changed frozen browse body".into());
    }
    println!("ok: question response arrived behind frozen Browse view");
    session.send(b"\x1db")?;
    session.wait_for("native fruit question", Duration::from_secs(60), |screen| {
        screen.contains("本次测试选择哪一种水果") && screen.contains("香蕉")
    })?;
    println!("ok: Ctrl+] b revealed native fruit question");
    session.send(b"\x1b[B")?;
    session.pump(Duration::from_millis(250))?;
    session.send(b"\r")?;
    session.wait_for(
        "selected fruit follow-up",
        Duration::from_secs(90),
        |screen| screen.contains("已选择：香蕉"),
    )?;
    println!("ok: selected 香蕉 and received native model follow-up");
    session.slash("/status")?;
    session.pump(Duration::from_millis(600))?;
    let id = session
        .screen()
        .split_whitespace()
        .find(|word| is_uuid(word))
        .map(|word| {
            word.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != '-')
                .to_owned()
        })
        .ok_or("native /status did not reveal a test session ID")?;
    println!("test session ID: {id}");
    drop(session);
    let mut resumed = start_ready(&["resume", &id], debug)?;
    resumed.wait_for("resume own test conversation", STARTUP, |screen| {
        screen.contains("已选择：香蕉")
    })?;
    println!("ok: resumed the test-created conversation by ID");
    Ok(())
}

fn is_uuid(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != '-');
    word.len() == 36
        && word.char_indices().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn run_picker(debug: bool) -> Result<()> {
    println!("native smoke: opening resume picker");
    let mut picker = Session::start(&["resume"], debug)?;
    let deadline = Instant::now() + STARTUP;
    let mut update_dismissed = false;
    loop {
        picker.pump(Duration::from_millis(120))?;
        let screen = picker.screen();
        let lower = screen.to_ascii_lowercase();
        if lower.contains("resume") && (lower.contains("session") || lower.contains("conversation"))
        {
            break;
        }
        if !update_dismissed && lower.contains("enter continue") && lower.contains("esc skip") {
            picker.pump(Duration::from_secs(3))?;
            picker.send(b"\x1b")?;
            update_dismissed = true;
            println!("ok: resume invocation update prompt dismissed");
        }
        if picker.child.try_wait()?.is_some() {
            return Err("resume invocation exited before picker".into());
        }
        if Instant::now() >= deadline {
            if debug {
                eprintln!("DEBUG resume picker:\n{screen}");
            }
            return Err("resume picker: timed out after 90 seconds".into());
        }
    }
    println!("ok: native resume picker shown without selecting a conversation");
    picker.send(b"\x1b")?;
    picker.pump(Duration::from_millis(500))?;
    println!("ok: resume picker dismissed");
    Ok(())
}

fn main() {
    let debug = env::var("CODEX24H_SMOKE_DEBUG").is_ok_and(|v| v == "1");
    if let Err(error) = run(debug) {
        eprintln!("native smoke failed: {error}");
        std::process::exit(1);
    }
}
