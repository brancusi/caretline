//! A headless terminal screen for tests (terminal harness step 3): the bytes the runtime
//! writes, fed to an emulator and read back as cells, styles and the cursor. The emulator is
//! the `vt100` crate behind [`Screen`], so it can be swapped for Ghostty's own
//! (libghostty-vt, which needs Zig to build) if a test ever needs Ghostty's exact behaviour.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use caretline::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{TerminalOptions, Viewport};

use crate::runtime::{Decor, draw};

/// A colour as the terminal holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Default,
    Idx(u8),
    Rgb(u8, u8, u8),
}

/// One cell of the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub symbol: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub reverse: bool,
    pub underline: bool,
    /// The second cell of a wide char.
    pub wide_tail: bool,
}

/// A terminal screen fed with bytes.
pub trait Screen {
    fn feed(&mut self, bytes: &[u8]);
    /// Columns and rows.
    fn size(&self) -> (u16, u16);
    fn cell(&self, x: u16, y: u16) -> Cell;
    /// Where the cursor is, if it shows.
    fn cursor(&self) -> Option<(u16, u16)>;

    /// A row's text, trailing blanks dropped: a cell never written is a space, a wide char's
    /// second cell adds nothing.
    fn row(&self, y: u16) -> String {
        let (w, _) = self.size();
        let mut s = String::new();
        for x in 0..w {
            let c = self.cell(x, y);
            match (c.wide_tail, c.symbol.is_empty()) {
                (true, _) => {}
                (false, true) => s.push(' '),
                (false, false) => s.push_str(&c.symbol),
            }
        }
        s.trim_end().to_string()
    }

    /// Every row's text, joined by newlines.
    fn text(&self) -> String {
        let (_, h) = self.size();
        (0..h).map(|y| self.row(y)).collect::<Vec<_>>().join("\n")
    }
}

/// [`Screen`] over the `vt100` crate.
pub struct Vt100(vt100::Parser);

impl Vt100 {
    pub fn new(width: u16, height: u16) -> Vt100 {
        Vt100(vt100::Parser::new(height, width, 0))
    }
}

fn color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(i) => Color::Idx(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

impl Screen for Vt100 {
    fn feed(&mut self, bytes: &[u8]) {
        self.0.process(bytes);
    }

    fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.0.screen().size();
        (cols, rows)
    }

    fn cell(&self, x: u16, y: u16) -> Cell {
        let Some(c) = self.0.screen().cell(y, x) else {
            return Cell {
                symbol: String::new(),
                fg: Color::Default,
                bg: Color::Default,
                bold: false,
                dim: false,
                reverse: false,
                underline: false,
                wide_tail: false,
            };
        };
        Cell {
            symbol: c.contents().to_string(),
            fg: color(c.fgcolor()),
            bg: color(c.bgcolor()),
            bold: c.bold(),
            dim: c.dim(),
            reverse: c.inverse(),
            underline: c.underline(),
            wide_tail: c.is_wide_continuation(),
        }
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        let s = self.0.screen();
        if s.hide_cursor() {
            return None;
        }
        let (row, col) = s.cursor_position();
        Some((col, row))
    }
}

/// Bytes written into a buffer the painter shares.
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The runtime's painter writing into memory: each [`Painter::paint`] returns the exact bytes
/// the editor would write to the terminal for that frame (a diff after the first).
pub struct Painter {
    terminal: Terminal<CrosstermBackend<Shared>>,
    out: Shared,
}

impl Painter {
    pub fn new(width: u16, height: u16) -> Painter {
        let out = Shared::default();
        let backend = CrosstermBackend::new(out.clone());
        let viewport = Viewport::Fixed(Rect::new(0, 0, width, height));
        let terminal =
            Terminal::with_options(backend, TerminalOptions { viewport }).expect("terminal");
        Painter { terminal, out }
    }

    pub fn paint(&mut self, frame: &Frame) -> Vec<u8> {
        draw(&mut self.terminal, frame, &Decor::default()).expect("draw");
        std::mem::take(&mut *self.out.0.borrow_mut())
    }
}

