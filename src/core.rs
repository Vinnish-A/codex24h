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
    SearchResults,
    Search,
    Help,
}

struct Document {
    rows: Vec<Vec<Cell>>,
    wrapped: Vec<bool>,
    cols: usize,
    top: usize,
    cursor: (usize, usize),
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
    pub pin_rows: usize,
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
}

impl Core {
    pub fn new(size: Size, history: usize, kitty: bool, foreground: Rgb, background: Rgb) -> Self {
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
            pin_rows: 0,
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
        }
    }
    pub fn input_mode(&self) -> InputMode {
        if matches!(self.mode, Mode::SearchResults | Mode::Search | Mode::Help) {
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
        // Keep the original frozen frame intact. Cropping belongs to display,
        // otherwise a phone rotation permanently erases its off-screen cells.
        if let Some(doc) = &mut self.document {
            doc.top = doc.top.min(doc.rows.len().saturating_sub(size.rows));
        }
    }
    // Infer terminal focus from the visible caret or a reverse-video selection.
    // Blank rows bound the active block; no Codex labels or business text are read.
    fn pin_band(&self) -> Option<(usize, usize, usize)> {
        if self.mode != Mode::Browse || self.pin_rows == 0 || self.size.rows < 5 {
            return None;
        }
        let mut blank = vec![true; self.size.rows];
        let mut inverse = vec![false; self.size.rows];
        for row in 0..self.size.rows {
            for col in 0..self.size.cols {
                let cell = &self.term.grid()[Line(row as i32)][Column(col)];
                if cell.c != ' ' || cell.zerowidth().is_some() {
                    blank[row] = false;
                    inverse[row] |= cell.flags.contains(Flags::INVERSE);
                }
            }
        }
        let caret = (self.term.grid().cursor.point.line.0.max(0) as usize).min(self.size.rows - 1);
        let (focus_start, focus_end, gap) = if self.term.mode().contains(TermMode::SHOW_CURSOR) {
            (caret, caret, 1)
        } else if let Some(end) = inverse.iter().rposition(|selected| *selected) {
            let mut start = end;
            while start > 0 && inverse[start - 1] {
                start -= 1;
            }
            // A menu can contain single blank rows between its heading, options
            // and help. Two blank rows separate it from the transcript.
            (start, end, 2)
        } else {
            let last = blank.iter().rposition(|empty| !empty).unwrap_or(caret);
            (last, last, 1)
        };
        let mut start = 0;
        let mut end = self.size.rows;
        let mut empty = 0;
        for row in (0..focus_start).rev() {
            empty = if blank[row] { empty + 1 } else { 0 };
            if empty == gap {
                start = row + gap;
                break;
            }
        }
        empty = 0;
        for (row, is_blank) in blank.iter().enumerate().skip(focus_end + 1) {
            empty = if *is_blank { empty + 1 } else { 0 };
            if empty == gap {
                end = row + 1 - gap;
                break;
            }
        }
        let height = self.pin_rows.min(self.size.rows - 3).min(end - start);
        let focus_height = (focus_end - focus_start + 1).min(height);
        let source = focus_start
            .saturating_sub((height - focus_height) / 2)
            .clamp(start, end - height);
        Some((source, self.size.rows - height, height))
    }
    fn browse_rows(&self) -> usize {
        if let Some((_, dest, _)) = self.pin_band() {
            dest - 1
        } else if self.pin_rows > 0 && self.size.rows >= 5 {
            self.size.rows - self.pin_rows.min(self.size.rows - 3) - 1
        } else {
            self.size.rows
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
            Action::PageUp => self.scroll(self.browse_rows().saturating_sub(1).max(1) as i32),
            Action::PageDown => self.scroll(-(self.browse_rows().saturating_sub(1).max(1) as i32)),
            Action::Bottom => self.bottom(),
            Action::PinToggle | Action::PinResize(_) => {
                self.pin_rows = match action {
                    Action::PinToggle => {
                        if self.pin_rows == 0 {
                            6
                        } else {
                            0
                        }
                    }
                    Action::PinResize(delta) => {
                        (self.pin_rows as i32 + delta).clamp(0, 100) as usize
                    }
                    _ => unreachable!(),
                };
                if matches!(self.mode, Mode::SearchResults | Mode::Search | Mode::Help) {
                    self.leave_local();
                }
                self.note = format!(
                    "Live input max: {} rows{}",
                    self.pin_rows,
                    if self.mode == Mode::Follow {
                        " (scroll up to view)"
                    } else {
                        ""
                    }
                );
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
                mut row,
                release,
            } => {
                // Selection and copying belong to the client. Do not capture
                // drags, freeze the display, or write to its clipboard.
                if button & 0x43 == 2 || row as usize > self.size.rows {
                    return;
                }
                let band = self.pin_band();
                let live_band = band.is_some_and(|(_, dest, _)| row as usize > dest);
                let application_mouse = (self.mode == Mode::Follow || live_band)
                    && self.term.mode().intersects(TermMode::MOUSE_MODE);
                if application_mouse {
                    if let Some((source, dest, _)) = band {
                        row = (source + row as usize - dest) as u16;
                    }
                }
                if application_mouse {
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
                    self.mode = Mode::SearchResults;
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
            _ => {}
        }
        let Some(doc) = &mut self.document else {
            return;
        };
        match key {
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
            for r in 0..self.size.rows {
                if let Some(row) = doc.rows.get(doc.top + r) {
                    for (c, cell) in row.iter().take(self.size.cols).enumerate() {
                        cells[r * self.size.cols + c] = cell.clone();
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
            }
        } else if self.mode != Mode::Follow {
            let mut frame = self.frozen.clone().unwrap_or_else(|| self.live_frame());
            if frame.rows != self.size.rows || frame.cols != self.size.cols {
                let mut cells = vec![Cell::default(); self.size.rows * self.size.cols];
                for r in 0..frame.rows.min(self.size.rows) {
                    for c in 0..frame.cols.min(self.size.cols) {
                        cells[r * self.size.cols + c] = frame.cells[r * frame.cols + c].clone();
                    }
                }
                frame.rows = self.size.rows;
                frame.cols = self.size.cols;
                frame.cells = cells;
            }
            frame
        } else {
            self.live_frame()
        };
        if matches!(self.mode, Mode::Browse | Mode::Help) {
            frame.cursor = None;
        }
        if let Some((source, dest, height)) = self.pin_band() {
            let live = self.live_frame();
            let cols = self.size.cols;
            frame.cells[dest * cols..(dest + height) * cols]
                .clone_from_slice(&live.cells[source * cols..(source + height) * cols]);
            frame.cells[(dest - 1) * cols..dest * cols].fill(Cell::default());
            for (col, ch) in "-- live input | Ctrl+] i toggle; +/- height --"
                .chars()
                .take(cols)
                .enumerate()
            {
                frame.cells[(dest - 1) * cols + col].c = ch;
            }
            frame.cursor = live.cursor.and_then(|(row, col)| {
                (row >= source && row < source + height)
                    .then_some((dest + row.saturating_sub(source), col))
            });
            frame.cursor_shape = live.cursor_shape;
        }
        frame.status = match self.mode {
            Mode::Follow => "codex24h · PgUp / wheel: history · Ctrl+] ?: help".into(),
            Mode::Browse => format!(
                "HISTORY (frozen) · ↓ {} new updates · PgDn: latest · Ctrl+] b: bottom · /: search via Ctrl+]{}",
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
            Mode::SearchResults => {
                if self.note.is_empty() {
                    format!(
                        "SEARCH RESULTS · Esc: browse · n/N: next/previous · ↓ {}",
                        self.unread
                    )
                } else {
                    self.note.clone()
                }
            }
            Mode::Help => "HELP · Esc: browse · Ctrl+] b: live".into(),
        };
        if matches!(self.mode, Mode::Follow | Mode::Browse) && !self.note.is_empty() {
            frame.status = format!("{} · {}", self.note, frame.status);
        }
        if self.mode == Mode::Help {
            frame.cells.fill(Cell::default());
            frame.cursor = None;
            for (r, line) in [
                "codex24h — original Codex, stable history",
                "",
                "Wheel / PageUp / PageDown   browse terminal history",
                "Ctrl+] then b              return to latest",
                "Ctrl+] then i / + / -      toggle / grow / shrink live input",
                "Ctrl+] then /              search history (literal, case sensitive)",
                "Ctrl+] then p              pass next key unchanged",
                "Ctrl+] then Ctrl+]         send literal Ctrl+]",
                "",
                "Search results: / search; n / N next / previous",
                "Search results: arrows/hjkl; Esc returns to browsing",
                "Select in your terminal (Shift+drag); Ctrl+Shift+C to copy",
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
        )
    }
    fn text(c: &Core) -> String {
        c.frame().cells.iter().map(|c| c.c).collect()
    }
    #[test]
    fn mouse_drag_never_copies_or_freezes_output() {
        let mut c = core(20);
        c.process(b"working");
        for (button, col, release) in [(0, 1, false), (32, 4, false), (3, 4, true)] {
            c.action(Action::Mouse {
                button,
                col,
                row: 1,
                release,
            });
            assert_eq!(c.mode, Mode::Follow);
            assert_eq!(c.input_mode(), InputMode::Normal);
        }
        c.process(b"\rupdated");
        assert!(text(&c).starts_with("updated"));
        assert!(!c.controls().windows(5).any(|b| b == b"\x1b]52;"));
        assert!(!c.frame().status.contains("Sent"));
    }

    #[test]
    fn empty_mouse_selection_preserves_clipboard_and_lost_release_keeps_native_keys() {
        let mut c = core(20);
        for (button, col, release) in [(0, 1, false), (32, 4, false), (0, 4, true)] {
            c.action(Action::Mouse {
                button,
                col,
                row: 1,
                release,
            });
        }
        assert_eq!(c.mode, Mode::Follow);
        assert!(
            !String::from_utf8(c.controls())
                .unwrap()
                .contains("\x1b]52;c;")
        );
        c.process(b"working");
        for (button, col) in [(0, 1), (32, 4)] {
            c.action(Action::Mouse {
                button,
                col,
                row: 1,
                release: false,
            });
        }
        assert_eq!(c.input_mode(), InputMode::Normal);
        c.process(b"\rupdated");
        c.action(Action::Forward(b"\x03".to_vec()));
        assert_eq!(c.mode, Mode::Follow);
        assert_eq!(c.pty_out, b"\x03");
        assert!(text(&c).starts_with("updated"));
        c.process(b"\x1b[?1000h\x1b[?1006h");
        c.pty_out.clear();
        c.action(Action::Mouse {
            button: 2,
            col: 2,
            row: 1,
            release: false,
        });
        assert!(c.pty_out.is_empty());
        assert_eq!(c.mode, Mode::Follow);
    }

    #[test]
    fn resizing_live_input_exits_search_without_losing_history() {
        let mut c = core(100);
        c.process(b"old history\r\n\r\ncomposer");
        c.pin_rows = 6;
        c.action(Action::Search);
        let rows = c.document.as_ref().unwrap().rows.clone();
        assert!(c.pin_band().is_none());
        c.action(Action::PinResize(2));
        assert_eq!(c.mode, Mode::Browse);
        assert_eq!(c.pin_rows, 8);
        assert!(c.pin_band().is_some());
        assert_eq!(c.document.as_ref().unwrap().rows, rows);
        assert!(c.frame().status.starts_with("Live input max: 8 rows"));
        c.action(Action::Forward(b"x".to_vec()));
        assert_eq!(c.pty_out, b"x");
    }

    #[test]
    fn pinned_input_updates_without_moving_history_or_stealing_keys() {
        let mut c = core(100);
        c.pin_rows = 3;
        for i in 0..20 {
            c.process(format!("history-{i}\r\n").as_bytes());
        }
        c.process(b"\x1b[5;1Hdraft\x1b[?25h");
        c.action(Action::Scroll(4));
        let frozen = c.frame().cells[..2 * c.size.cols].to_vec();
        c.process(b"\x1b[1;1Hnew output\x1b[5;1Hupdated");
        let frame = c.frame();
        assert_eq!(frame.cells[..2 * c.size.cols], frozen);
        assert!(
            frame.cells[3 * c.size.cols..]
                .iter()
                .map(|v| v.c)
                .collect::<String>()
                .contains("updated")
        );
        assert!(frame.cursor.is_some());
        c.action(Action::Forward(b"\t\x1b[A".to_vec()));
        assert!(c.pty_out.ends_with(b"\t\x1b[A"));
        assert_eq!(c.mode, Mode::Browse);
        c.action(Action::PinToggle);
        assert!(c.frame().cursor.is_none());
        c.action(Action::Search);
        assert!(c.pin_band().is_none());
    }

    #[test]
    fn hidden_caret_menu_follows_reverse_video_selection_and_caps_region() {
        let mut c = core(100);
        c.resize(Size { rows: 24, cols: 40 });
        c.pin_rows = 5;
        c.process(b"\x1b[3;1HRUNNING COMMAND\x1b[6;1HQuestion\x1b[8;1H\x1b[7mFirst option\x1b[9;1Hwrapped description\x1b[0m\x1b[10;1HSecond option\x1b[11;1Hdescription\x1b[12;1HThird option\x1b[13;1Hdescription\x1b[15;1HKeyboard help\x1b[?25l");
        c.action(Action::PageUp);
        let (first, dest, height) = c.pin_band().unwrap();
        assert!(first <= 7 && first + height > 8);
        let frozen = c.frame().cells[..(dest - 1) * c.size.cols].to_vec();
        c.process(b"\x1b[8;1H\x1b[0mFirst option\x1b[9;1Hwrapped description\x1b[12;1H\x1b[7mThird option\x1b[13;1Hdescription\x1b[0m\x1b[3;1HCOMMAND UPDATED");
        let (next, dest, height) = c.pin_band().unwrap();
        assert!(next > first && next <= 11 && next + height > 12);
        assert_eq!(c.frame().cells[..(dest - 1) * c.size.cols], frozen);
        c.action(Action::PinResize(50));
        let (source, dest, height) = c.pin_band().unwrap();
        assert_eq!((source, height), (5, 10));
        let live_text: String = c.frame().cells[dest * c.size.cols..]
            .iter()
            .map(|c| c.c)
            .collect();
        assert!(!live_text.contains("COMMAND"));
        assert!(live_text.contains("Question"));
    }

    #[test]
    fn enlarged_visible_composer_does_not_swallow_command_output() {
        let mut c = core(100);
        c.resize(Size { rows: 24, cols: 40 });
        c.pin_rows = 20;
        c.process(b"\x1b[10;1HRUNNING COMMAND\x1b[12;1Hdraft\x1b[?25h");
        c.action(Action::PageUp);
        assert_eq!(c.pin_band(), Some((11, 23, 1)));
    }

    #[test]
    fn pinned_cursor_band_maps_mouse_back_to_original_rows() {
        let mut c = core(100);
        c.resize(Size { rows: 20, cols: 40 });
        c.pin_rows = 5;
        c.process(b"\x1b[5;1Hdraft\x1b[?25h\x1b[?1000h\x1b[?1006h");
        c.action(Action::Scroll(1));
        let (source, dest, _) = c.pin_band().unwrap();
        assert_eq!((source, dest), (4, 19));
        c.action(Action::Mouse {
            button: 0,
            col: 2,
            row: 20,
            release: false,
        });
        assert!(c.pty_out.ends_with(b"\x1b[<0;2;5M"));
        c.pty_out.clear();
        c.action(Action::Mouse {
            button: 0,
            col: 2,
            row: 1,
            release: false,
        });
        assert!(c.pty_out.is_empty());
        c.resize(Size { rows: 2, cols: 2 });
        assert!(c.pin_band().is_none());
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
        let original_size = c.size;
        c.resize(Size { rows: 2, cols: 5 });
        c.process(b"\x1b[2JMOBILE REPAINT");
        c.resize(original_size);
        assert_eq!(text(&c), before, "shrinking must not erase frozen cells");
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
    fn search_unicode_snapshot_never_copies() {
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
        assert!(!out.contains("\x1b]52;"));
        assert!(text(&c).contains("target"));
        assert_eq!(
            c.document
                .as_ref()
                .unwrap()
                .find("中文 target", None, false),
            Some((1, 0))
        );
        c.action(Action::LocalKey(Key::Escape));
        assert_eq!(c.input_mode(), InputMode::Normal);
        assert_eq!(c.mode, Mode::Browse);
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
    fn search_keeps_spaces_at_soft_wraps() {
        let mut first = vec![Cell::default(); 4];
        first[0].c = '你';
        first[1].flags.insert(Flags::WIDE_CHAR_SPACER);
        first[2].c = ' ';
        first[3].c = ' ';
        first[3].flags.insert(Flags::WRAPLINE);
        let mut second = vec![Cell::default(); 4];
        second[0].c = '好';
        second[1].c = '!';
        let doc = Document {
            rows: vec![first, second],
            wrapped: vec![true, false],
            cols: 4,
            top: 0,
            cursor: (1, 1),
        };
        assert_eq!(doc.find("你  好", None, false), Some((0, 0)));
        assert_eq!(doc.find("  好!", None, false), Some((0, 2)));
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
        c.action(Action::Search);
        let before = text(&c);
        c.action(Action::Help);
        c.action(Action::LocalKey(Key::Escape));
        assert_eq!(c.mode, Mode::Browse);
        assert!(c.document.is_some());
        assert_eq!(text(&c), before);
        c.process(b"\x1b[2Jchanged");
        c.action(Action::Search);
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
    fn drag_in_pinned_history_is_ignored_even_when_child_tracks_mouse() {
        let mut c = core(50);
        c.pin_rows = 6;
        c.process(b"history text\x1b[?1002h\x1b[?1006h");
        c.action(Action::PageUp);
        c.action(Action::Mouse {
            button: 0,
            col: 1,
            row: 1,
            release: false,
        });
        c.action(Action::Mouse {
            button: 32,
            col: 7,
            row: 1,
            release: false,
        });
        c.action(Action::Mouse {
            button: 0,
            col: 7,
            row: 1,
            release: true,
        });
        assert_eq!(c.mode, Mode::Browse);
        assert!(c.pty_out.is_empty());
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
        let original = c.document.as_ref().unwrap().rows.clone();
        c.process(b"\x1b[2Jreplacement\r\n");
        c.action(Action::Search);
        assert_eq!(c.document.as_ref().unwrap().rows.clone(), original);
        c.action(Action::LocalKey(Key::Escape));
        assert_eq!(c.mode, Mode::Browse);
        c.action(Action::PageDown);
        assert_eq!(c.mode, Mode::Follow);
        assert!(c.document.is_none());
        assert!(text(&c).contains("replacement"));
    }
}
