//! The cell renderer: draws a [`Scene`] onto any grid of cells.

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::layout::{PanelKind, Scene};
use crate::text;

/// Per-cell marks that restyle what's there without changing it: the host's theme decides
/// what they look like (a dimmed cell keeps its symbol and its role).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Flags(pub u8);

impl Flags {
    pub const NONE: Flags = Flags(0);
    /// Outside a spotlight's holes.
    pub const DIM: Flags = Flags(1);
    /// Part of a ringed anchor.
    pub const RING: Flags = Flags(2);

    pub const fn contains(self, f: Flags) -> bool {
        self.0 & f.0 == f.0 && f.0 != 0
    }
}

impl std::ops::BitOr for Flags {
    type Output = Flags;
    fn bitor(self, o: Flags) -> Flags {
        Flags(self.0 | o.0)
    }
}

impl std::ops::BitOrAssign for Flags {
    fn bitor_assign(&mut self, o: Flags) {
        self.0 |= o.0;
    }
}

/// A grid of cells the overlay draws on: a host's screen buffer, caretline's frame, a test grid.
///
/// The second cell of a wide grapheme holds `""` or a filler; [`compose`] finds wide graphemes
/// by the first cell's width and never splits one.
pub trait CellGrid {
    fn size(&self) -> (u16, u16);
    /// The grapheme at a cell.
    fn symbol(&self, x: u16, y: u16) -> &str;
    /// Replaces a cell: its grapheme and role, its flags cleared. `""` is the second half of
    /// a wide grapheme written just before.
    fn set(&mut self, x: u16, y: u16, symbol: &str, role: &str);
    /// Replaces a cell's grapheme with a space, keeping its role and flags.
    fn blank(&mut self, x: u16, y: u16);
    /// Adds flags to a cell.
    fn flag(&mut self, x: u16, y: u16, flags: Flags);
}

/// Writes one grapheme without splitting a wide one: overwriting either half of a wide
/// grapheme blanks the other half; a wide grapheme that doesn't fit is drawn as a space.
/// Returns the width written.
pub fn put(grid: &mut (impl CellGrid + ?Sized), x: u16, y: u16, g: &str, role: &str) -> u16 {
    let (w, h) = grid.size();
    if x >= w || y >= h {
        return 0;
    }
    let mut g = text::printable(g);
    let mut gw = text::width(g) as u16;
    if x + gw > w {
        g = " ";
        gw = 1;
    }
    // The left neighbour is a wide grapheme whose second half we're about to cover.
    if x > 0 && text::width(grid.symbol(x - 1, y)) >= 2 {
        grid.blank(x - 1, y);
    }
    // A wide grapheme starting under our last cell loses its second half.
    let last = x + gw - 1;
    if text::width(grid.symbol(last, y)) >= 2 && last + 1 < w {
        grid.blank(last + 1, y);
    }
    grid.set(x, y, g, role);
    if gw == 2 {
        grid.set(x + 1, y, "", role);
    }
    gw
}

/// Writes a string from (`x`, `y`), stopping before column `limit`. Returns the column after it.
pub fn put_str(
    grid: &mut (impl CellGrid + ?Sized),
    x: u16,
    y: u16,
    s: &str,
    role: &str,
    limit: u16,
) -> u16 {
    let mut cx = x;
    for g in s.graphemes(true) {
        let gw = text::width(text::printable(g)) as u16;
        if cx + gw > limit {
            break;
        }
        cx += put(grid, cx, y, g, role);
    }
    cx
}