/// The ratatui colour as the terminal holds it after ratatui writes it.
pub fn term_color(c: Option<ratatui::style::Color>) -> Color {
    use ratatui::style::Color as R;
    match c {
        None | Some(R::Reset) => Color::Default,
        Some(R::Rgb(r, g, b)) => Color::Rgb(r, g, b),
        Some(R::Indexed(i)) => Color::Idx(i),
        Some(named) => Color::Idx(match named {
            R::Black => 0,
            R::Red => 1,
            R::Green => 2,
            R::Yellow => 3,
            R::Blue => 4,
            R::Magenta => 5,
            R::Cyan => 6,
            R::Gray => 7,
            R::DarkGray => 8,
            R::LightRed => 9,
            R::LightGreen => 10,
            R::LightYellow => 11,
            R::LightBlue => 12,
            R::LightMagenta => 13,
            R::LightCyan => 14,
            _ => 15,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caretline::helix::{Range, Selection};
    use caretline::view::Role;
    use caretline::{State, Viewport as Vp, script_to_msgs_for, update, view};

    fn state(text: &str, keys: &str, w: u16, h: u16) -> State {
        let mut s = State::new(
            text,
            None,
            Vp {
                width: w,
                height: h,
            },
        );
        for m in script_to_msgs_for(keys, 0, false).unwrap() {
            update(&mut s, m);
        }
        s
    }

    /// The frame's text, row by row, as the screen shows it.
    fn frame_text(f: &Frame) -> String {
        (0..f.height)
            .map(|y| {
                let s: String = (0..f.width).map(|x| f.cell(x, y).symbol.as_str()).collect();
                s.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// What the frame says each cell looks like, against the screen.
    fn assert_screen_is_frame(screen: &Vt100, f: &Frame) {
        assert_eq!(screen.text(), frame_text(f), "text");
        assert_eq!(screen.cursor(), f.cursor, "cursor");
        for y in 0..f.height {
            for x in 0..f.width {
                let fc = f.cell(x, y);
                if fc.symbol.is_empty() {
                    continue;
                }
                let st = crate::runtime::style_in(f, fc.role);
                let c = screen.cell(x, y);
                assert_eq!(
                    (c.fg, c.bg),
                    (term_color(st.fg), term_color(st.bg)),
                    "colours of ({x},{y}) {:?} {:?}",
                    fc.symbol,
                    fc.role
                );
                let reverse = st.add_modifier.contains(ratatui::style::Modifier::REVERSED);
                assert_eq!(c.reverse, reverse, "reverse at ({x},{y}) {:?}", fc.role);
            }
        }
    }

    #[test]
    fn the_screen_shows_the_frame() {
        let (w, h) = (24, 5);
        let mut painter = Painter::new(w, h);
        let mut screen = Vt100::new(w, h);
        // A selection over a line break, and a wide grapheme.
        let mut s = state(
            "hello world\nsecond 漢字 line\nthird",
            "<s-down><s-right>",
            w,
            h,
        );
        let f = view(&s);
        assert!(f.cells.iter().any(|c| c.role == Role::Selection));
        screen.feed(&painter.paint(&f));
        assert_screen_is_frame(&screen, &f);
        // The next paint is a diff; the screen still matches.
        for m in script_to_msgs_for("<down><end>x", 0, false).unwrap() {
            update(&mut s, m);
        }
        let f = view(&s);
        screen.feed(&painter.paint(&f));
        assert_screen_is_frame(&screen, &f);
        assert_eq!(screen.row(2), "thirdx");
    }

    #[test]
    fn other_carets_are_reverse_cells() {
        let (w, h) = (20, 4);
        let mut painter = Painter::new(w, h);
        let mut screen = Vt100::new(w, h);
        let mut s = state("one\ntwo\nthree", "", w, h);
        // Carets at the start of each line; the last pushed is the primary.
        s.view.selection = Selection::point(0)
            .push(Range::point(4))
            .push(Range::point(8));
        let f = view(&s);
        screen.feed(&painter.paint(&f));
        assert_screen_is_frame(&screen, &f);
        // The text rows (the last is the status bar).
        let carets = (0..h - 1).filter(|&y| screen.cell(0, y).reverse).count();
        assert_eq!(carets, 2, "two carets beside the cursor");
        assert_eq!(screen.cursor(), Some((0, 2)));
    }
}
