//! ratatui: [`CellGrid`] for a [`Buffer`], and a [`Theme`] that gives the overlay roles and the
//! dim and ring flags their colours.

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

use crate::compose::{CellGrid, Flags};

type Rgb = (u8, u8, u8);

/// Colours for the overlay roles, and how dim and ring restyle the host's cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// The screen's default foreground and background, used where a cell's colour is `Reset`.
    pub fg: Rgb,
    pub bg: Rgb,
    pub callout_bg: Rgb,
    pub border: Rgb,
    pub key_bg: Rgb,
    pub muted: Rgb,
    /// The agent hue: agents' borders, titles and arrows.
    pub agent: Rgb,
    /// No colours: modifiers only (bold, reverse, faint, underline), for 16 colours and NO_COLOR.
    pub plain: bool,
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::dark()
    }
}

fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

fn rgb((r, g, b): Rgb) -> Color {
    Color::Rgb(r, g, b)
}

impl Theme {
    pub fn dark() -> Theme {
        Theme {
            fg: (0xd8, 0xdc, 0xe6),
            bg: (0x12, 0x14, 0x1a),
            callout_bg: (0x1f, 0x24, 0x30),
            border: (0x8a, 0xa4, 0xff),
            key_bg: (0x33, 0x40, 0x5c),
            muted: (0x6b, 0x72, 0x80),
            agent: (0x5f, 0xb3, 0xff),
            plain: false,
        }
    }

    pub fn light() -> Theme {
        Theme {
            fg: (0x1f, 0x23, 0x28),
            bg: (0xff, 0xff, 0xff),
            callout_bg: (0xf4, 0xf1, 0xea),
            border: (0x3b, 0x5b, 0xdb),
            key_bg: (0xdd, 0xe4, 0xff),
            muted: (0x8a, 0x8f, 0x98),
            agent: (0x1c, 0x6f, 0xd1),
            plain: false,
        }
    }

    /// Modifiers only.
    pub fn plain() -> Theme {
        Theme {
            plain: true,
            ..Theme::dark()
        }
    }

    /// The style of an overlay role (`overlay.callout.border`, `overlay.agent.key`…); `None` for
    /// a role that isn't the overlay's.
    pub fn style(&self, role: &str) -> Option<Style> {
        let rest = role.strip_prefix("overlay.")?;
        let (hue, rest) = match rest.strip_prefix("agent.") {
            Some(r) => (self.agent, r),
            None => (self.border, rest),
        };
        let s = Style::default();
        if self.plain {
            return Some(match rest {
                "callout.border" | "callout.title" | "arrow" | "dots.on" => {
                    s.add_modifier(Modifier::BOLD)
                }
                "key" | "strip" | "chip" => s.add_modifier(Modifier::REVERSED),
                "dots" => s.add_modifier(Modifier::DIM),
                _ => s,
            });
        }
        let panel = s.fg(rgb(self.fg)).bg(rgb(self.callout_bg));
        Some(match rest {
            "callout" | "strip" => panel,
            "callout.border" => panel.fg(rgb(hue)),
            "callout.title" => panel.fg(rgb(hue)).add_modifier(Modifier::BOLD),
            "arrow" => s.fg(rgb(hue)).add_modifier(Modifier::BOLD),
            "key" => panel.bg(rgb(self.key_bg)),
            "code" => panel.fg(rgb(hue)),
            "dots" => panel.fg(rgb(self.muted)),
            "dots.on" => panel.fg(rgb(hue)),
            "chip" => s.fg(rgb(self.bg)).bg(rgb(hue)),
            _ => panel,
        })
    }

    fn colour(c: Option<Color>, default: Rgb) -> Option<Rgb> {
        match c {
            None | Some(Color::Reset) => Some(default),
            Some(Color::Rgb(r, g, b)) => Some((r, g, b)),
            _ => None,
        }
    }

    /// A dimmed cell: its foreground 60% of the way to its background in truecolour, else faint.
    pub fn dim(&self, s: Style) -> Style {
        match (Self::colour(s.fg, self.fg), Self::colour(s.bg, self.bg)) {
            (Some(f), Some(b)) if !self.plain => s.fg(rgb(blend(f, b, 0.6))),
            _ => s.add_modifier(Modifier::DIM),
        }
    }

    /// A ringed cell: its background 20% of the way to the border colour, else underlined.
    pub fn ring(&self, s: Style) -> Style {
        match Self::colour(s.bg, self.bg) {
            Some(b) if !self.plain => s.bg(rgb(blend(b, self.border, 0.2))),
            _ => s.add_modifier(Modifier::UNDERLINED),
        }
    }
}

/// A buffer with a theme: what [`compose`](crate::compose) draws into.
pub struct Themed<'a> {
    pub buf: &'a mut Buffer,
    pub theme: Theme,
}

impl<'a> Themed<'a> {
    pub fn new(buf: &'a mut Buffer, theme: Theme) -> Themed<'a> {
        Themed { buf, theme }
    }
}

fn pos(buf: &Buffer, x: u16, y: u16) -> (u16, u16) {
    (buf.area.x + x, buf.area.y + y)
}

fn set(buf: &mut Buffer, theme: &Theme, x: u16, y: u16, symbol: &str, role: &str) {
    let p = pos(buf, x, y);
    let style = theme.style(role).unwrap_or_default();
    let c = &mut buf[p];
    c.reset();
    // The second half of a wide grapheme: ratatui skips it when the first is wide.
    c.set_symbol(if symbol.is_empty() { " " } else { symbol });
    c.set_style(style);
}

fn flag(buf: &mut Buffer, theme: &Theme, x: u16, y: u16, flags: Flags) {
    let p = pos(buf, x, y);
    let c = &mut buf[p];
    let mut s = c.style();
    if flags.contains(Flags::DIM) {
        s = theme.dim(s);
    }
    if flags.contains(Flags::RING) {
        s = theme.ring(s);
    }
    c.set_style(s);
}

impl CellGrid for Themed<'_> {
    fn size(&self) -> (u16, u16) {
        (self.buf.area.width, self.buf.area.height)
    }

    fn symbol(&self, x: u16, y: u16) -> &str {
        self.buf[pos(self.buf, x, y)].symbol()
    }

    fn set(&mut self, x: u16, y: u16, symbol: &str, role: &str) {
        set(self.buf, &self.theme, x, y, symbol, role);
    }

    fn blank(&mut self, x: u16, y: u16) {
        let p = pos(self.buf, x, y);
        self.buf[p].set_symbol(" ");
    }

    fn flag(&mut self, x: u16, y: u16, flags: Flags) {
        flag(self.buf, &self.theme, x, y, flags);
    }
}

/// A buffer with the dark theme.
impl CellGrid for Buffer {
    fn size(&self) -> (u16, u16) {
        (self.area.width, self.area.height)
    }

    fn symbol(&self, x: u16, y: u16) -> &str {
        self[pos(self, x, y)].symbol()
    }

    fn set(&mut self, x: u16, y: u16, symbol: &str, role: &str) {
        set(self, &Theme::dark(), x, y, symbol, role);
    }

    fn blank(&mut self, x: u16, y: u16) {
        let p = pos(self, x, y);
        self[p].set_symbol(" ");
    }

    fn flag(&mut self, x: u16, y: u16, flags: Flags) {
        flag(self, &Theme::dark(), x, y, flags);
    }
}
