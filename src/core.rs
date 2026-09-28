//! Terminal state and browsing policy. Child output never changes browsing mode.
use crate::{
    input::{Action, InputMode, Key},
    render::Frame,
};
use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    grid::{Dimensions, Scroll},
    index::{Column, Line},
    term::{
        Config, Osc52, TermMode,
        cell::{Cell, Flags},
    },
    vte::ansi::{Color, CursorShape, Processor, Rgb},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub struct Size {
    pub rows: usize,
    pub cols: usize,
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}
#[derive(Clone)]
pub struct Events(Rc<RefCell<Vec<Event>>>);
impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.borrow_mut().push(event);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Follow,
    Browse,
    Copy,
    Search,
    Help,
}

struct Document {
    rows: Vec<Vec<Cell>>,
    wrapped: Vec<bool>,
    cols: usize,
    top: usize,
    cursor: (usize, usize),
    anchor: Option<(usize, usize)>,
}
impl Document {
    fn row_chars(&self, row: usize, lo: usize, hi: usize) -> Vec<(char, usize)> {
        let mut chars = Vec::new();
        for col in lo..=hi {
            if let Some(cell) = self.rows[row].get(col) {
                if !cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    chars.push((cell.c, col));
                    if let Some(extra) = cell.zerowidth() {
                        chars.extend(extra.iter().copied().map(|ch| (ch, col)));
                    }
                }
            } else {
                chars.push((' ', col));
            }
        }
        chars
    }

    fn text(&self, selection: bool) -> String {
        let (start, end) = if selection {
            let a = self.anchor.unwrap_or((self.cursor.0, 0));
            let b = if self.anchor.is_some() {
                self.cursor
            } else {
                (self.cursor.0, self.cols - 1)
            };
            (a.min(b), a.max(b))
        } else {
            ((0, 0), (self.rows.len() - 1, self.cols - 1))
        };
        let mut out = String::new();
        for row in start.0..=end.0 {
            let lo = if row == start.0 { start.1 } else { 0 };
            let hi = if row == end.0 { end.1 } else { self.cols - 1 };
            let mut chars = self.row_chars(row, lo, hi);
            // A soft wrap is part of the same logical line. Its spaces are
            // content; trimming them would join words on copy and export.
            if row == end.0 || !self.wrapped[row] {
                while chars.last().is_some_and(|(ch, _)| *ch == ' ') {
                    chars.pop();
                }
            }
            out.extend(chars.into_iter().map(|(ch, _)| ch));
            if row != end.0 && !self.wrapped[row] {
                out.push('\n');
            }
        }
        out
    }

    fn find(
        &self,
        query: &str,
        after: Option<(usize, usize)>,
        reverse: bool,
    ) -> Option<(usize, usize)> {
        if query.is_empty() {
            return None;
        }
        let mut matches = Vec::new();
        let mut logical = String::new();
        let mut positions = Vec::new();
        for r in 0..self.rows.len() {
            let mut chars = self.row_chars(r, 0, self.cols - 1);
            if !self.wrapped[r] || r + 1 == self.rows.len() {
                while chars.last().is_some_and(|(ch, _)| *ch == ' ') {
                    chars.pop();
                }
            }
            for (ch, col) in chars {
                positions.extend(std::iter::repeat_n((r, col), ch.len_utf8()));
                logical.push(ch);
            }
            if !self.wrapped[r] || r + 1 == self.rows.len() {
                for (offset, _) in logical.match_indices(query) {
                    matches.push(positions[offset]);
                }
                logical.clear();
                positions.clear();
            }
        }
        if reverse {
            matches.reverse();
        }
        matches
            .iter()
            .copied()
            .find(|p| after.is_none_or(|a| if reverse { *p < a } else { *p > a }))
            .or_else(|| matches.first().copied())
    }
}

pub struct Core {
    pub term: Term<Events>,
    parser: Processor,
    events: Events,
    pub size: Size,
    pub mode: Mode,
    frozen: Option<Frame>,
    document: Option<Document>,
    pub pty_out: Vec<u8>,
    control: Vec<u8>,
    title: Option<String>,
    bell: bool,
    last_live: Vec<Cell>,
    pub unread: u64,
    pub dirty: bool,
    pub query: String,
    note: String,
    history_limit: usize,
    foreground: Rgb,
    background: Rgb,
    outer_kitty: bool,
    last_modes: Option<(u32, bool)>,
    clipboard_pending: Option<(
        Instant,
        std::sync::Arc<dyn Fn(&str) -> String + Send + Sync>,
    )>,
    pub export_dir: PathBuf,
}

