//! Delegate missing scrollback to Codex's native full-transcript search.
//! Only fixed UI chrome is recognized; conversation/tool text is never rendered here.
use crate::input::{Action, Key};
use std::time::{Duration, Instant};

enum Phase {
    Opening,
    SearchOpening,
    Finding,
    Browsing,
    Closing,
}
pub struct NativeHistory {
    phase: Phase,
    index: usize,
    count: usize,
    notice: &'static str,
    query: String,
    started: Instant,
    editing: bool,
    keyword_search: bool,
    searching: bool,
    pending_scroll: i32,
    cancelled: bool,
}
impl NativeHistory {
    fn request_query(text: &str) -> String {
        // Native search has a 4 KiB query limit. A short substantive line also
        // avoids pasting a whole multi-page request into its one-line editor.
        let line = text
            .lines()
            .find(|s| s.trim().chars().count() >= 12)
            .or_else(|| text.lines().find(|s| !s.trim().is_empty()))
            .unwrap_or("");
        line.trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect()
    }
    pub fn new(requests: &[String], index: usize) -> Self {
        Self {
            phase: Phase::Opening,
            index,
            count: requests.len(),
            notice: "",
            query: Self::request_query(&requests[index]),
            started: Instant::now(),
            editing: false,
            keyword_search: false,
            searching: true,
            pending_scroll: 0,
            cancelled: false,
        }
    }
    pub fn was_cancelled(&self) -> bool {
        self.cancelled
    }
    pub fn is_open(&self) -> bool {
        !matches!(self.phase, Phase::Opening)
    }
    pub fn status(&self) -> String {
        let state = if !self.notice.is_empty() {
            self.notice
        } else {
            match self.phase {
                Phase::Opening | Phase::SearchOpening => "opening · Esc: close",
                Phase::Closing => "closing…",
                Phase::Finding if self.searching => "searching · Esc: close",
                Phase::Finding if self.keyword_search => {
                    "Enter/Ctrl+P: next/prev match · Esc: close"
                }
                _ => "Ctrl+P/Enter: prev/next request · Esc: close",
            }
        };
        if self.keyword_search {
            format!("NATIVE FIND · {state}")
        } else {
            format!("NATIVE HISTORY {}/{} · {state}", self.index + 1, self.count)
        }
    }
    fn paste_query(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x15\x1b[200~");
        out.extend_from_slice(self.query.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
    }
    /// Request navigation is separate from matching occurrences of one query.
    pub fn navigate_request(
        &mut self,
        action: &Action,
        requests: &[String],
        out: &mut Vec<u8>,
    ) -> bool {
        if self.keyword_search || matches!(self.phase, Phase::Closing) {
            return false;
        }
        let reverse = match action {
            Action::LocalKey(Key::Ctrl(16)) => true,
            Action::LocalKey(Key::Enter | Key::Ctrl(14)) => false,
            _ => return false,
        };
        let next = if reverse {
            self.index.checked_sub(1)
        } else {
            self.index.checked_add(1).filter(|i| *i < requests.len())
        };
        let Some(index) = next else {
            self.notice = if reverse {
                "First request · Enter: next · Esc: close"
            } else {
                "Last request · Ctrl+P: previous · Esc: close"
            };
            return true;
        };
        self.index = index;
        self.query = Self::request_query(&requests[index]);
        self.notice = "";
        self.pending_scroll = 0;
        self.searching = true;
        match self.phase {
            Phase::Finding => self.paste_query(out),
            Phase::Browsing => {
                out.push(b'/');
                self.phase = Phase::SearchOpening;
                self.started = Instant::now();
            }
            _ => {} // The opening handshake will paste the latest chosen request.
        }
        true
    }
    pub fn tick(&mut self, screen: &str, out: &mut Vec<u8>) -> bool {
        let transcript = screen
            .lines()
            .next()
            .is_some_and(|s| s.trim_start().starts_with("/ T R A N S C R I P T"));
        let footer: Vec<_> = screen.lines().rev().take(5).map(str::trim_start).collect();
        let find = footer.iter().any(|s| s.starts_with("Find:"));
        match self.phase {
            Phase::Closing if !transcript => return false,
            Phase::Opening if transcript => {
                if self.cancelled {
                    out.push(b'q');
                    self.phase = Phase::Closing;
                    return true;
                }
                out.push(b'/');
                self.phase = Phase::SearchOpening;
            }
            Phase::SearchOpening if transcript && find => {
                // No Enter is generated, so this can never submit a model task.
                self.paste_query(out);
                self.phase = Phase::Finding;
            }
            Phase::Opening | Phase::SearchOpening
                if self.started.elapsed() > Duration::from_secs(10) =>
            {
                return false;
            }
            Phase::Finding if transcript && find => {
                if footer
                    .iter()
                    .any(|s| s.starts_with("Searching") || s.starts_with("Loading…"))
                {
                    self.searching = true;
                    if self.keyword_search {
                        self.notice = "";
                    }
                } else if footer.iter().any(|s| {
                    (self.editing && s.starts_with("Type to find"))
                        || s.starts_with("enter next")
                        || s.starts_with("No matches")
                        || s.starts_with("No more matches")
                        || s.starts_with("History unavailable")
                }) {
                    self.searching = false;
                    if self.keyword_search {
                        self.notice = if footer.iter().any(|s| s.starts_with("No more matches")) {
                            "No more matches · Esc: close"
                        } else if footer.iter().any(|s| s.starts_with("No matches")) {
                            "No matches · edit query or Esc: close"
                        } else {
                            ""
                        };
                    }
                    let delta = std::mem::take(&mut self.pending_scroll);
                    Self::wheel(delta, out);
                }
            }
            // Native wheel navigation to the very bottom ends search itself.
            Phase::Finding if transcript && !find => {
                self.phase = Phase::Browsing;
                self.keyword_search = false;
                self.searching = false;
            }
            _ => {}
        }
        true
    }
    fn wheel(delta: i32, out: &mut Vec<u8>) {
        // Native Find consumes all keyboard navigation; mouse wheel is the
        // supported way to move its matched viewport without cancelling it.
        for _ in 0..delta.unsigned_abs().div_ceil(3).min(100) {
            out.extend_from_slice(if delta > 0 {
                b"\x1b[<64;1;2M"
            } else {
                b"\x1b[<65;1;2M"
            });
        }
    }
    fn scroll(&mut self, delta: i32, out: &mut Vec<u8>) {
        self.editing = false;
        if self.searching && matches!(self.phase, Phase::Finding) {
            self.pending_scroll = (self.pending_scroll + delta).clamp(-300, 300);
        } else {
            Self::wheel(delta, out);
        }
    }
    fn cancel_search(&mut self, out: &mut Vec<u8>) {
        if matches!(self.phase, Phase::Finding | Phase::SearchOpening) {
            // A bare Escape followed by q would be interpreted as Alt+Q.
            out.extend_from_slice(b"\x1b[27u");
            self.phase = Phase::Browsing;
            self.keyword_search = false;
        }
    }
    // Keep input local until Codex acknowledges closing, so repeated Escape or
    // Ctrl+C cannot reach the composer or cancel a running task.
    pub fn action(&mut self, action: &Action, out: &mut Vec<u8>, rows: usize) {
        if matches!(self.phase, Phase::Closing) {
            return;
        }
        let close = matches!(
            action,
            Action::Bottom | Action::LocalKey(Key::Escape | Key::Ctrl(3) | Key::Ctrl(20))
        ) || (!self.editing && matches!(action, Action::LocalKey(Key::Char('q'))));
        if close {
            if matches!(self.phase, Phase::Opening) {
                // Ctrl+T may still be in flight. Close only once its UI appears.
                self.cancelled = true;
                return;
            }
            self.cancel_search(out);
            self.notice = "";
            self.cancelled = true;
            self.phase = Phase::Closing;
            out.push(b'q');
            return;
        }
        if matches!(self.phase, Phase::SearchOpening) && self.editing {
            // A fast "/query" can arrive in one terminal read, before Codex
            // acknowledges Find. Keep it here until pasting is safe.
            match action {
                Action::LocalKey(Key::Char(c)) => self.query.push(*c),
                Action::LocalKey(Key::Backspace) => {
                    self.query.pop();
                }
                Action::LocalKey(Key::Ctrl(21)) => self.query.clear(),
                _ => {}
            }
            return;
        }
        if matches!(self.phase, Phase::Opening | Phase::SearchOpening) {
            return;
        }
        self.notice = "";
        match action {
            Action::Scroll(delta) => self.scroll(*delta, out),
            Action::PageUp | Action::PageDown => {
                let page = rows.saturating_sub(6).max(3) as i32;
                self.scroll(
                    if matches!(action, Action::PageUp) {
                        page
                    } else {
                        -page
                    },
                    out,
                );
            }
            Action::LocalKey(Key::Up | Key::Down) => self.scroll(
                if matches!(action, Action::LocalKey(Key::Up)) {
                    3
                } else {
                    -3
                },
                out,
            ),
            Action::LocalKey(Key::Home | Key::End) if !self.editing => {
                self.cancel_search(out);
                out.extend_from_slice(if matches!(action, Action::LocalKey(Key::Home)) {
                    b"\x1b[H"
                } else {
                    b"\x1b[F"
                });
            }
            Action::Search | Action::LocalKey(Key::Char('/')) if !self.editing => {
                if matches!(self.phase, Phase::Browsing) {
                    out.push(b'/');
                    self.phase = Phase::SearchOpening;
                    self.query.clear();
                    self.started = Instant::now();
                } else {
                    out.push(0x15);
                }
                self.searching = false;
                self.editing = true;
                self.keyword_search = true;
            }
            Action::LocalKey(Key::Enter | Key::Ctrl(14) | Key::Ctrl(16))
                if matches!(self.phase, Phase::Finding) =>
            {
                out.push(if matches!(action, Action::LocalKey(Key::Ctrl(16))) {
                    16
                } else {
                    14
                });
                self.searching = true;
            }
            Action::LocalKey(key) if self.editing => match key {
                Key::Char(c) => {
                    let mut b = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                }
                Key::Backspace => out.push(127),
                Key::Delete => out.extend_from_slice(b"\x1b[3~"),
                Key::Left => out.extend_from_slice(b"\x1b[D"),
                Key::Right => out.extend_from_slice(b"\x1b[C"),
                Key::Home => out.extend_from_slice(b"\x1b[H"),
                Key::End => out.extend_from_slice(b"\x1b[F"),
                Key::Ctrl(c) => out.push(*c),
                _ => {}
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn finding() -> NativeHistory {
        let mut h = NativeHistory::new(&["Q:\n\nActual request line with enough text".into()], 0);
        let mut out = vec![];
        h.tick("› ordinary composer\nFind:", &mut out);
        assert!(out.is_empty());
        h.tick("/ T R A N S C R I P T /\nq close · f3 find", &mut out);
        assert_eq!(out, b"/");
        out.clear();
        h.tick("/ T R A N S C R I P T /\nFind:\nType to find", &mut out);
        assert_eq!(
            out,
            b"\x15\x1b[200~Actual request line with enough text\x1b[201~"
        );
        h
    }
    #[test]
    fn request_navigation_changes_query_and_reports_boundaries() {
        let requests = vec!["first request".into(), "second request".into()];
        let mut h = finding();
        h.count = requests.len();
        let mut out = vec![];
        assert!(h.navigate_request(&Action::LocalKey(Key::Enter), &requests, &mut out));
        assert_eq!(out, b"\x15\x1b[200~second request\x1b[201~");
        assert!(h.status().contains("2/2"));
        out.clear();
        h.navigate_request(&Action::LocalKey(Key::Enter), &requests, &mut out);
        assert!(out.is_empty());
        assert!(h.status().contains("Last request"));
        h.navigate_request(&Action::LocalKey(Key::Ctrl(16)), &requests, &mut out);
        assert_eq!(out, b"\x15\x1b[200~first request\x1b[201~");
        out.clear();
        h.navigate_request(&Action::LocalKey(Key::Ctrl(16)), &requests, &mut out);
        assert!(out.is_empty());
        assert!(h.status().contains("First request"));
        h.action(&Action::Search, &mut out, 24);
        assert!(!h.navigate_request(&Action::LocalKey(Key::Ctrl(16)), &requests, &mut out));
        out.clear();
        h.action(&Action::LocalKey(Key::Ctrl(16)), &mut out, 24);
        assert_eq!(out, b"\x10");
        h.action(&Action::Scroll(-3), &mut out, 24);
        assert!(!h.navigate_request(&Action::LocalKey(Key::Enter), &requests, &mut out));
        assert!(h.status().starts_with("NATIVE FIND"));
    }
    #[test]
    fn navigation_waits_for_search_and_never_cancels_the_match() {
        let mut h = finding();
        let mut out = vec![];
        h.action(&Action::Scroll(-3), &mut out, 45);
        assert!(out.is_empty());
        h.tick(
            "/ T R A N S C R I P T /\nFind: query\nenter next · ctrl+p previous",
            &mut out,
        );
        assert_eq!(out, b"\x1b[<65;1;2M");
        out.clear();
        h.action(&Action::PageUp, &mut out, 24);
        assert_eq!(out, b"\x1b[<64;1;2M".repeat(6));
        out.clear();
        h.action(&Action::LocalKey(Key::Ctrl(3)), &mut out, 24);
        assert_eq!(out, b"\x1b[27uq");
        assert!(!out.contains(&3));
    }
    #[test]
    fn cancelling_in_flight_open_closes_when_acknowledged() {
        let mut h = NativeHistory::new(&["request".into()], 0);
        let mut out = vec![];
        h.action(&Action::LocalKey(Key::Ctrl(3)), &mut out, 24);
        assert!(out.is_empty());
        assert!(h.tick("/ T R A N S C R I P T /", &mut out));
        assert_eq!(out, b"q");
        out.clear();
        h.action(&Action::LocalKey(Key::Ctrl(3)), &mut out, 24);
        assert!(out.is_empty());
        assert!(!h.tick("› original draft", &mut out));
    }
    #[test]
    fn clearing_search_after_a_resize_finishes_the_busy_state() {
        let mut h = finding();
        let mut out = vec![];
        h.tick("/ T R A N S C R I P T /\nFind: query\nSearching…", &mut out);
        h.action(&Action::Search, &mut out, 24);
        h.tick("/ T R A N S C R I P T /\nFind: query\nSearching…", &mut out);
        h.tick("/ T R A N S C R I P T /\nFind:\nType to find", &mut out);
        assert!(h.status().starts_with("NATIVE FIND"));
        out.clear();
        h.action(&Action::Scroll(-3), &mut out, 24);
        assert_eq!(out, b"\x1b[<65;1;2M");
    }
    #[test]
    fn fast_query_waits_for_the_native_search_editor() {
        let mut h = finding();
        let mut out = vec![];
        h.tick("/ T R A N S C R I P T /\nq close · f3 find", &mut out);
        h.action(&Action::Search, &mut out, 24);
        assert_eq!(out, b"/");
        out.clear();
        for c in "quick query".chars() {
            h.action(&Action::LocalKey(Key::Char(c)), &mut out, 24);
        }
        assert!(out.is_empty());
        h.tick("/ T R A N S C R I P T /\nFind:\nType to find", &mut out);
        assert_eq!(out, b"\x15\x1b[200~quick query\x1b[201~");
    }
    #[test]
    fn native_search_ending_at_bottom_does_not_leave_stale_find_state() {
        let mut h = finding();
        let mut out = vec![];
        h.tick("/ T R A N S C R I P T /\nq close · f3 find", &mut out);
        h.action(&Action::Bottom, &mut out, 24);
        assert_eq!(out, b"q");
    }
}