/// Draws a scene: dims (once per cell), then rings, then each layer's arrow and panels in z
/// order, so a later layer's box covers an earlier one's arrow.
pub fn compose(scene: &Scene, grid: &mut (impl CellGrid + ?Sized)) {
    let (w, h) = grid.size();
    if !scene.spots.is_empty() {
        let mut dim = vec![false; w as usize * h as usize];
        for s in &scene.spots {
            for y in s.area.y..s.area.bottom().min(h) {
                for x in s.area.x..s.area.right().min(w) {
                    if !s.holes.iter().any(|r| r.contains(x, y)) {
                        dim[y as usize * w as usize + x as usize] = true;
                    }
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                if dim[y as usize * w as usize + x as usize] {
                    grid.flag(x, y, Flags::DIM);
                }
            }
        }
    }
    for r in &scene.rings {
        for rect in &r.rects {
            for y in rect.y..rect.bottom().min(h) {
                for x in rect.x..rect.right().min(w) {
                    grid.flag(x, y, Flags::RING);
                }
            }
        }
    }
    let [tl, tr, bl, br, hz, vt] = scene.glyphs.frame();
    for l in &scene.layers {
        if let Some(a) = &l.arrow {
            for c in &a.cells {
                put(grid, c.x, c.y, &c.g, &a.role);
            }
        }
        for p in &l.panels {
            let r = p.rect;
            if r.is_empty() {
                continue;
            }
            let right = r.right().min(w);
            for y in r.y..r.bottom().min(h) {
                for x in r.x..right {
                    put(grid, x, y, " ", &p.fill);
                }
            }
            if let (PanelKind::Callout, Some(b)) = (p.kind, &p.border) {
                if r.w >= 2 && r.h >= 2 {
                    let (x1, y1) = (r.right() - 1, r.bottom() - 1);
                    for x in r.x + 1..x1 {
                        put(grid, x, r.y, hz, b);
                        put(grid, x, y1, hz, b);
                    }
                    for y in r.y + 1..y1 {
                        put(grid, r.x, y, vt, b);
                        put(grid, x1, y, vt, b);
                    }
                    put(grid, r.x, r.y, tl, b);
                    put(grid, x1, r.y, tr, b);
                    put(grid, r.x, y1, bl, b);
                    put(grid, x1, y1, br, b);
                    if let Some((jx, jy, g)) = &p.junction {
                        put(grid, *jx, *jy, g, b);
                    }
                }
            }
            // Text stays inside the panel (inside the border for a callout).
            let limit = if p.kind == PanelKind::Callout {
                r.right().saturating_sub(2)
            } else {
                r.right()
            };
            for run in &p.runs {
                let mut x = run.x;
                for s in &run.spans {
                    x = put_str(grid, x, run.y, &s.text, &s.role, limit.min(w));
                }
            }
        }
    }
}

/// A plain grid of cells for tests and goldens: symbols, role names and flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestGrid {
    pub width: u16,
    pub height: u16,
    pub cells: Vec<TestCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCell {
    pub symbol: String,
    pub role: String,
    pub flags: Flags,
}

impl TestGrid {
    /// A blank grid in the `text` role.
    pub fn new(width: u16, height: u16) -> TestGrid {
        TestGrid {
            width,
            height,
            cells: vec![
                TestCell {
                    symbol: " ".into(),
                    role: "text".into(),
                    flags: Flags::NONE
                };
                width as usize * height as usize
            ],
        }
    }

    /// Lines of text, each cut to the width, in the `text` role. Wide graphemes take two cells.
    pub fn from_text(width: u16, height: u16, s: &str) -> TestGrid {
        let mut g = TestGrid::new(width, height);
        for (y, line) in s.lines().take(height as usize).enumerate() {
            put_str(&mut g, 0, y as u16, line, "text", width);
        }
        g
    }

    /// A caretline frame's cells, roles by name (`text`, `selection`, `status`, a host's names).
    #[cfg(feature = "caretline")]
    pub fn from_frame(f: &caretline::view::Frame) -> TestGrid {
        let mut g = TestGrid::new(f.width, f.height);
        for y in 0..f.height {
            for x in 0..f.width {
                let c = f.cell(x, y);
                let i = y as usize * f.width as usize + x as usize;
                g.cells[i] = TestCell {
                    symbol: c.symbol.to_string(),
                    role: f.role_name(c.role).to_string(),
                    flags: Flags::NONE,
                };
            }
        }
        g
    }

    pub fn cell(&self, x: u16, y: u16) -> &TestCell {
        &self.cells[y as usize * self.width as usize + x as usize]
    }

    fn cell_mut(&mut self, x: u16, y: u16) -> &mut TestCell {
        &mut self.cells[y as usize * self.width as usize + x as usize]
    }

    /// The rows as text, trailing spaces trimmed.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for y in 0..self.height {
            let line: String = (0..self.width)
                .map(|x| self.cell(x, y).symbol.as_str())
                .collect();
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// One char per cell saying what's drawn there (design §8.2's role map):
    /// `#` a callout (border, fill, title), `>` an arrow, `k` a key badge, `o` step dots,
    /// `=` a strip, `c` a chip, `@` any part of an agent's callout, `*` a ringed cell, `.` a
    /// dimmed one, `s` the status row, and a blank for the rest.
    pub fn role_map(&self) -> String {
        let mut out = String::new();
        for y in 0..self.height {
            let mut line = String::new();
            for x in 0..self.width {
                let c = self.cell(x, y);
                let r = c.role.as_str();
                let ch = if let Some(rest) = r.strip_prefix("overlay.") {
                    let (agent, rest) = match rest.strip_prefix("agent.") {
                        Some(x) => (true, x),
                        None => (false, rest),
                    };
                    match rest {
                        "arrow" => '>',
                        "key" => 'k',
                        "dots" | "dots.on" => 'o',
                        "strip" => '=',
                        "chip" => 'c',
                        _ if agent => '@',
                        _ => '#',
                    }
                } else if c.flags.contains(Flags::RING) {
                    '*'
                } else if c.flags.contains(Flags::DIM) {
                    '.'
                } else if r == "status" || r == "status_accent" {
                    's'
                } else {
                    ' '
                };
                line.push(ch);
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}

impl CellGrid for TestGrid {
    fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    fn symbol(&self, x: u16, y: u16) -> &str {
        &self.cell(x, y).symbol
    }

    fn set(&mut self, x: u16, y: u16, symbol: &str, role: &str) {
        *self.cell_mut(x, y) = TestCell {
            symbol: symbol.into(),
            role: role.into(),
            flags: Flags::NONE,
        };
    }

    fn blank(&mut self, x: u16, y: u16) {
        self.cell_mut(x, y).symbol = " ".into();
    }

    fn flag(&mut self, x: u16, y: u16, flags: Flags) {
        self.cell_mut(x, y).flags |= flags;
    }
}
