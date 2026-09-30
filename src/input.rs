//! Incremental parser for input from the outer terminal.
//!
//! `Scroll` is positive towards older output. Mouse coordinates are 1-based.
//! Call `flush_timeout` after a short idle period to release an ambiguous bare
//! Escape (or an incomplete escape sequence) without swallowing input.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Forward(Vec<u8>),
    TerminalReply(Vec<u8>),
    Scroll(i32),
    PageUp,
    PageDown,
    Bottom,
    PinToggle,
    PinResize(i32),
    Search,
    Requests,
    Help,
    QuitBrowse,
    Mouse {
        button: u16,
        col: u16,
        row: u16,
        release: bool,
    },
    LocalKey(Key),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    Tab,
    Ctrl(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Browse,
    Local,
}

pub struct Router {
    mode: InputMode,
    pending: Vec<u8>,
    consumed: usize,
    paste: bool,
    paste_utf8: Vec<u8>,
    prefix: Option<Vec<u8>>,
    pass_next: bool,
    reply_wait: Option<Instant>,
    deferred: VecDeque<Action>,
    detachable: bool,
    detach_requested: bool,
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl Router {
    pub fn new() -> Self {
        Self {
            mode: InputMode::Normal,
            pending: Vec::new(),
            consumed: 0,
            paste: false,
            paste_utf8: Vec::new(),
            prefix: None,
            pass_next: false,
            reply_wait: None,
            deferred: VecDeque::new(),
            detachable: false,
            detach_requested: false,
        }
    }

    pub fn set_mode(&mut self, mode: InputMode) {
        self.mode = mode;
        self.prefix = None;
        self.pass_next = false;
    }

    pub fn set_detachable(&mut self, enabled: bool) {
        self.detachable = enabled;
    }

    pub fn detach_requested(&self) -> bool {
        self.detach_requested
    }

    pub fn has_pending(&self) -> bool {
        self.consumed < self.pending.len() || !self.deferred.is_empty()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Action> {
        self.append(bytes);
        self.process(false, false)
    }

    /// Accept a whole read, then stop before the next action can change input mode.
    pub fn feed_step(&mut self, bytes: &[u8]) -> Vec<Action> {
        self.append(bytes);
        self.process(false, true)
    }

    /// Continue parsing the buffered read after applying the preceding action.
    pub fn drain_step(&mut self) -> Vec<Action> {
        self.process(false, true)
    }

    fn append(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if self.consumed > 0 {
            self.pending.drain(..self.consumed);
            self.consumed = 0;
        }
        self.pending.extend_from_slice(bytes);
    }

    pub fn flush_timeout(&mut self) -> Vec<Action> {
        self.process(true, false)
    }

    fn process(&mut self, timeout: bool, one_action: bool) -> Vec<Action> {
        let mut out = Vec::new();
        if one_action {
            if let Some(action) = self.deferred.pop_front() {
                return vec![action];
            }
        } else {
            out.extend(self.deferred.drain(..));
        }
        let pending = std::mem::take(&mut self.pending);
        let mut at = self.consumed;
        const PASTE_END: &[u8] = b"\x1b[201~";
        loop {
            if one_action && !out.is_empty() {
                break;
            }
            if at == pending.len() {
                break;
            }
            if self.paste {
                if pending[at..].starts_with(PASTE_END) {
                    at += PASTE_END.len();
                    self.paste = false;
                    if self.mode != InputMode::Local {
                        forward(&mut out, PASTE_END);
                    } else {
                        self.local_paste_finish(&mut out);
                    }
                    continue;
                }
                if PASTE_END.starts_with(&pending[at..]) {
                    break;
                }
                if self.mode != InputMode::Local {
                    let n = pending[at..]
                        .iter()
                        .position(|b| *b == 0x1b)
                        .unwrap_or(pending.len() - at)
                        .max(1);
                    forward(&mut out, &pending[at..at + n]);
                    at += n;
                } else {
                    self.local_paste_byte(pending[at], &mut out);
                    at += 1;
                }
                continue;
            }
            // In wrapper dialogs, Escape followed by another escape sequence
            // is a close key, not an Alt+Escape token to discard. Preserve the
            // second sequence for mouse reports, arrows and terminal replies.
            if self.mode == InputMode::Local
                && self.prefix.is_none()
                && !self.pass_next
                && pending[at..].starts_with(b"\x1b\x1b")
            {
                out.push(Action::LocalKey(Key::Escape));
                at += 1;
                continue;
            }
            if self.mode != InputMode::Local
                && self.prefix.is_none()
                && !self.pass_next
                && pending[at] != 0x1b
                && pending[at] != 0x1d
                && !(self.mode == InputMode::Browse && pending[at] == 3)
            {
                let n = pending[at..]
                    .iter()
                    .position(|b| {
                        *b == 0x1b || *b == 0x1d || (self.mode == InputMode::Browse && *b == 3)
                    })
                    .unwrap_or(pending.len() - at);
                forward(&mut out, &pending[at..at + n]);
                at += n;
                continue;
            }
            let token_len = match token_len(
                &pending[at..],
                self.mode == InputMode::Local || self.pass_next || self.prefix.is_some(),
            ) {
                Some(n) => n,
                None if timeout => {
                    if reply_prefix(&pending[at..]) {
                        let deadline = self
                            .reply_wait
                            .get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
                        if Instant::now() < *deadline && pending.len() - at < 4 * 1024 * 1024 {
                            break;
                        }
                        out.push(Action::TerminalReply(pending[at..].to_vec()));
                        at = pending.len();
                        self.reply_wait = None;
                        break;
                    }
                    // An incomplete sequence is still the user's input. In normal
                    // mode return every byte; in local mode only a bare Escape is a key.
                    let bytes = pending[at..].to_vec();
                    at = pending.len();
                    if let Some(prefix) = self.prefix.take() {
                        forward(&mut out, &prefix);
                    }
                    if self.pass_next {
                        forward(&mut out, &bytes);
                        self.pass_next = false;
                    } else if self.mode != InputMode::Local {
                        forward(&mut out, &bytes);
                    } else if bytes == [0x1b] {
                        out.push(Action::LocalKey(Key::Escape));
                    }
                    break;
                }
                None => break,
            };
            self.reply_wait = None;
            let token = pending[at..at + token_len].to_vec();
            at += token_len;
            if self.pass_next {
                if kitty_release(&token) {
                    continue;
                }
                forward(&mut out, &token);
                self.pass_next = false;
                if token == b"\x1b[200~" {
                    self.paste = true;
                }
                continue;
            }
            if let Some(prefix) = self.prefix.take() {
                if kitty_release(&token) {
                    self.prefix = Some(prefix);
                    continue;
                }
                match command_char(&token) {
                    Some('d') if self.detachable => self.detach_requested = true,
                    Some('b') => out.push(Action::Bottom),
                    Some('i') => out.push(Action::PinToggle),
                    Some('+') | Some('=') => out.push(Action::PinResize(2)),
                    Some('-') => out.push(Action::PinResize(-2)),
                    Some('/') => out.push(Action::Search),
                    Some('r') => out.push(Action::Requests),
                    Some('?') => out.push(Action::Help),
                    Some('p') => self.pass_next = true,
                    Some('\x1d') => forward(&mut out, &token),
                    _ => {
                        forward(&mut out, &prefix);
                        forward(&mut out, &token);
                    }
                }
                continue;
            }
            if is_prefix_key(&token) {
                self.prefix = Some(token);
                continue;
            }
            if kitty_release(&token) {
                if self.mode != InputMode::Local && normal_action(&token).is_none() {
                    forward(&mut out, &token);
                }
                continue;
            }
            if token == b"\x1b[200~" {
                self.paste = true;
                if self.mode != InputMode::Local {
                    forward(&mut out, &token);
                }
                continue;
            }
            if terminal_reply(&token) {
                out.push(Action::TerminalReply(token));
            } else if self.mode != InputMode::Local {
                if self.mode == InputMode::Browse
                    && matches!(local_action(&token), Some(Action::LocalKey(Key::Ctrl(3))))
                {
                    out.push(Action::QuitBrowse);
                } else if let Some(action) = normal_action(&token) {
                    out.push(action);
                } else {
                    forward(&mut out, &token);
                }
            } else if let Some(action) = local_action(&token) {
                if let Action::Forward(bytes) = &action {
                    forward(&mut out, bytes);
                } else {
                    out.push(action);
                }
            }
        }
        self.pending = pending;
        self.consumed = at;
        if self.consumed == self.pending.len() {
            self.pending.clear();
            self.consumed = 0;
        }
        if one_action && out.len() > 1 {
            self.deferred.extend(out.drain(1..));
        }
        out
    }

    fn local_paste_byte(&mut self, byte: u8, out: &mut Vec<Action>) {
        self.paste_utf8.push(byte);
        loop {
            match std::str::from_utf8(&self.paste_utf8) {
                Ok(s) if !s.is_empty() => {
                    for c in s.chars() {
                        out.push(Action::LocalKey(Key::Char(c)));
                    }
                    self.paste_utf8.clear();
                    break;
                }
                Ok(_) => break,
                Err(e) if e.valid_up_to() > 0 => {
                    let valid = self.paste_utf8.drain(..e.valid_up_to()).collect::<Vec<_>>();
                    for c in std::str::from_utf8(&valid).unwrap().chars() {
                        out.push(Action::LocalKey(Key::Char(c)));
                    }
                }
                Err(e) if e.error_len().is_some() => {
                    out.push(Action::LocalKey(Key::Char('\u{fffd}')));
                    self.paste_utf8.remove(0);
                }
                Err(_) => break,
            }
        }
    }

    fn local_paste_finish(&mut self, out: &mut Vec<Action>) {
        if !self.paste_utf8.is_empty() {
            out.push(Action::LocalKey(Key::Char('\u{fffd}')));
            self.paste_utf8.clear();
        }
    }
}

fn forward(out: &mut Vec<Action>, bytes: &[u8]) {
    if let Some(Action::Forward(previous)) = out.last_mut() {
        previous.extend_from_slice(bytes);
    } else {
        out.push(Action::Forward(bytes.to_vec()));
    }
}

// A token is one key, a complete terminal sequence, or one UTF-8 scalar in
// local/prefix mode. Normal text is forwarded one byte at a time and coalesced.
fn token_len(bytes: &[u8], unicode: bool) -> Option<usize> {
    let first = *bytes.first()?;
    if first != 0x1b {
        if !unicode || first < 0x80 {
            return Some(1);
        }
        let width = match first {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        if bytes.len() < width {
            return None;
        }
        return Some(width);
    }
    if bytes.len() == 1 {
        return None;
    }
    match bytes[1] {
        b'[' => {
            if bytes.len() >= 3 && bytes[2] == b'M' {
                return (bytes.len() >= 6).then_some(6); // X10 mouse
            }
            for (i, &b) in bytes.iter().enumerate().skip(2) {
                if (0x40..=0x7e).contains(&b) {
                    return Some(i + 1);
                }
                if !(0x20..=0x3f).contains(&b) {
                    return Some(1);
                }
            }
            None
        }
        b']' | b'P' | b'_' | b'^' | b'X' => {
            for (i, &b) in bytes.iter().enumerate().skip(2) {
                if b == 0x07 && bytes[1] == b']' {
                    return Some(i + 1);
                }
                if b == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                    return Some(i + 2);
                }
            }
            None
        }
        b'O' => (bytes.len() >= 3).then_some(3),
        _ => Some(2),
    }
}

fn normal_action(token: &[u8]) -> Option<Action> {
    if token.starts_with(b"\x1b[M") && token.len() == 6 {
        let b = token[3].checked_sub(32)? as u16;
        let col = token[4].checked_sub(32)? as u16;
        let row = token[5].checked_sub(32)? as u16;
        return mouse_action(b, col, row, b & 3 == 3);
    }
    let (params, final_byte) = csi(token)?;
    if (final_byte == b'M' || final_byte == b'm') && params.starts_with(b"<") {
        let mut parts = params[1..].split(|&b| b == b';');
        let button = number(parts.next()?)?;
        let col = number(parts.next()?)?;
        let row = number(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        return mouse_action(button, col, row, final_byte == b'm');
    }
    let mut fields = params.split(|&b| b == b';');
    let key = fields.next()?;
    let modifiers = fields.next().unwrap_or(b"1").split(|b| *b == b':').next()?;
    // Only bare paging keys belong to the wrapper. Modified paging keys are
    // application shortcuts, just like modified arrows and function keys.
    if modifiers != b"1" && !modifiers.is_empty() {
        return None;
    }
    match final_byte {
        b'~' => match key {
            b"5" => Some(Action::PageUp),
            b"6" => Some(Action::PageDown),
            _ => None,
        },
        // Kitty assigns these codes to keypad Page Up/Down.
        b'u' => match key {
            b"57421" => Some(Action::PageUp),
            b"57422" => Some(Action::PageDown),
            _ => None,
        },
        _ => None,
    }
}

fn mouse_action(button: u16, col: u16, row: u16, release: bool) -> Option<Action> {
    if col == 0 || row == 0 {
        return None;
    }
    match button & 0x43 {
        64 => Some(Action::Scroll(3)),
        65 => Some(Action::Scroll(-3)),
        _ => Some(Action::Mouse {
            button,
            col,
            row,
            release,
        }),
    }
}

fn csi(token: &[u8]) -> Option<(&[u8], u8)> {
    if token.len() < 3 || &token[..2] != b"\x1b[" {
        return None;
    }
    Some((&token[2..token.len() - 1], *token.last()?))
}

fn number(bytes: &[u8]) -> Option<u16> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

fn decimal(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

struct KittyKey {
    code: u32,
    shifted: Option<u32>,
    base: Option<u32>,
    modifiers: u32,
    event: u32,
}

fn kitty_key(token: &[u8]) -> Option<KittyKey> {
    let (params, final_byte) = csi(token)?;
    if final_byte != b'u' {
        return None;
    }
    let mut fields = params.split(|b| *b == b';');
    let mut codes = fields.next()?.split(|b| *b == b':');
    let code = decimal(codes.next()?)?;
    let shifted = codes.next().and_then(decimal);
    let base = codes.next().and_then(decimal);
    let mut modifier_fields = fields.next().unwrap_or(b"1").split(|b| *b == b':');
    let modifiers = decimal(modifier_fields.next()?)?.checked_sub(1)?;
    let event = modifier_fields.next().and_then(decimal).unwrap_or(1);
    if !(1..=3).contains(&event) {
        return None;
    }
    Some(KittyKey {
        code,
        shifted,
        base,
        modifiers,
        event,
    })
}

fn kitty_release(token: &[u8]) -> bool {
    if let Some(key) = kitty_key(token) {
        return key.event == 3;
    }
    let Some((params, _)) = csi(token) else {
        return false;
    };
    params
        .split(|b| *b == b';')
        .nth(1)
        .and_then(|m| m.split(|b| *b == b':').nth(1))
        .and_then(decimal)
        == Some(3)
}

fn is_prefix_chord(token: &[u8]) -> bool {
    kitty_key(token).is_some_and(|key| {
        key.event != 3
            && key.modifiers & 4 != 0
            && key.modifiers & !5 == 0
            && (key.code == 93 || key.base == Some(93))
    })
}

fn is_prefix_key(token: &[u8]) -> bool {
    token == [0x1d] || is_prefix_chord(token)
}

fn shifted_char(code: u32) -> Option<char> {
    if let Some(ch) = char::from_u32(code) {
        if ch.is_ascii_lowercase() {
            return Some(ch.to_ascii_uppercase());
        }
    }
    let shifted = match code {
        49..=57 => b"!@#$%^&*("[(code - 49) as usize] as char,
        48 => ')',
        45 => '_',
        61 => '+',
        91 => '{',
        93 => '}',
        92 => '|',
        59 => ':',
        39 => '"',
        44 => '<',
        46 => '>',
        47 => '?',
        96 => '~',
        _ => return char::from_u32(code),
    };
    Some(shifted)
}

fn command_char(token: &[u8]) -> Option<char> {
    if is_prefix_key(token) {
        return Some('\x1d');
    }
    if token.len() == 1 {
        return Some(token[0] as char);
    }
    let key = kitty_key(token)?;
    if key.event == 3 || key.modifiers & !1 != 0 {
        return None;
    }
    let shifted = key.modifiers & 1 != 0;
    let primary = if shifted {
        key.shifted
            .and_then(char::from_u32)
            .or_else(|| shifted_char(key.code))
    } else {
        char::from_u32(key.code)
    };
    if primary.is_some_and(|ch| matches!(ch, 'b' | 'd' | '/' | '[' | '?' | 'p')) {
        return primary;
    }
    key.base
        .and_then(|code| {
            if shifted {
                shifted_char(code)
            } else {
                char::from_u32(code)
            }
        })
        .or(primary)
}

fn reply_prefix(bytes: &[u8]) -> bool {
    if bytes.len() < 2 {
        return false;
    }
    [b"\x1b]10;".as_slice(), b"\x1b]11;", b"\x1b]52;", b"\x1b[?"]
        .iter()
        .any(|prefix| prefix.starts_with(bytes) || bytes.starts_with(prefix))
}

fn terminal_reply(token: &[u8]) -> bool {
    if let Some((params, final_byte)) = csi(token) {
        return (final_byte == b'c' && (params.starts_with(b"?") || params.starts_with(b">")))
            || (final_byte == b'u' && params.starts_with(b"?"));
    }
    token.starts_with(b"\x1b]10;")
        || token.starts_with(b"\x1b]11;")
        || token.starts_with(b"\x1b]52;")
}

fn local_action(token: &[u8]) -> Option<Action> {
    if token == b"\x1b[27;5;99~" {
        return Some(Action::LocalKey(Key::Ctrl(3)));
    }
    if let Some(key) = kitty_key(token) {
        if key.event == 3 || key.modifiers & !(1 | 4) != 0 {
            return None;
        }
        let code = key.code;
        let value = match code {
            13 => Key::Enter,
            27 => Key::Escape,
            9 => Key::Tab,
            127 => Key::Backspace,
            1..=26 => Key::Ctrl(code as u8),
            65..=90 | 97..=122 if key.modifiers & 4 != 0 => {
                Key::Ctrl((code as u8).to_ascii_lowercase() - b'a' + 1)
            }
            _ if key.modifiers & 4 != 0 => return None,
            _ => {
                let ch = if key.modifiers & 1 != 0 {
                    shifted_char(key.shifted.unwrap_or(code))?
                } else {
                    char::from_u32(code)?
                };
                Key::Char(ch)
            }
        };
        return Some(Action::LocalKey(value));
    }
    if token == [0x1b] {
        return Some(Action::LocalKey(Key::Escape));
    }
    if let Some(
        action @ (Action::PageUp | Action::PageDown | Action::Scroll(_) | Action::Mouse { .. }),
    ) = normal_action(token)
    {
        return Some(action);
    }
    if token.len() == 1 {
        return Some(Action::LocalKey(match token[0] {
            b'\r' | b'\n' => Key::Enter,
            b'\t' => Key::Tab,
            0x7f | 0x08 => Key::Backspace,
            0x00..=0x1f => Key::Ctrl(token[0]),
            b => Key::Char(b as char),
        }));
    }
    if let Some((params, final_byte)) = csi(token) {
        let first = params.split(|&b| b == b';').next().unwrap_or_default();
        let key = match final_byte {
            b'A' => Some(Key::Up),
            b'B' => Some(Key::Down),
            b'C' => Some(Key::Right),
            b'D' => Some(Key::Left),
            b'H' => Some(Key::Home),
            b'F' => Some(Key::End),
            b'~' => match first {
                b"1" | b"7" => Some(Key::Home),
                b"4" | b"8" => Some(Key::End),
                b"3" => Some(Key::Delete),
                _ => None,
            },
            _ => None,
        };
        return key.map(Action::LocalKey);
    }
    if token.starts_with(b"\x1bO") && token.len() == 3 {
        let key = match token[2] {
            b'A' => Key::Up,
            b'B' => Key::Down,
            b'C' => Key::Right,
            b'D' => Key::Left,
            b'H' => Key::Home,
            b'F' => Key::End,
            _ => return None,
        };
        return Some(Action::LocalKey(key));
    }
    if let Ok(s) = std::str::from_utf8(token) {
        if s.chars().count() == 1 {
            return Some(Action::LocalKey(Key::Char(s.chars().next()?)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_escape_closes_local_ui_without_eating_the_next_sequence() {
        let mut r = Router::new();
        r.set_mode(InputMode::Local);
        assert_eq!(r.feed(b"\x1b\x1b"), vec![Action::LocalKey(Key::Escape)]);
        assert_eq!(r.flush_timeout(), vec![Action::LocalKey(Key::Escape)]);
        assert_eq!(
            r.feed(b"\x1b\x1b[<65;1;2M"),
            vec![Action::LocalKey(Key::Escape), Action::Scroll(-3)]
        );
        r.set_mode(InputMode::Normal);
        assert_eq!(
            r.feed(b"\x1b\x1b[<65;1;2M"),
            vec![Action::Forward(b"\x1b\x1b[<65;1;2M".to_vec())]
        );
    }
    #[test]
    fn browse_only_reserves_cancel_and_preserves_composer_and_paste() {
        for key in [b"\x03".as_slice(), b"\x1b[99;5u", b"\x1b[27;5;99~"] {
            let mut router = Router::new();
            router.set_mode(InputMode::Browse);
            assert_eq!(router.feed(key), vec![Action::QuitBrowse]);
            router.set_mode(InputMode::Normal);
            assert_eq!(router.feed(key), vec![Action::Forward(key.to_vec())]);
        }
        let mut router = Router::new();
        router.set_mode(InputMode::Browse);
        let keys = b"\x1b[A\x1b[B\ttext\x1b[200~\x03\x1b[201~";
        assert_eq!(router.feed(keys), vec![Action::Forward(keys.to_vec())]);
    }
    #[test]
    fn pin_commands_do_not_consume_native_letters() {
        let mut r = Router::new();
        assert_eq!(
            r.feed(b"\x1di\x1d+\x1d-"),
            vec![
                Action::PinToggle,
                Action::PinResize(2),
                Action::PinResize(-2)
            ]
        );
        assert_eq!(
            r.feed(b"\x1b[93;5u\x1b[61:43;2u"),
            vec![Action::PinResize(2)]
        );
        assert_eq!(r.feed(b"i+-"), vec![Action::Forward(b"i+-".to_vec())]);
    }

    #[test]
    fn detach_is_attach_only_and_never_triggered_by_paste() {
        let mut router = Router::new();
        assert_eq!(
            router.feed(b"\x1dd"),
            vec![Action::Forward(b"\x1dd".to_vec())]
        );
        router.set_detachable(true);
        let paste = b"\x1b[200~\x1dd\x1b[201~";
        assert_eq!(router.feed(paste), vec![Action::Forward(paste.to_vec())]);
        assert!(!router.detach_requested());
        router.set_mode(InputMode::Local);
        assert!(router.feed(b"\x1b[93;5u\x1b[100u").is_empty());
        assert!(router.detach_requested());
    }

    #[test]
    fn ordinary_input_and_unknown_sequences_are_exact() {
        let input = b"abc\xc3\xa9\x00\x1b[?25h\x1b]0;title\x07\x1bOP";
        for split in 0..=input.len() {
            let mut router = Router::new();
            let mut got = router.feed(&input[..split]);
            got.extend(router.feed(&input[split..]));
            got.extend(router.flush_timeout());
            let flat = got
                .into_iter()
                .flat_map(|action| match action {
                    Action::Forward(bytes) => bytes,
                    other => panic!("unexpected action: {other:?}"),
                })
                .collect::<Vec<_>>();
            assert_eq!(flat, input, "split {split}");
        }
    }

    #[test]
    fn shortcuts_and_passthrough_are_distinct() {
        let mut router = Router::new();
        assert_eq!(
            router.feed(b"a\x1db\x1d/\x1d?"),
            vec![
                Action::Forward(b"a".to_vec()),
                Action::Bottom,
                Action::Search,
                Action::Help
            ]
        );
        assert_eq!(
            router.feed(b"\x1d\x1d\x1dz\x1dp\x1b[5;2~"),
            vec![Action::Forward(b"\x1d\x1dz\x1b[5;2~".to_vec())]
        );
        assert_eq!(router.feed(b"\x1b[6~"), vec![Action::PageDown]);
    }

    #[test]
    fn mouse_and_pages_survive_every_split() {
        let cases = [
            (b"\x1b[<64;12;30M".as_slice(), Action::Scroll(3)),
            (b"\x1b[<65;12;30M".as_slice(), Action::Scroll(-3)),
            (
                b"\x1b[<0;12;30m".as_slice(),
                Action::Mouse {
                    button: 0,
                    col: 12,
                    row: 30,
                    release: true,
                },
            ),
            (b"\x1b[M`-?".as_slice(), Action::Scroll(3)),
            (b"\x1b[5;1~".as_slice(), Action::PageUp),
            (b"\x1b[6;1:1~".as_slice(), Action::PageDown),
        ];
        for (bytes, expected) in cases {
            for split in 0..=bytes.len() {
                let mut router = Router::new();
                let mut got = router.feed(&bytes[..split]);
                got.extend(router.feed(&bytes[split..]));
                assert_eq!(got, vec![expected.clone()], "{bytes:?}, split {split}");
            }
        }
    }

    #[test]
    fn paste_is_literal_even_when_split_at_every_byte() {
        let paste = b"\x1b[200~x\x1db\x1b[5~\x1b[<64;2;2M\x1b[201~";
        for chunk in 1..=paste.len() {
            let mut router = Router::new();
            let mut got = Vec::new();
            for part in paste.chunks(chunk) {
                got.extend(router.feed(part));
            }
            got.extend(router.flush_timeout());
            let bytes = got
                .into_iter()
                .map(|action| match action {
                    Action::Forward(bytes) => bytes,
                    other => panic!("paste shortcut: {other:?}"),
                })
                .flatten()
                .collect::<Vec<_>>();
            assert_eq!(bytes, paste, "chunk {chunk}");
        }
    }

    #[test]
    fn local_paste_is_text_and_replies_are_distinct() {
        let mut router = Router::new();
        router.set_mode(InputMode::Local);
        assert_eq!(
            router.feed(b"\x1b[200~\x1db/\xc3\xa9\n\x1b[201~"),
            vec![
                Action::LocalKey(Key::Char('\x1d')),
                Action::LocalKey(Key::Char('b')),
                Action::LocalKey(Key::Char('/')),
                Action::LocalKey(Key::Char('é')),
                Action::LocalKey(Key::Char('\n')),
            ]
        );
        assert_eq!(
            router.feed(b"\x1b[2;3R\x1b[?1;2c\x1b]52;c;QQ==\x07"),
            vec![
                Action::TerminalReply(b"\x1b[?1;2c".to_vec()),
                Action::TerminalReply(b"\x1b]52;c;QQ==\x07".to_vec()),
            ]
        );
        assert_eq!(
            router.feed(b"\x1b[A\x7f\x03"),
            vec![
                Action::LocalKey(Key::Up),
                Action::LocalKey(Key::Backspace),
                Action::LocalKey(Key::Ctrl(3)),
            ]
        );
    }

    #[test]
    fn timeout_releases_escape_but_keeps_prefix_armed() {
        let mut router = Router::new();
        assert!(router.feed(b"\x1b").is_empty());
        assert!(router.has_pending());
        assert_eq!(
            router.flush_timeout(),
            vec![Action::Forward(b"\x1b".to_vec())]
        );
        assert!(router.feed(b"\x1d").is_empty());
        assert!(!router.has_pending());
        assert!(router.flush_timeout().is_empty());
        assert_eq!(router.feed(b"b"), vec![Action::Bottom]);
        router.set_mode(InputMode::Local);
        assert!(router.feed(b"\x1b").is_empty());
        assert_eq!(router.flush_timeout(), vec![Action::LocalKey(Key::Escape)]);
    }
    #[test]
    fn clipboard_reply_has_its_own_protocol_action() {
        let mut router = Router::new();
        assert_eq!(
            router.feed(b"a\x1b]52;c;QQ==\x07b"),
            vec![
                Action::Forward(b"a".to_vec()),
                Action::TerminalReply(b"\x1b]52;c;QQ==\x07".to_vec()),
                Action::Forward(b"b".to_vec()),
            ]
        );
    }
    #[test]
    fn pasted_osc52_is_literal_payload() {
        let mut router = Router::new();
        let bytes = b"\x1b[200~\x1b]52;c;QQ==\x07\x1b[201~";
        assert_eq!(router.feed(bytes), vec![Action::Forward(bytes.to_vec())]);
    }

    #[test]
    fn prefix_works_in_local_and_kitty_modes() {
        let mut router = Router::new();
        router.set_mode(InputMode::Local);
        assert_eq!(router.feed(b"\x1db"), vec![Action::Bottom]);
        assert_eq!(
            router.feed(b"\x1b[93;5u\x1b[93;5:3u\x1b[98u"),
            vec![Action::Bottom]
        );
        assert_eq!(router.feed(b"\x1b[93;5u\x1b[47:63;2u"), vec![Action::Help]);
        assert_eq!(
            router.feed(b"\x1b[93;5u\x1b[1077::98u"),
            vec![Action::Bottom]
        );
        assert_eq!(
            router.feed(b"\x1b[93;5u\x1b[93;5u"),
            vec![Action::Forward(b"\x1b[93;5u".to_vec())]
        );
    }

    #[test]
    fn kitty_local_keys_decode_modifiers_unicode_and_events() {
        let mut router = Router::new();
        router.set_mode(InputMode::Local);
        assert_eq!(
            router.feed(b"\x1b[99;5u\x1b[97;2u\x1b[128512u\x1b[27u\x1b[27;1:3u\x1b[1;5:3A"),
            vec![
                Action::LocalKey(Key::Ctrl(3)),
                Action::LocalKey(Key::Char('A')),
                Action::LocalKey(Key::Char('😀')),
                Action::LocalKey(Key::Escape),
            ]
        );
    }

    #[test]
    fn shifted_f3_and_unreserved_kitty_keys_remain_child_input() {
        let mut router = Router::new();
        let bytes =
            b"\x1b[1;2R\x1b[128512;3u\x1b[99;5u\x1b[1;2D\x1b[Z\x1b[5;3~\x1b[6;2:1~\x1b[57421;2u";
        assert_eq!(router.feed(bytes), vec![Action::Forward(bytes.to_vec())]);
    }

    #[test]
    fn step_parser_applies_mode_change_inside_one_read() {
        let mut router = Router::new();
        assert_eq!(
            router.feed_step(b"a\x1d/\x1b[97u"),
            vec![Action::Forward(b"a".to_vec())]
        );
        assert_eq!(router.drain_step(), vec![Action::Search]);
        router.set_mode(InputMode::Local);
        assert_eq!(router.drain_step(), vec![Action::LocalKey(Key::Char('a'))]);
        assert!(router.drain_step().is_empty());
    }

    #[test]
    fn step_parser_delivers_one_local_paste_action_at_a_time() {
        let mut router = Router::new();
        router.set_mode(InputMode::Local);
        assert_eq!(
            router.feed_step(b"\x1b[200~\xc2a"),
            vec![Action::LocalKey(Key::Char('\u{fffd}'))]
        );
        // An invalid UTF-8 lead followed by ASCII produces two edits from one byte.
        assert_eq!(router.drain_step(), vec![Action::LocalKey(Key::Char('a'))]);
    }

    #[test]
    fn partial_query_reply_waits_for_fragment() {
        let mut router = Router::new();
        assert!(router.feed(b"\x1b]10;rgb:ff/").is_empty());
        assert!(router.flush_timeout().is_empty());
        assert_eq!(
            router.feed(b"00/00\x1b\\"),
            vec![Action::TerminalReply(
                b"\x1b]10;rgb:ff/00/00\x1b\\".to_vec()
            )]
        );
    }
}
