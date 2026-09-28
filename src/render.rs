use std::fmt::Write as _;

use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use unicode_width::UnicodeWidthChar;

/// Inclusive viewport coordinates, in row/column order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    pub start: (usize, usize),
    pub end: (usize, usize),
}

#[derive(Clone, Debug)]
pub struct Frame {
    /// Child content height. The renderer uses one additional physical row for status.
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Cell>,
    pub cursor: Option<(usize, usize)>,
    /// DECSCUSR shape (0–6).
    pub cursor_shape: u8,
    pub status: String,
    pub selection: Option<Selection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Style {
    fg: Color,
    bg: Color,
    underline_color: Option<Color>,
    flags: Flags,
}

pub struct Renderer {
    previous: Option<Frame>,
}

impl Renderer {
    pub fn new() -> Self {
        Self { previous: None }
    }

    /// Force a full repaint after the host terminal may have changed independently.
    pub fn invalidate(&mut self) {
        self.previous = None;
    }

    /// Render a frame as terminal output. The caller writes the returned bytes atomically.
    pub fn render(&mut self, frame: &Frame) -> Vec<u8> {
        assert_eq!(frame.cells.len(), frame.rows * frame.cols);

        let old = self.previous.as_ref();
        let full = old.is_none_or(|p| p.rows != frame.rows || p.cols != frame.cols);
        let mut out = String::new();
        let mut style = None;
        let mut link: Option<Hyperlink> = None;

        for row in 0..frame.rows {
            for col in 0..frame.cols {
                let index = row * frame.cols + col;
                let cell = &frame.cells[index];
                let selected = cell_selected(frame, row, col);
                let dirty = full
                    || old.is_some_and(|p| {
                        p.cells[index] != *cell || cell_selected(p, row, col) != selected
                    });
                if !dirty
                    || cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                        && col > 0
                        && frame.cells[index - 1].flags.contains(Flags::WIDE_CHAR)
                {
                    continue;
                }

                if out.is_empty() {
                    out.push_str("\x1b[?25l");
                    if full {
                        out.push_str("\x1b[0m\x1b[2J");
                    }
                }
                move_to(&mut out, row, col);

                let next_link = cell.hyperlink();
                if link != next_link {
                    close_link(&mut out, &mut link);
                    if let Some(ref hyperlink) = next_link {
                        let uri: String = hyperlink
                            .uri()
                            .chars()
                            .filter(|c| !c.is_control())
                            .collect();
                        out.push_str("\x1b]8;;");
                        out.push_str(&uri);
                        out.push_str("\x1b\\");
                    }
                    link = next_link;
                }

                let next_style = Style {
                    fg: cell.fg,
                    bg: cell.bg,
                    underline_color: cell.underline_color(),
                    flags: if selected {
                        cell.flags ^ Flags::INVERSE
                    } else {
                        cell.flags
                    },
                };
                if style != Some(next_style) {
                    emit_style(&mut out, next_style);
                    style = Some(next_style);
                }

                // A spacer ordinarily comes from its preceding wide glyph. If that glyph
                // disappeared, explicitly erase the second column it formerly occupied.
                let spacer = cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
                let invalid_wide_edge =
                    cell.flags.contains(Flags::WIDE_CHAR) && col + 1 == frame.cols;
                out.push(if spacer || invalid_wide_edge || cell.c.is_control() {
                    ' '
                } else {
                    cell.c
                });
                if !spacer && !invalid_wide_edge {
                    if let Some(marks) = cell.zerowidth() {
                        for mark in marks {
                            out.push(*mark);
                        }
                    }
                }
            }
        }

        let status_changed = full || old.is_some_and(|p| p.status != frame.status);
        if status_changed && frame.cols > 0 {
            if out.is_empty() {
                out.push_str("\x1b[?25l");
            }
            close_link(&mut out, &mut link);
            move_to(&mut out, frame.rows, 0);
            out.push_str("\x1b[0;7m");
            let mut width = 0;
            for ch in frame.status.chars() {
                if ch.is_control() {
                    continue;
                }
                let columns = UnicodeWidthChar::width(ch).unwrap_or(0);
                if width + columns > frame.cols {
                    break;
                }
                out.push(ch);
                width += columns;
            }
            for _ in width..frame.cols {
                out.push(' ');
            }
            style = None;
        }

        let cursor_changed = full
            || old
                .is_some_and(|p| p.cursor != frame.cursor || p.cursor_shape != frame.cursor_shape);
        if !out.is_empty() || cursor_changed {
            if out.is_empty() {
                out.push_str("\x1b[?25l");
            }
            close_link(&mut out, &mut link);
            if style.is_some() || status_changed {
                out.push_str("\x1b[0m");
            }
            if let Some((row, col)) = frame
                .cursor
                .filter(|&(r, c)| r < frame.rows && c < frame.cols)
            {
                if frame.cursor_shape <= 6 {
                    write!(out, "\x1b[{} q", frame.cursor_shape).unwrap();
                }
                move_to(&mut out, row, col);
                out.push_str("\x1b[?25h");
            }
        }

        self.previous = Some(frame.clone());
        out.into_bytes()
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

fn is_selected(selection: Option<Selection>, row: usize, col: usize) -> bool {
    selection.is_some_and(|s| {
        let (start, end) = if s.start <= s.end {
            (s.start, s.end)
        } else {
            (s.end, s.start)
        };
        (row, col) >= start && (row, col) <= end
    })
}

fn cell_selected(frame: &Frame, row: usize, col: usize) -> bool {
    let index = row * frame.cols + col;
    if frame.cells[index].flags.contains(Flags::WIDE_CHAR) && col + 1 < frame.cols {
        is_selected(frame.selection, row, col) || is_selected(frame.selection, row, col + 1)
    } else {
        is_selected(frame.selection, row, col)
    }
}

fn move_to(out: &mut String, row: usize, col: usize) {
    write!(out, "\x1b[{};{}H", row + 1, col + 1).unwrap();
}

fn close_link(out: &mut String, link: &mut Option<Hyperlink>) {
    if link.take().is_some() {
        out.push_str("\x1b]8;;\x1b\\");
    }
}

fn emit_style(out: &mut String, style: Style) {
    let flags = style.flags;
    out.push_str("\x1b[0");
    if flags.contains(Flags::BOLD) {
        out.push_str(";1");
    }
    if flags.contains(Flags::DIM) {
        out.push_str(";2");
    }
    if flags.contains(Flags::ITALIC) {
        out.push_str(";3");
    }
    let underline = if flags.contains(Flags::DOUBLE_UNDERLINE) {
        Some(2)
    } else if flags.contains(Flags::UNDERCURL) {
        Some(3)
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        Some(4)
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        Some(5)
    } else if flags.contains(Flags::UNDERLINE) {
        Some(1)
    } else {
        None
    };
    if let Some(kind) = underline {
        write!(out, ";4:{kind}").unwrap();
    }
    if flags.contains(Flags::INVERSE) {
        out.push_str(";7");
    }
    if flags.contains(Flags::HIDDEN) {
        out.push_str(";8");
    }
    if flags.contains(Flags::STRIKEOUT) {
        out.push_str(";9");
    }
    emit_color(out, style.fg, 38, 30, 90, 39);
    emit_color(out, style.bg, 48, 40, 100, 49);
    if let Some(color) = style.underline_color {
        emit_color(out, color, 58, 0, 0, 59);
    }
    out.push('m');
}

fn emit_color(out: &mut String, color: Color, extended: u8, normal: u8, bright: u8, default: u8) {
    match color {
        Color::Spec(rgb) => {
            write!(out, ";{extended};2;{};{};{}", rgb.r, rgb.g, rgb.b).unwrap();
        }
        Color::Indexed(index) => {
            write!(out, ";{extended};5;{index}").unwrap();
        }
        Color::Named(name) => {
            let n = name as usize;
            if extended == 58 {
                out.push_str(";59");
            } else if n < 8 {
                write!(out, ";{}", normal as usize + n).unwrap();
            } else if n < 16 {
                write!(out, ";{}", bright as usize + n - 8).unwrap();
            } else if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&n) {
                write!(
                    out,
                    ";{}",
                    normal as usize + n - NamedColor::DimBlack as usize
                )
                .unwrap();
            } else {
                write!(out, ";{default}").unwrap();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::{
        Term,
        event::VoidListener,
        grid::Dimensions,
        index::{Column, Line},
        term::{Config, TermMode},
        vte::ansi::{CursorShape, Processor, Rgb},
    };

    struct Size {
        rows: usize,
        cols: usize,
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

    fn outer(rows: usize, cols: usize) -> (Term<VoidListener>, Processor) {
        (
            Term::new(Config::default(), &Size { rows, cols }, VoidListener),
            Processor::new(),
        )
    }

    fn feed(term: &mut Term<VoidListener>, parser: &mut Processor, bytes: &[u8]) {
        parser.advance(term, bytes);
    }

    fn at(term: &Term<VoidListener>, row: usize, col: usize) -> &Cell {
        &term.grid()[Line(row as i32)][Column(col)]
    }

    fn frame(cols: usize) -> Frame {
        Frame {
            rows: 1,
            cols,
            cells: vec![Cell::default(); cols],
            cursor: None,
            cursor_shape: 1,
            status: String::new(),
            selection: None,
        }
    }

    #[test]
    fn unchanged_frame_emits_nothing_and_invalidation_repaints() {
        let mut renderer = Renderer::new();
        let f = frame(3);
        assert!(!renderer.render(&f).is_empty());
        assert!(renderer.render(&f).is_empty());
        renderer.invalidate();
        assert!(
            String::from_utf8(renderer.render(&f))
                .unwrap()
                .contains("\x1b[2J")
        );
    }

    #[test]
    fn replacing_wide_glyph_clears_spacer_column() {
        let mut renderer = Renderer::new();
        let mut f = frame(3);
        f.cells[0].c = '界';
        f.cells[0].flags = Flags::WIDE_CHAR;
        f.cells[1].flags = Flags::WIDE_CHAR_SPACER;
        renderer.render(&f);
        f.cells[0] = Cell::default();
        f.cells[1] = Cell::default();
        let output = String::from_utf8(renderer.render(&f)).unwrap();
        assert!(output.contains("\x1b[1;1H"));
        assert!(output.contains("\x1b[1;2H"));
    }

    #[test]
    fn styled_hyperlink_is_closed_before_footer() {
        let mut renderer = Renderer::new();
        let mut f = frame(4);
        f.cells[0].c = 'x';
        f.cells[0].flags = Flags::BOLD | Flags::UNDERCURL;
        f.cells[0].fg = Color::Indexed(42);
        f.cells[0].set_hyperlink(Some(Hyperlink::new(
            None::<String>,
            "https://example.org".into(),
        )));
        f.status = "好a".into();
        let output = String::from_utf8(renderer.render(&f)).unwrap();
        assert!(output.contains("\x1b[0;1;4:3;38;5;42"));
        assert!(output.contains("\x1b]8;;https://example.org\x1b\\"));
        assert!(output.find("\x1b]8;;\x1b\\").unwrap() < output.find("\x1b[2;1H").unwrap());
        assert!(output.contains("好a "));
    }

    #[test]
    fn cursor_is_placed_above_footer() {
        let mut renderer = Renderer::new();
        let mut f = frame(2);
        f.cursor = Some((0, 1));
        let output = String::from_utf8(renderer.render(&f)).unwrap();
        assert!(output.ends_with("\x1b[1 q\x1b[1;2H\x1b[?25h"));
    }

    #[test]
    fn footer_truncates_by_terminal_columns() {
        let mut renderer = Renderer::new();
        let mut f = frame(3);
        f.status = "界ab".into();
        let output = String::from_utf8(renderer.render(&f)).unwrap();
        assert!(output.contains("\x1b[2;1H\x1b[0;7m界a"));
        assert!(!output.contains("界ab"));
    }

    #[test]
    fn selecting_wide_spacer_repaints_leading_glyph() {
        let mut renderer = Renderer::new();
        let mut f = frame(3);
        f.cells[0].c = '界';
        f.cells[0].flags = Flags::WIDE_CHAR;
        f.cells[1].flags = Flags::WIDE_CHAR_SPACER;
        renderer.render(&f);
        f.selection = Some(Selection {
            start: (0, 1),
            end: (0, 1),
        });
        let output = String::from_utf8(renderer.render(&f)).unwrap();
        assert!(output.contains("\x1b[1;1H\x1b[0;7;39;49m界"));
        assert!(!output.contains("\x1b[1;2H"));
    }

    #[test]
    fn parser_roundtrip_wide_combining_color_and_replacement() {
        let (mut term, mut parser) = outer(2, 5);
        let mut renderer = Renderer::new();
        let mut f = frame(5);
        f.cells[1].c = '界';
        f.cells[1].flags = Flags::WIDE_CHAR | Flags::BOLD | Flags::ITALIC | Flags::UNDERLINE;
        f.cells[1].fg = Color::Spec(Rgb { r: 1, g: 2, b: 3 });
        f.cells[1].bg = Color::Indexed(42);
        f.cells[1].set_underline_color(Some(Color::Spec(Rgb { r: 4, g: 5, b: 6 })));
        f.cells[1].push_zerowidth('\u{0301}');
        f.cells[2].flags = Flags::WIDE_CHAR_SPACER;
        f.cells[3].c = 'Z';
        feed(&mut term, &mut parser, &renderer.render(&f));
        assert_eq!(at(&term, 0, 1).c, '界');
        assert_eq!(at(&term, 0, 1).zerowidth().unwrap(), &['\u{0301}']);
        assert!(at(&term, 0, 1).flags.contains(Flags::WIDE_CHAR));
        assert!(
            at(&term, 0, 1)
                .flags
                .contains(Flags::BOLD | Flags::ITALIC | Flags::UNDERLINE)
        );
        assert!(at(&term, 0, 2).flags.contains(Flags::WIDE_CHAR_SPACER));
        assert_eq!(at(&term, 0, 1).fg, f.cells[1].fg);
        assert_eq!(at(&term, 0, 1).bg, f.cells[1].bg);
        assert_eq!(
            at(&term, 0, 1).underline_color(),
            f.cells[1].underline_color()
        );

        f.cells[1] = Cell::default();
        f.cells[1].c = 'a';
        f.cells[2] = Cell::default();
        f.cells[2].c = 'b';
        feed(&mut term, &mut parser, &renderer.render(&f));
        assert_eq!(at(&term, 0, 1).c, 'a');
        assert_eq!(at(&term, 0, 2).c, 'b');
        assert!(!at(&term, 0, 1).flags.contains(Flags::WIDE_CHAR));
        assert!(!at(&term, 0, 2).flags.contains(Flags::WIDE_CHAR_SPACER));
        assert_eq!(at(&term, 0, 3).c, 'Z');
    }

    #[test]
    fn parser_roundtrip_footer_only_update_keeps_frozen_body() {
        let (mut term, mut parser) = outer(2, 5);
        let mut renderer = Renderer::new();
        let mut f = frame(5);
        f.cells[0].c = '界';
        f.cells[0].flags = Flags::WIDE_CHAR;
        f.cells[1].flags = Flags::WIDE_CHAR_SPACER;
        f.cells[2].c = 'Q';
        f.status = "one".into();
        feed(&mut term, &mut parser, &renderer.render(&f));
        let body = (0..5)
            .map(|col| at(&term, 0, col).clone())
            .collect::<Vec<_>>();
        f.status = "二two!".into();
        let delta = renderer.render(&f);
        assert!(!String::from_utf8_lossy(&delta).contains("\x1b[1;"));
        feed(&mut term, &mut parser, &delta);
        assert_eq!(
            body,
            (0..5)
                .map(|col| at(&term, 0, col).clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(at(&term, 1, 0).c, '二');
        assert!(at(&term, 1, 0).flags.contains(Flags::INVERSE));
        assert!(at(&term, 1, 1).flags.contains(Flags::WIDE_CHAR_SPACER));
        assert_eq!(
            (2..5).map(|col| at(&term, 1, col).c).collect::<String>(),
            "two"
        );
        assert!(!term.mode().contains(TermMode::SHOW_CURSOR));

        f.status = "x".into();
        feed(&mut term, &mut parser, &renderer.render(&f));
        assert_eq!(
            (0..5).map(|col| at(&term, 1, col).c).collect::<String>(),
            "x    "
        );
        assert_eq!(
            body,
            (0..5)
                .map(|col| at(&term, 0, col).clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn hyperlink_uri_cannot_inject_terminal_commands() {
        let mut renderer = Renderer::new();
        let mut f = frame(1);
        f.cells[0].set_hyperlink(Some(Hyperlink::new(
            Some("ignored\x1b[2J"),
            "https://site/\x1b[2J\x07ok".into(),
        )));
        let output = String::from_utf8(renderer.render(&f)).unwrap();
        assert!(output.contains("\x1b]8;;https://site/[2Jok\x1b\\"));
        assert_eq!(output.matches("\x1b[2J").count(), 1);
        assert!(!output.contains("ignored"));
    }

    #[test]
    fn parser_roundtrip_cursor_shape_and_browse_hiding() {
        let (mut term, mut parser) = outer(2, 4);
        let mut renderer = Renderer::new();
        let mut f = frame(4);
        f.cursor = Some((0, 2));
        f.cursor_shape = 6;
        feed(&mut term, &mut parser, &renderer.render(&f));
        assert!(term.mode().contains(TermMode::SHOW_CURSOR));
        assert_eq!(
            term.grid().cursor.point,
            alacritty_terminal::index::Point::new(Line(0), Column(2))
        );
        assert_eq!(term.cursor_style().shape, CursorShape::Beam);
        assert!(!term.cursor_style().blinking);

        f.cursor = None;
        feed(&mut term, &mut parser, &renderer.render(&f));
        assert!(!term.mode().contains(TermMode::SHOW_CURSOR));
    }

    #[test]
    fn parser_roundtrip_right_edge_updates_do_not_scroll() {
        let (mut term, mut parser) = outer(2, 3);
        let mut renderer = Renderer::new();
        let mut f = frame(3);
        f.status = "abc".into();
        feed(&mut term, &mut parser, &renderer.render(&f));
        f.cells[2].c = 'X';
        feed(&mut term, &mut parser, &renderer.render(&f));
        assert_eq!(at(&term, 0, 2).c, 'X');
        assert_eq!(
            (0..3).map(|col| at(&term, 1, col).c).collect::<String>(),
            "abc"
        );
        f.cells[2].c = 'Y';
        feed(&mut term, &mut parser, &renderer.render(&f));
        assert_eq!(at(&term, 0, 2).c, 'Y');
        assert_eq!(
            (0..3).map(|col| at(&term, 1, col).c).collect::<String>(),
            "abc"
        );
    }
}
