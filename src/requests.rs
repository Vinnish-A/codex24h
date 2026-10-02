//! Session records supply labels only. Navigation uses native terminal cells.
use crate::{
    core::Size,
    input::{Action, Key},
    render::Frame,
};
use alacritty_terminal::term::cell::{Cell, Flags};
use serde_json::Value;
use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Receiver},
};
use unicode_width::UnicodeWidthChar;

#[derive(Default)]
pub struct Requests {
    pub pid: Option<u32>,
    pub session: Option<String>,
    pending: Option<Receiver<Result<Vec<String>, String>>>,
    pub texts: Vec<String>,
    pub locations: Vec<Option<usize>>,
    selected: usize,
    filter: String,
    pub message: String,
}
pub enum Choice {
    Stay,
    Close,
    Jump(usize),
}
impl Requests {
    pub fn open(&mut self) {
        self.filter.clear();
        self.selected = 0;
        self.texts.clear();
        self.locations.clear();
        let Some(pid) = self.pid else {
            self.message = "Session process is unavailable".into();
            return;
        };
        self.message = "Loading requests...".into();
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let session = self.session.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let mut helper = std::env::current_exe()
                    .map_err(|e| e.to_string())?
                    .with_file_name("codex24h-requests");
                if !helper.is_file() {
                    helper =
                        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/codex24h-requests");
                }
                let mut command = Command::new(helper);
                command.arg(pid.to_string());
                if let Some(session) = session {
                    command.arg("--session").arg(session);
                }
                let output = command.output().map_err(|e| e.to_string())?;
                if !output.status.success() {
                    return Err("Could not read request list".into());
                }
                let v: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
                if let Some(error) = v["error"].as_str() {
                    return Err(error.into());
                }
                Ok(v["requests"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r["text"].as_str().map(str::to_owned))
                    .collect())
            })();
            let _ = tx.send(result);
        });
    }
    pub fn tick(&mut self) -> bool {
        let Some(result) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return false;
        };
        self.pending = None;
        match result {
            Ok(texts) => {
                self.texts = texts;
                self.selected = if self.filter.is_empty() {
                    self.texts.len().saturating_sub(1)
                } else {
                    0
                };
                self.message = if self.texts.is_empty() {
                    "No saved requests".into()
                } else {
                    String::new()
                };
            }
            Err(error) => self.message = error,
        }
        true
    }
    fn matches(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.texts.len()).collect();
        }
        let query = self.filter.to_lowercase();
        self.texts
            .iter()
            .enumerate()
            .filter(|(_, s)| s.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect()
    }
    pub fn action(&mut self, action: &Action, size: Size) -> Choice {
        let matches = self.matches();
        let page = size.rows.saturating_sub(2).max(1) as i64;
        let delta = match action {
            Action::Scroll(n) => Some(-(*n as i64)),
            Action::PageUp => Some(-page),
            Action::PageDown => Some(page),
            Action::LocalKey(Key::Up) => Some(-1),
            Action::LocalKey(Key::Down) => Some(1),
            _ => None,
        };
        if let Some(delta) = delta {
            self.selected = (self.selected as i64 + delta)
                .clamp(0, matches.len().saturating_sub(1) as i64)
                as usize;
        }
        match action {
            Action::LocalKey(Key::Escape | Key::Ctrl(3)) => return Choice::Close,
            Action::LocalKey(Key::Enter) => {
                if let Some(&i) = matches.get(self.selected) {
                    return Choice::Jump(i);
                }
            }
            Action::LocalKey(Key::Home) => self.selected = 0,
            Action::LocalKey(Key::End) => self.selected = matches.len().saturating_sub(1),
            Action::LocalKey(Key::Char(c)) if !c.is_control() => {
                self.filter.push(*c);
                self.selected = 0;
            }
            Action::LocalKey(Key::Backspace) => {
                self.filter.pop();
                self.selected = 0;
            }
            Action::LocalKey(Key::Ctrl(21)) => {
                self.filter.clear();
                self.selected = 0;
            }
            _ => {}
        }
        Choice::Stay
    }
    pub fn frame(&self, size: Size) -> Frame {
        let mut f = Frame {
            rows: size.rows,
            cols: size.cols,
            cells: vec![Cell::default(); size.rows * size.cols],
            cursor: None,
            cursor_shape: 0,
            status: "REQUESTS · ↑↓ select · type to filter · Enter: jump · Esc: close".into(),
        };
        put(
            &mut f,
            0,
            &format!(
                "Requests ({}){} /{}",
                self.texts.len(),
                self.session
                    .as_ref()
                    .map(|s| format!(" · resumed {}", s))
                    .unwrap_or_default(),
                self.filter
            ),
            false,
        );
        if !self.message.is_empty() {
            f.status = self.message.clone();
        }
        let matches = self.matches();
        let count = size.rows.saturating_sub(1).max(1);
        let top = self
            .selected
            .saturating_sub(count / 2)
            .min(matches.len().saturating_sub(count));
        if matches.is_empty() {
            put(&mut f, 1, "No matching requests", false);
        }
        for (pos, &i) in matches.iter().enumerate().skip(top).take(count) {
            let preview: String = self.texts[i]
                .chars()
                .take(size.cols * 4)
                .map(|c| if c.is_whitespace() { ' ' } else { c })
                .collect();
            let label = format!(
                "{} {}{}  {}",
                if pos == self.selected { ">" } else { " " },
                i + 1,
                if self.locations.get(i).copied().flatten().is_none() {
                    " [full history]"
                } else {
                    ""
                },
                preview
            );
            put(&mut f, pos - top + 1, &label, pos == self.selected);
        }
        f
    }
}
fn put(frame: &mut Frame, row: usize, text: &str, selected: bool) {
    if row >= frame.rows {
        return;
    }
    let preview: String = text.chars().take(frame.cols.saturating_mul(4)).collect();
    if let Some(cells) = wrap(&preview, frame.cols).first() {
        for (col, cell) in cells.iter().enumerate() {
            frame.cells[row * frame.cols + col] = cell.clone();
        }
    }
    if selected {
        for cell in &mut frame.cells[row * frame.cols..(row + 1) * frame.cols] {
            cell.flags.insert(Flags::INVERSE);
        }
    }
}
fn wrap(text: &str, cols: usize) -> Vec<Vec<Cell>> {
    let mut rows = vec![];
    let mut row: Vec<Cell> = vec![];
    for ch in text.chars() {
        if ch == '\n' {
            rows.push(std::mem::take(&mut row));
            continue;
        }
        if ch.is_control() && ch != '\t' {
            continue;
        }
        let ch = if ch == '\t' { ' ' } else { ch };
        let width = ch.width().unwrap_or(0);
        if width == 0 {
            if let Some(cell) = row
                .iter_mut()
                .rev()
                .find(|c| !c.flags.contains(Flags::WIDE_CHAR_SPACER))
            {
                cell.push_zerowidth(ch);
            }
            continue;
        }
        if width > cols {
            continue;
        }
        if row.len() + width > cols {
            rows.push(std::mem::take(&mut row));
        }
        let mut cell = Cell::default();
        cell.c = ch;
        if width == 2 {
            cell.flags.insert(Flags::WIDE_CHAR);
        }
        row.push(cell);
        if width == 2 {
            let mut spacer = Cell::default();
            spacer.flags.insert(Flags::WIDE_CHAR_SPACER);
            row.push(spacer);
        }
    }
    rows.push(row);
    rows
}