impl Core {
    pub fn new(
        size: Size,
        history: usize,
        kitty: bool,
        foreground: Rgb,
        background: Rgb,
        export_dir: PathBuf,
    ) -> Self {
        let events = Events(Rc::new(RefCell::new(Vec::new())));
        let term = Term::new(
            Config {
                scrolling_history: history,
                kitty_keyboard: kitty,
                osc52: Osc52::CopyPaste,
                ..Config::default()
            },
            &size,
            events.clone(),
        );
        Self {
            term,
            parser: Processor::new(),
            events,
            size,
            mode: Mode::Follow,
            frozen: None,
            document: None,
            pty_out: vec![],
            control: vec![],
            title: None,
            bell: false,
            last_live: vec![],
            unread: 0,
            dirty: true,
            query: String::new(),
            note: String::new(),
            history_limit: history,
            foreground,
            background,
            outer_kitty: kitty,
            last_modes: None,
            clipboard_pending: None,
            export_dir,
        }
    }
    pub fn input_mode(&self) -> InputMode {
        if matches!(self.mode, Mode::Copy | Mode::Search | Mode::Help) {
            InputMode::Local
        } else {
            InputMode::Normal
        }
    }
    pub fn invalidate_modes(&mut self) {
        self.last_modes = None;
        self.dirty = true;
    }
    pub fn exit_text(&self) -> String {
        let cells = self.cells_at(0);
        let mut lines = Vec::new();
        for row in cells.chunks(self.size.cols) {
            let mut line = String::new();
            for cell in row {
                if !cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    line.push(cell.c);
                    if let Some(z) = cell.zerowidth() {
                        line.extend(z);
                    }
                }
            }
            lines.push(line.trim_end().to_string());
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        self.handle_events();
        self.dirty = true;
    }
    fn handle_events(&mut self) {
        let events = std::mem::take(&mut *self.events.0.borrow_mut());
        for event in events {
            match event {
                Event::PtyWrite(s) => self.pty_out.extend_from_slice(s.as_bytes()),
                Event::ColorRequest(i, reply) => {
                    let color = self.term.colors()[i].unwrap_or_else(|| match i {
                        256 => self.foreground,
                        257 => self.background,
                        258 => self.foreground,
                        _ => palette(i),
                    });
                    self.pty_out.extend_from_slice(reply(color).as_bytes());
                }
                Event::TextAreaSizeRequest(reply) => self.pty_out.extend_from_slice(
                    reply(WindowSize {
                        num_lines: self.size.rows as u16,
                        num_cols: self.size.cols as u16,
                        cell_width: 0,
                        cell_height: 0,
                    })
                    .as_bytes(),
                ),
                Event::Title(title) => {
                    self.title = Some(title.chars().filter(|c| !c.is_control()).collect())
                }
                Event::ResetTitle => self.title = Some("codex24h".into()),
                Event::Bell => self.bell = true,
                Event::ClipboardStore(_, text) => self.clipboard(&text),
                Event::ClipboardLoad(_, reply) => {
                    self.control.extend_from_slice(b"\x1b]52;c;?\x1b\\");
                    self.clipboard_pending = Some((Instant::now() + Duration::from_secs(2), reply));
                }
                _ => {}
            }
        }
    }
    pub fn tick(&mut self) {
        if self
            .parser
            .sync_timeout()
            .sync_timeout()
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.parser.stop_sync(&mut self.term);
            self.handle_events();
            self.dirty = true;
        }
        if self
            .clipboard_pending
            .as_ref()
            .is_some_and(|(deadline, _)| Instant::now() >= *deadline)
        {
            let (_, reply) = self.clipboard_pending.take().unwrap();
            self.pty_out.extend_from_slice(reply("").as_bytes());
        }
        if self.dirty {
            let live = self.cells_at(0);
            if !self.last_live.is_empty() && live != self.last_live && self.mode != Mode::Follow {
                self.unread += 1;
            }
            self.last_live = live;
        }
    }
    pub fn resize(&mut self, size: Size) {
        self.term.resize(size);
        self.size = size;
        self.dirty = true;
        // Keep frozen cell content across a physical resize; do not read live output.
        if let Some(old) = self.frozen.take() {
            let mut cells = vec![Cell::default(); size.rows * size.cols];
            for r in 0..old.rows.min(size.rows) {
                for c in 0..old.cols.min(size.cols) {
                    cells[r * size.cols + c] = old.cells[r * old.cols + c].clone();
                }
            }
            self.frozen = Some(Frame {
                rows: size.rows,
                cols: size.cols,
                cells,
                cursor: None,
                cursor_shape: 0,
                status: String::new(),
                selection: None,
            });
        }
        if let Some(doc) = &mut self.document {
            doc.top = doc.top.min(doc.rows.len().saturating_sub(size.rows));
        }
    }
    fn resolved(&self, mut cell: Cell) -> Cell {
        let resolve = |color| match color {
            Color::Named(n) => self.term.colors()[n].map(Color::Spec).unwrap_or(color),
            Color::Indexed(i) => self.term.colors()[i as usize]
                .map(Color::Spec)
                .unwrap_or(color),
            _ => color,
        };
        cell.fg = resolve(cell.fg);
        cell.bg = resolve(cell.bg);
        if let Some(c) = cell.underline_color() {
            cell.set_underline_color(Some(resolve(c)));
        }
        cell
    }
    fn cells_at(&self, offset: usize) -> Vec<Cell> {
        (0..self.size.rows)
            .flat_map(|r| {
                (0..self.size.cols).map(move |c| {
                    self.resolved(
                        self.term.grid()[Line(r as i32 - offset as i32)][Column(c)].clone(),
                    )
                })
            })
            .collect()
    }
    fn capture(&mut self) {
        self.frozen = Some(Frame {
            rows: self.size.rows,
            cols: self.size.cols,
            cells: self.cells_at(self.term.grid().display_offset()),
            cursor: None,
            cursor_shape: 0,
            status: String::new(),
            selection: None,
        });
        self.dirty = true;
    }
    fn bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
        self.mode = Mode::Follow;
        self.frozen = None;
        self.document = None;
        self.unread = 0;
        self.note.clear();
        self.dirty = true;
    }
    fn scroll(&mut self, delta: i32) {
        if let Some(doc) = &mut self.document {
            let end = doc.rows.len().saturating_sub(self.size.rows);
            doc.top = (doc.top as i64 - delta as i64).clamp(0, end as i64) as usize;
            doc.cursor.0 = doc.cursor.0.clamp(
                doc.top,
                (doc.top + self.size.rows - 1).min(doc.rows.len() - 1),
            );
            if self.mode == Mode::Browse && delta < 0 && doc.top == end {
                self.bottom();
                return;
            }
        } else {
            self.term.scroll_display(Scroll::Delta(delta));
            if delta < 0 && self.term.grid().display_offset() == 0 {
                self.bottom();
                return;
            }
            self.mode = Mode::Browse;
            self.capture();
        }
        self.dirty = true;
    }
    fn document(&mut self) {
        if self.document.is_some() {
            return;
        }
        let history = self.term.history_size();
        let mut rows = Vec::with_capacity(history + self.size.rows);
        let mut wrapped = Vec::with_capacity(rows.capacity());
        for r in -(history as i32)..self.size.rows as i32 {
            let mut cells: Vec<_> = (0..self.size.cols)
                .map(|c| self.resolved(self.term.grid()[Line(r)][Column(c)].clone()))
                .collect();
            wrapped.push(
                cells
                    .last()
                    .is_some_and(|c| c.flags.contains(Flags::WRAPLINE)),
            );
            while cells.last().is_some_and(|c| *c == Cell::default()) {
                cells.pop();
            }
            rows.push(cells);
        }
        let top = history.saturating_sub(self.term.grid().display_offset());
        // Preserve exactly the visible cells already frozen, including overwritten live rows.
        if let Some(frame) = &self.frozen {
            for r in 0..frame.rows {
                if top + r < rows.len() {
                    let frozen_row = frame.cells[r * frame.cols..(r + 1) * frame.cols].to_vec();
                    wrapped[top + r] = frozen_row
                        .last()
                        .is_some_and(|c| c.flags.contains(Flags::WRAPLINE));
                    rows[top + r] = frozen_row;
                }
            }
        }
        self.document = Some(Document {
            rows,
            wrapped,
            cols: self.size.cols,
            top,
            cursor: (top, 0),
            anchor: None,
        });
    }
    pub fn action(&mut self, action: Action) {
        self.dirty = true;
        match action {
            Action::Forward(bytes) => self.pty_out.extend(bytes),
            Action::TerminalReply(bytes) => {
                if bytes.starts_with(b"\x1b]52;") && self.clipboard_pending.is_some() {
                    let text = String::from_utf8_lossy(&bytes);
                    let payload = text
                        .splitn(3, ';')
                        .nth(2)
                        .unwrap_or("")
                        .trim_end_matches(['\x07', '\x1b', '\\']);
                    let decoded = STANDARD.decode(payload).unwrap_or_default();
                    let (_, reply) = self.clipboard_pending.take().unwrap();
                    self.pty_out
                        .extend_from_slice(reply(&String::from_utf8_lossy(&decoded)).as_bytes());
                }
            }
            Action::Scroll(delta) => self.scroll(delta),
            Action::PageUp => self.scroll(self.size.rows.saturating_sub(1).max(1) as i32),
            Action::PageDown => self.scroll(-(self.size.rows.saturating_sub(1).max(1) as i32)),
            Action::Bottom => self.bottom(),
            Action::CopyMode => {
                self.document();
                self.mode = Mode::Copy;
                self.note.clear();
            }
            Action::Search => {
                self.document();
                self.mode = Mode::Search;
                self.query.clear();
                self.note.clear();
            }
            Action::Help => {
                if self.mode == Mode::Follow {
                    self.capture();
                }
                self.mode = Mode::Help;
            }
            Action::QuitBrowse => self.bottom(),
            Action::LocalKey(key) => self.local_key(key),
            Action::Mouse {
                button,
                col,
                row,
                release,
            } => {
                if row as usize > self.size.rows {
                    if !release && button & 3 == 0 {
                        self.bottom();
                    }
                    return;
                }
                if self.mode == Mode::Follow && self.term.mode().intersects(TermMode::MOUSE_MODE) {
                    if button & 32 != 0
                        && !self.term.mode().contains(TermMode::MOUSE_MOTION)
                        && (button & 3 == 3 || !self.term.mode().contains(TermMode::MOUSE_DRAG))
                    {
                        return;
                    }
                    if self.term.mode().contains(TermMode::SGR_MOUSE) {
                        self.pty_out.extend_from_slice(
                            format!(
                                "\x1b[<{button};{col};{row}{}",
                                if release { 'm' } else { 'M' }
                            )
                            .as_bytes(),
                        );
                    } else if button <= 223 && col <= 223 && row <= 223 {
                        self.pty_out.extend_from_slice(&[
                            27,
                            b'[',
                            b'M',
                            if release { 35 } else { button as u8 + 32 },
                            col as u8 + 32,
                            row as u8 + 32,
                        ]);
                    }
                } else if self.mode == Mode::Copy && button & 3 == 0 {
                    self.document();
                    self.mode = Mode::Copy;
                    let doc = self.document.as_mut().unwrap();
                    let point = (
                        (doc.top + row.saturating_sub(1) as usize).min(doc.rows.len() - 1),
                        (col.saturating_sub(1) as usize).min(doc.cols - 1),
                    );
                    if button & 32 == 0 && !release {
                        doc.anchor = Some(point);
                    }
                    doc.cursor = point;
                }
            }
        }
    }
    fn leave_local(&mut self) {
        // Keep the full document while browsing. A visible-only frame cannot
        // preserve older rows if child output rewrites or evicts live history.
        self.mode = Mode::Browse;
        self.note.clear();
    }
    fn local_key(&mut self, key: Key) {
        if self.mode == Mode::Help {
            if matches!(
                key,
                Key::Escape | Key::Enter | Key::Char('q') | Key::Ctrl(3)
            ) {
                self.leave_local();
            }
            return;
        }
        if self.mode == Mode::Search {
            match key {
                Key::Escape | Key::Ctrl(3) => self.leave_local(),
                Key::Enter => {
                    self.mode = Mode::Copy;
                    self.find(false, false);
                }
                Key::Backspace => {
                    self.query.pop();
                    self.find(false, false);
                }
                Key::Ctrl(21) => {
                    self.query.clear();
                    self.note.clear();
                }
                Key::Char(c) => {
                    if !c.is_control() {
                        self.query.push(c);
                        self.find(false, false);
                    }
                }
                _ => {}
            }
            return;
        }
        match key {
            Key::Escape | Key::Char('q') | Key::Ctrl(3) => {
                self.leave_local();
                return;
            }
            Key::Char('/') => {
                self.mode = Mode::Search;
                self.query.clear();
                return;
            }
            Key::Char('n') => {
                self.find(false, true);
                return;
            }
            Key::Char('N') => {
                self.find(true, true);
                return;
            }
            Key::Char('y') | Key::Enter => {
                let text = self.document.as_ref().unwrap().text(true);
                self.clipboard(&text);
                self.note = format!("Copied {} bytes via OSC52 · e export", text.len());
                return;
            }
            Key::Char('e') => {
                self.export();
                return;
            }
            _ => {}
        }
        let Some(doc) = &mut self.document else {
            return;
        };
        match key {
            Key::Char('v') => {
                doc.anchor = if doc.anchor.is_some() {
                    None
                } else {
                    Some(doc.cursor)
                };
            }
            Key::Up | Key::Char('k') => doc.cursor.0 = doc.cursor.0.saturating_sub(1),
            Key::Down | Key::Char('j') => doc.cursor.0 = (doc.cursor.0 + 1).min(doc.rows.len() - 1),
            Key::Left | Key::Char('h') => doc.cursor.1 = doc.cursor.1.saturating_sub(1),
            Key::Right | Key::Char('l') => doc.cursor.1 = (doc.cursor.1 + 1).min(doc.cols - 1),
            Key::Home | Key::Char('0') => doc.cursor.1 = 0,
            Key::End | Key::Char('$') => {
                doc.cursor.1 = doc.rows[doc.cursor.0].len().saturating_sub(1)
            }
            Key::Char('g') => doc.cursor.0 = 0,
            Key::Char('G') => doc.cursor.0 = doc.rows.len() - 1,
            _ => {}
        }
        if doc.cursor.0 < doc.top {
            doc.top = doc.cursor.0;
        }
        if doc.cursor.0 >= doc.top + self.size.rows {
            doc.top = doc.cursor.0 + 1 - self.size.rows;
        }
    }
    fn find(&mut self, reverse: bool, next: bool) {
        let Some(doc) = &mut self.document else {
            return;
        };
        if self.query.is_empty() {
            self.note.clear();
            return;
        }
        if let Some(pos) = doc.find(
            &self.query,
            if next { Some(doc.cursor) } else { None },
            reverse,
        ) {
            doc.cursor = pos;
            doc.top = pos
                .0
                .saturating_sub(self.size.rows / 2)
                .min(doc.rows.len().saturating_sub(self.size.rows));
            self.note.clear();
        } else if !self.query.is_empty() {
            self.note = "No match".into();
        }
    }
    fn clipboard(&mut self, text: &str) {
        self.control
            .extend_from_slice(format!("\x1b]52;c;{}\x1b\\", STANDARD.encode(text)).as_bytes());
    }
    fn export(&mut self) {
        use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, time::SystemTime};
        let text = self
            .document
            .as_ref()
            .unwrap()
            .text(self.document.as_ref().unwrap().anchor.is_some());
        let path = self.export_dir.join(format!(
            "copy-{}-{}.txt",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let result = std::fs::create_dir_all(&self.export_dir)
            .and_then(|_| {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
            })
            .and_then(|mut f| f.write_all(text.as_bytes()));
        self.note = match result {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Export failed: {e}"),
        };
    }
    pub fn controls(&mut self) -> Vec<u8> {
        let mut out = std::mem::take(&mut self.control);
        if let Some(title) = self.title.take() {
            out.extend_from_slice(format!("\x1b]0;{title}\x1b\\").as_bytes());
        }
        if std::mem::take(&mut self.bell) {
            out.push(7);
        }
        let modes = (*self.term.mode()).bits();
        let local = self.input_mode() == InputMode::Local;
        if self.last_modes != Some((modes, local)) {
            for (flag, n) in [
                (TermMode::APP_CURSOR, 1),
                (TermMode::FOCUS_IN_OUT, 1004),
                (TermMode::BRACKETED_PASTE, 2004),
            ] {
                let on =
                    self.term.mode().contains(flag) || (local && flag == TermMode::BRACKETED_PASTE);
                out.extend_from_slice(
                    format!("\x1b[?{n}{}", if on { 'h' } else { 'l' }).as_bytes(),
                );
            }
            out.extend_from_slice(if self.term.mode().contains(TermMode::APP_KEYPAD) {
                b"\x1b="
            } else {
                b"\x1b>"
            });
            out.extend_from_slice(if self.term.mode().contains(TermMode::MOUSE_MOTION) {
                b"\x1b[?1003h"
            } else {
                b"\x1b[?1003l"
            });
            out.extend_from_slice(b"\x1b[?1002h\x1b[?1006h");
            if self.outer_kitty {
                let flags = (modes >> 18) & 31;
                out.extend_from_slice(format!("\x1b[={flags}u").as_bytes());
            }
            self.last_modes = Some((modes, local));
        }
        out
    }
    pub fn frame(&self) -> Frame {
        let mut frame = if let Some(doc) = &self.document {
            let mut cells = vec![Cell::default(); self.size.rows * self.size.cols];
            let selection = doc.anchor.map(|a| (a.min(doc.cursor), a.max(doc.cursor)));
            for r in 0..self.size.rows {
                if let Some(row) = doc.rows.get(doc.top + r) {
                    for (c, cell) in row.iter().take(self.size.cols).enumerate() {
                        let mut cell = cell.clone();
                        if selection
                            .is_some_and(|(a, b)| (doc.top + r, c) >= a && (doc.top + r, c) <= b)
                        {
                            cell.flags.toggle(Flags::INVERSE);
                        }
                        cells[r * self.size.cols + c] = cell;
                    }
                }
            }
            Frame {
                rows: self.size.rows,
                cols: self.size.cols,
                cells,
                cursor: Some((
                    doc.cursor.0.saturating_sub(doc.top).min(self.size.rows - 1),
                    doc.cursor.1.min(self.size.cols - 1),
                )),
                cursor_shape: 2,
                status: String::new(),
                selection: None,
            }
        } else if self.mode != Mode::Follow {
            self.frozen.clone().unwrap_or_else(|| self.live_frame())
        } else {
            self.live_frame()
        };
        if matches!(self.mode, Mode::Browse | Mode::Help) {
            frame.cursor = None;
        }
        frame.status = match self.mode {
            Mode::Follow => "codex24h · PgUp / wheel: history · Ctrl+] ?: help".into(),
            Mode::Browse => format!(
                "↓ {} new updates · PgDn: latest · Ctrl+] b: bottom · /: search via Ctrl+]{}",
                self.unread,
                if self.term.history_size() >= self.history_limit {
                    " · history limit"
                } else {
                    ""
                }
            ),
            Mode::Search => format!(
                "/{} · Enter: accept · Esc: browse · {}",
                self.query, self.note
            ),
            Mode::Copy => {
                if self.note.is_empty() {
                    format!(
                        "COPY · arrows/hjkl · v select · y copy · / search · n/N next · e export · ↓ {}",
                        self.unread
                    )
                } else {
                    self.note.clone()
                }
            }
            Mode::Help => "HELP · Esc: browse · Ctrl+] b: live".into(),
        };
        if self.mode == Mode::Help {
            frame.cells.fill(Cell::default());
            frame.cursor = None;
            for (r, line) in [
                "codex24h — original Codex, stable history",
                "",
                "Wheel / PageUp / PageDown   browse terminal history",
                "Ctrl+] then b              return to latest",
                "Ctrl+] then /              search history (literal, case sensitive)",
                "Ctrl+] then [              copy mode",
                "Ctrl+] then p              pass next key unchanged",
                "Ctrl+] then Ctrl+]         send literal Ctrl+]",
                "",
                "Copy: arrows or hjkl; v select; y copy via OSC52",
                "Copy: / search; n / N next / previous; e export text",
                "Copy: g / G first / last; Esc returns to frozen browsing",
                "Click and drag to select; click footer to return to latest",
                "",
                "Ordinary keys still go to Codex while browsing.",
                "Touch requires SSH client mouse reporting or PgUp/PgDown.",
                "History covers terminal output received during this run.",
            ]
            .iter()
            .enumerate()
            .take(self.size.rows)
            {
                for (c, ch) in line.chars().take(self.size.cols).enumerate() {
                    frame.cells[r * self.size.cols + c].c = ch;
                }
            }
        }
        frame
    }
    fn live_frame(&self) -> Frame {
        let cursor = self.term.grid().cursor.point;
        let style = self.term.cursor_style();
        let shape = match style.shape {
            CursorShape::Block => 2,
            CursorShape::Underline => 4,
            CursorShape::Beam => 6,
            _ => 0,
        };
        Frame {
            rows: self.size.rows,
            cols: self.size.cols,
            cells: self.cells_at(0),
            cursor: if self.term.mode().contains(TermMode::SHOW_CURSOR)
                && style.shape != CursorShape::Hidden
            {
                Some((cursor.line.0.max(0) as usize, cursor.column.0))
            } else {
                None
            },
            cursor_shape: if style.blinking && shape > 0 {
                shape - 1
            } else {
                shape
            },
            status: String::new(),
            selection: None,
        }
    }
}

fn palette(i: usize) -> Rgb {
    const BASIC: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let (r, g, b) = if i < 16 {
        BASIC[i]
    } else if i < 232 {
        let i = i - 16;
        let channel = |n: usize| if n == 0 { 0 } else { 55 + 40 * n as u8 };
        (channel(i / 36), channel(i / 6 % 6), channel(i % 6))
    } else if i < 256 {
        let n = 8 + (i - 232) as u8 * 10;
        (n, n, n)
    } else {
        (229, 229, 229)
    };
    Rgb { r, g, b }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn core(history: usize) -> Core {
        Core::new(
            Size { rows: 6, cols: 30 },
            history,
            false,
            Rgb {
                r: 220,
                g: 220,
                b: 220,
            },
            Rgb { r: 0, g: 0, b: 0 },
            PathBuf::from("/tmp"),
        )
    }
    fn text(c: &Core) -> String {
        c.frame().cells.iter().map(|c| c.c).collect()
    }
    #[test]
    fn freeze_survives_repaint_clear_eviction_and_resize() {
        let mut c = core(10);
        for i in 0..30 {
            c.process(format!("line-{i}\r\n").as_bytes());
        }
        c.tick();
        c.action(Action::Scroll(2));
        let before = text(&c);
        c.process(b"\x1b[1;1HOVERWRITE\x1b[2J\x1b[3J");
        for _ in 0..50 {
            c.process(b"new\r\n");
        }
        c.tick();
        assert_eq!(c.mode, Mode::Browse);
        assert_eq!(text(&c), before);
        c.resize(Size { rows: 7, cols: 30 });
        assert_eq!(&text(&c)[..before.len()], before);
        c.action(Action::Bottom);
        assert_eq!(c.mode, Mode::Follow);
        assert!(text(&c).contains("new"));
    }
    #[test]
    fn mouse_forwarding_respects_child_tracking_mode_and_legacy_range() {
        let mut c = core(10);
        c.process(b"\x1b[?1000h\x1b[?1006h");
        let movement = Action::Mouse {
            button: 32,
            col: 2,
            row: 2,
            release: false,
        };
        c.action(movement.clone());
        assert!(c.pty_out.is_empty());
        c.process(b"\x1b[?1002h");
        c.action(movement);
        assert_eq!(c.pty_out, b"\x1b[<32;2;2M");
        c.pty_out.clear();
        c.action(Action::Mouse {
            button: 35,
            col: 2,
            row: 2,
            release: false,
        });
        assert!(c.pty_out.is_empty());
        c.process(b"\x1b[?1006l");
        c.action(Action::Mouse {
            button: 256,
            col: 2,
            row: 2,
            release: false,
        });
        assert!(c.pty_out.is_empty());
    }
    #[test]
    fn terminal_queries_work_while_frozen() {
        let mut c = core(10);
        c.action(Action::PageUp);
        c.process(b"\x1b[2;3H\x1b[6n\x1b]11;?\x07");
        assert!(c.pty_out.windows(6).any(|b| b == b"\x1b[2;3R"));
        assert!(String::from_utf8_lossy(&c.pty_out).contains("rgb:"));
        assert_eq!(c.mode, Mode::Browse);
    }
    #[test]
    fn search_and_copy_unicode_snapshot() {
        let mut c = core(100);
        c.process("zero\r\n中文 target\r\nlast".as_bytes());
        c.action(Action::Search);
        for ch in "中文".chars() {
            c.action(Action::LocalKey(Key::Char(ch)));
        }
        c.action(Action::LocalKey(Key::Enter));
        let doc = c.document.as_ref().unwrap();
        assert_eq!(doc.cursor, (1, 0));
        c.process(b"\x1b[2Jchanged");
        c.action(Action::LocalKey(Key::Char('y')));
        let out = String::from_utf8(c.controls()).unwrap();
        assert!(out.contains(&STANDARD.encode("中文 target")));
    }
    #[test]
    fn ordinary_keys_do_not_unfreeze() {
        let mut c = core(10);
        c.action(Action::PageUp);
        c.action(Action::Forward(b"hello\r".to_vec()));
        assert_eq!(c.pty_out, b"hello\r");
        assert_eq!(c.mode, Mode::Browse);
        c.action(Action::PageDown);
        assert_eq!(c.mode, Mode::Follow);
    }
    #[test]
    fn selection_and_search_keep_spaces_at_soft_wraps() {
        let mut first = vec![Cell::default(); 4];
        first[0].c = '你';
        first[1].flags.insert(Flags::WIDE_CHAR_SPACER);
        first[2].c = ' ';
        first[3].c = ' ';
        first[3].flags.insert(Flags::WRAPLINE);
        let mut second = vec![Cell::default(); 4];
        second[0].c = '好';
        second[1].c = '!';
        let mut doc = Document {
            rows: vec![first, second],
            wrapped: vec![true, false],
            cols: 4,
            top: 0,
            cursor: (1, 1),
            anchor: Some((0, 0)),
        };
        assert_eq!(doc.text(true), "你  好!");
        assert_eq!(doc.text(false), "你  好!");
        assert_eq!(doc.find("你  好", None, false), Some((0, 0)));
        assert_eq!(doc.find("  好!", None, false), Some((0, 2)));
        doc.anchor = None;
        assert_eq!(doc.text(true), "好!");
    }

    #[test]
    fn frozen_rows_keep_their_wrap_flags_in_document() {
        let mut c = core(10);
        let mut cells = vec![Cell::default(); c.size.rows * c.size.cols];
        cells[0].c = 'A';
        cells[c.size.cols - 1].flags.insert(Flags::WRAPLINE);
        cells[c.size.cols].c = 'B';
        c.frozen = Some(Frame {
            rows: c.size.rows,
            cols: c.size.cols,
            cells,
            cursor: None,
            cursor_shape: 0,
            status: String::new(),
            selection: None,
        });
        c.document();
        let doc = c.document.as_ref().unwrap();
        assert!(doc.wrapped[0]);
        assert_eq!(doc.find("A", None, false), Some((0, 0)));
    }

    #[test]
    fn help_exit_restores_underlying_snapshot_and_normal_input() {
        let mut c = core(20);
        c.process(b"original");
        c.action(Action::Help);
        assert_eq!(c.mode, Mode::Help);
        c.action(Action::LocalKey(Key::Escape));
        assert_eq!(c.mode, Mode::Browse);
        assert_eq!(c.input_mode(), InputMode::Normal);
        assert!(text(&c).contains("original"));
        assert!(!text(&c).contains("codex24h "));
        c.action(Action::CopyMode);
        let before = text(&c);
        c.action(Action::Help);
        c.action(Action::LocalKey(Key::Escape));
        assert_eq!(c.mode, Mode::Browse);
        assert!(c.document.is_some());
        assert_eq!(text(&c), before);
        c.process(b"\x1b[2Jchanged");
        c.action(Action::CopyMode);
        assert_eq!(text(&c), before);
    }

    #[test]
    fn ordinary_click_does_not_capture_composer_keys() {
        let mut c = core(20);
        let click = Action::Mouse {
            button: 0,
            col: 2,
            row: 2,
            release: false,
        };
        c.action(click.clone());
        assert_eq!(c.mode, Mode::Follow);
        assert_eq!(c.input_mode(), InputMode::Normal);
        c.action(Action::Forward(b"\x1b[A\x1b[B\t".to_vec()));
        assert_eq!(c.pty_out, b"\x1b[A\x1b[B\t");
        c.action(Action::PageUp);
        c.action(click);
        assert_eq!(c.mode, Mode::Browse);
        assert_eq!(c.input_mode(), InputMode::Normal);
    }

    #[test]
    fn mouse_drag_copies_selected_text_and_export_uses_selection() {
        let export_dir = std::env::temp_dir().join(format!(
            "codex24h-core-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut c = core(20);
        c.export_dir = export_dir.clone();
        c.process(b"abcdef");
        c.action(Action::CopyMode);
        c.action(Action::Mouse {
            button: 0,
            col: 2,
            row: 1,
            release: false,
        });
        c.action(Action::Mouse {
            button: 32,
            col: 5,
            row: 1,
            release: false,
        });
        c.action(Action::Mouse {
            button: 0,
            col: 5,
            row: 1,
            release: true,
        });
        assert_eq!(c.mode, Mode::Copy);
        assert_eq!(c.document.as_ref().unwrap().text(true), "bcde");
        c.action(Action::LocalKey(Key::Char('y')));
        assert!(
            String::from_utf8(c.controls())
                .unwrap()
                .contains(&STANDARD.encode("bcde"))
        );
        c.action(Action::LocalKey(Key::Char('e')));
        let path = std::path::PathBuf::from(c.note.strip_prefix("Saved ").unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "bcde");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(export_dir).unwrap();
    }

    #[test]
    fn browse_keeps_full_document_until_scrolling_to_bottom() {
        let mut c = core(40);
        for i in 0..20 {
            c.process(format!("old-{i}\r\n").as_bytes());
        }
        c.action(Action::Search);
        c.action(Action::LocalKey(Key::Escape));
        assert_eq!(c.mode, Mode::Browse);
        let original = c.document.as_ref().unwrap().text(false);
        c.process(b"\x1b[2Jreplacement\r\n");
        c.action(Action::CopyMode);
        assert_eq!(c.document.as_ref().unwrap().text(false), original);
        c.action(Action::LocalKey(Key::Escape));
        assert_eq!(c.mode, Mode::Browse);
        c.action(Action::PageDown);
        assert_eq!(c.mode, Mode::Follow);
        assert!(c.document.is_none());
        assert!(text(&c).contains("replacement"));
    }
}
