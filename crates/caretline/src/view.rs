//! `view`: a pure function from the state to a grid of cells, plus text and ANSI renderers
//! for snapshots. A terminal front end copies the grid to the screen.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::helix::graphemes::{grapheme_width, Grapheme};
use crate::helix::Tendril;
use crate::layout::{Layout, RowPos};
use crate::marks::MarkId;
use crate::outline::Hang;
use crate::state::{Document, State, View};
use unicode_segmentation::UnicodeSegmentation;

/// What a cell shows, by meaning. Front ends pick the colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Text,
    Selection,
    Status,
    /// The dirty marker and other emphasis in the status bar.
    StatusAccent,
    /// A block's hang in an outline layout: the columns before its content, where its
    /// marker's glyph goes. Blank unless a decoration or the layout's plain glyphs fill it.
    Hang,
    /// A role a host named in a [`Decoration`](crate::host::Decoration): its name is
    /// [`Frame::role_name`].
    Named(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// The grapheme drawn here. Empty for the second cell of a wide grapheme. Stored inline
    /// (no allocation) for anything up to 23 bytes.
    pub symbol: Tendril,
    pub role: Role,
    /// The document char drawn here, if any (both cells of a wide grapheme, every cell of a
    /// tab). A host styles spans of text by it.
    pub char_idx: Option<u32>,
    /// Marks a frame pass set for the renderer ([`Frame::flag`]): none from the engine.
    pub flags: CellFlags,
}

/// Generic marks on a cell, named for what they ask of a renderer: draw it dimmed, or ringed
/// (emphasised). The engine never sets or reads them; a host's frame pass sets them and its
/// renderer decides what they look like (`to_ansi` uses faint and underline). One byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CellFlags(u8);

impl CellFlags {
    pub const NONE: CellFlags = CellFlags(0);
    pub const DIM: CellFlags = CellFlags(1);
    pub const RING: CellFlags = CellFlags(2);

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every flag in `other` is set.
    pub fn contains(self, other: CellFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// The flags' names, space-separated (`"dim"`, `"ring"`, `"dim ring"`; empty for none): the
    /// `cells` format's spelling.
    pub fn names(self) -> &'static str {
        match self.0 & 3 {
            1 => "dim",
            2 => "ring",
            3 => "dim ring",
            _ => "",
        }
    }
}

impl std::ops::BitOr for CellFlags {
    type Output = CellFlags;
    fn bitor(self, other: CellFlags) -> CellFlags {
        CellFlags(self.0 | other.0)
    }
}

impl std::ops::BitOrAssign for CellFlags {
    fn bitor_assign(&mut self, other: CellFlags) {
        self.0 |= other.0;
    }
}

/// A clickable rect a frame pass put on the frame, with the host's id for it
/// ([`Frame::region_at`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub id: String,
}

impl Region {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x
            && y >= self.y
            && (x as u32) < self.x as u32 + self.w as u32
            && (y as u32) < self.y as u32 + self.h as u32
    }
}

/// What a frame row shows, for a host that draws around the engine's cells (a hang, marks,
/// fields beside a block).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RowInfo {
    /// A row of text: visual row `row` of document line `line`, in block `block` (outline
    /// documents). `first` and `last` say whether it is the block's first or last row,
    /// `chars` the chars it shows (its line break excluded), `x` the column its text starts.
    Text {
        block: Option<MarkId>,
        line: usize,
        row: u16,
        first: bool,
        last: bool,
        chars: Range<usize>,
        x: u16,
    },
    /// The blank row before a block.
    Gap { before: MarkId },
    /// A host's row after a block (`index` from 0).
    Extra { block: MarkId, index: u16 },
    /// Below the end of the document.
    Past,
    /// The status bar.
    Status,
}

/// Where a screen cell is, by meaning: what a click there means ([`hit`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Hit {
    /// Text: the char position a click there puts the caret at.
    Text { pos: usize },
    /// A block's hang (the columns before its content), with the id of the decoration drawn
    /// there, if it has one.
    Hang {
        block: MarkId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deco: Option<String>,
    },
    /// A block's gutter, left of everything, with its decoration's id.
    Gutter {
        block: MarkId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deco: Option<String>,
    },
    /// The blank row before a block.
    Gap { block: MarkId },
    /// A host's row after a block.
    Extra { block: MarkId, index: u16 },
    /// Below the document, or the status bar.
    Past,
}

/// A rendered screen: `height` rows of `width` cells, and where the caret is (if visible).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u16,
    pub height: u16,
    pub cells: Vec<Cell>,
    pub cursor: Option<(u16, u16)>,
    /// One per row: what it shows.
    pub rows: Vec<RowInfo>,
    /// The names of the [`Role::Named`] roles, by index.
    pub roles: Vec<String>,
    /// The view's cell size in device pixels, when known ([`crate::View::cell_px`]): for a host
    /// that draws pixels over these cells.
    pub cell_px: Option<crate::state::CellPx>,
    /// Clickable rects frame passes added, in the order they were added ([`Frame::region_at`]).
    pub regions: Vec<Region>,
}

impl Frame {
    /// A blank frame: `height` rows of `width` spaces in the text role.
    pub fn new(width: u16, height: u16) -> Frame {
        Frame {
            width,
            height,
            cells: vec![
                Cell {
                    symbol: " ".into(),
                    role: Role::Text,
                    char_idx: None,
                    flags: CellFlags::NONE,
                };
                width as usize * height as usize
            ],
            cursor: None,
            rows: Vec::with_capacity(height as usize),
            roles: Vec::new(),
            cell_px: None,
            regions: Vec::new(),
        }
    }

    /// The name of a role: a built-in role's wire name, or the name a host gave it.
    pub fn role_name(&self, role: Role) -> &str {
        match role {
            Role::Named(i) => self.roles.get(i as usize).map_or("host", String::as_str),
            r => crate::protocol::role_name(r),
        }
    }

    /// The role for a style name a host defines: `hang` is the built-in [`Role::Hang`], any
    /// other name a [`Role::Named`] ([`Frame::role_name`] gives it back). For decorations and
    /// for what a frame pass draws.
    pub fn role(&mut self, name: &str) -> Role {
        if name == "hang" {
            return Role::Hang;
        }
        let i = match self.roles.iter().position(|r| r == name) {
            Some(i) => i,
            None => {
                self.roles.push(name.to_string());
                self.roles.len() - 1
            }
        };
        Role::Named(i.min(u16::MAX as usize) as u16)
    }

    pub fn cell(&self, x: u16, y: u16) -> &Cell {
        &self.cells[y as usize * self.width as usize + x as usize]
    }

    /// Writes one grapheme of display width `w` at (`x`, `y`). Clipped at the right edge:
    /// a wide grapheme that doesn't fit is drawn as spaces.
    fn put(&mut self, x: usize, y: usize, symbol: &str, w: usize, role: Role) {
        self.put_at(x, y, symbol, w, role, None, self.width as usize);
    }

    /// [`Frame::put`] for a document char, clipped at `limit`.
    #[allow(clippy::too_many_arguments)]
    fn put_at(
        &mut self,
        x: usize,
        y: usize,
        symbol: &str,
        w: usize,
        role: Role,
        char_idx: Option<u32>,
        limit: usize,
    ) {
        // The row stride is the frame's width; `limit` only clips where this grapheme may go.
        let row = y * self.width as usize;
        let width = (self.width as usize).min(limit);
        if y >= self.height as usize || x >= width {
            return;
        }
        if w == 0 {
            return;
        }
        if x + w > width {
            for cx in x..width {
                self.cells[row + cx] = Cell {
                    symbol: " ".into(),
                    role,
                    char_idx,
                    flags: CellFlags::NONE,
                };
            }
            return;
        }
        self.cells[row + x] = Cell {
            symbol: Tendril::from(symbol),
            role,
            char_idx,
            flags: CellFlags::NONE,
        };
        for cx in x + 1..x + w {
            self.cells[row + cx] = Cell {
                symbol: Tendril::new(),
                role,
                char_idx,
                flags: CellFlags::NONE,
            };
        }
    }

    /// Writes a string from column `x`, stopping before `limit`. Returns the column after it.
    fn put_str(&mut self, x: usize, y: usize, s: &str, limit: usize, role: Role) -> usize {
        let mut cx = x;
        for g in s.graphemes(true) {
            let w = display_width(g);
            if cx + w > limit {
                break;
            }
            self.put(cx, y, printable(g), w, role);
            cx += w;
        }
        cx
    }

    /// The columns `[from, to)` of row `y` that the grapheme covering (`x`, `y`) takes: one
    /// cell, or a wide grapheme's lead and the empty cells after it.
    fn span(&self, x: u16, y: u16) -> (usize, usize) {
        let row = y as usize * self.width as usize;
        let mut from = x as usize;
        while from > 0 && self.cells[row + from].symbol.is_empty() {
            from -= 1;
        }
        let mut to = from + 1;
        while to < self.width as usize && self.cells[row + to].symbol.is_empty() {
            to += 1;
        }
        (from, to.max(x as usize + 1))
    }

    /// Writes one grapheme at (`x`, `y`) in `role`, for a frame pass, and returns the cells it
    /// took (0 off the frame or for an empty string). It never splits a wide grapheme:
    /// writing over either half of one blanks its other half, and a wide grapheme that
    /// doesn't fit before the right edge is drawn as a space. A control or zero-width cluster
    /// draws as a placeholder, as text does. The cell keeps no document char and no flags.
    pub fn set(&mut self, x: u16, y: u16, grapheme: &str, role: Role) -> u16 {
        if x >= self.width || y >= self.height || grapheme.is_empty() {
            return 0;
        }
        let g = printable(grapheme);
        let mut w = display_width(g).max(1);
        let symbol = if x as usize + w > self.width as usize {
            w = 1;
            " "
        } else {
            g
        };
        let row = y as usize * self.width as usize;
        for cx in x as usize..x as usize + w {
            let (from, to) = self.span(cx as u16, y);
            if to - from > 1 {
                for c in &mut self.cells[row + from..row + to] {
                    c.symbol = " ".into();
                }
            }
        }
        self.put_at(
            x as usize,
            y as usize,
            symbol,
            w,
            role,
            None,
            self.width as usize,
        );
        w as u16
    }

    /// Sets the role of the grapheme at (`x`, `y`) (both cells of a wide one).
    pub fn restyle(&mut self, x: u16, y: u16, role: Role) {
        if x < self.width && y < self.height {
            let row = y as usize * self.width as usize;
            let (from, to) = self.span(x, y);
            for c in &mut self.cells[row + from..row + to] {
                c.role = role;
            }
        }
    }

    /// Adds `flags` to the grapheme at (`x`, `y`) (both cells of a wide one). Flags don't
    /// compound: a cell dimmed twice is dimmed once.
    pub fn flag(&mut self, x: u16, y: u16, flags: CellFlags) {
        if x < self.width && y < self.height {
            let row = y as usize * self.width as usize;
            let (from, to) = self.span(x, y);
            for c in &mut self.cells[row + from..row + to] {
                c.flags |= flags;
            }
        }
    }

    /// The last region added that holds cell (`x`, `y`): the topmost, as passes draw in
    /// order.
    pub fn region_at(&self, x: u16, y: u16) -> Option<&Region> {
        self.regions.iter().rev().find(|r| r.contains(x, y))
    }

    /// The rows as plain text, trailing spaces trimmed.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for y in 0..self.height {
            let mut line = String::new();
            for x in 0..self.width {
                line.push_str(&self.cell(x, y).symbol);
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    /// The rows with ANSI styling: the selection in reverse video, the status bar
    /// highlighted, the caret as an underlined reverse cell (a snapshot has no
    /// terminal cursor), dimmed cells faint and ringed ones underlined.
    pub fn to_ansi(&self) -> String {
        let mut out = String::new();
        for y in 0..self.height {
            let mut current: Option<(Role, bool, CellFlags)> = None;
            for x in 0..self.width {
                let cell = self.cell(x, y);
                let is_cursor = self.cursor == Some((x, y));
                let key = (cell.role, is_cursor, cell.flags);
                if current != Some(key) {
                    out.push_str("\x1b[0m");
                    out.push_str(sgr(cell.role, is_cursor));
                    if cell.flags.contains(CellFlags::DIM) {
                        out.push_str("\x1b[2m");
                    }
                    if cell.flags.contains(CellFlags::RING) {
                        out.push_str("\x1b[4m");
                    }
                    current = Some(key);
                }
                out.push_str(&cell.symbol);
            }
            out.push_str("\x1b[0m\n");
        }
        out
    }
}

fn sgr(role: Role, cursor: bool) -> &'static str {
    match (role, cursor) {
        (_, true) => "\x1b[7;4m",
        (Role::Text, _) => "",
        (Role::Selection, _) => "\x1b[30;46m",
        (Role::Status, _) => "\x1b[30;47m",
        (Role::StatusAccent, _) => "\x1b[1;30;47m",
        (Role::Hang, _) => "\x1b[2m",
        (Role::Named(_), _) => "",
    }
}

/// A grapheme's display width, matching the formatter's rule (at least one cell).
pub fn display_width(g: &str) -> usize {
    if g.is_empty() {
        0
    } else {
        grapheme_width(g)
    }
}

/// Control characters would drive the terminal, and zero-width clusters would take no
/// cell where the layout gives them one; both draw as a placeholder.
fn printable(g: &str) -> &str {
    use unicode_width::UnicodeWidthStr;
    if g.chars().any(|c| c.is_control()) || UnicodeWidthStr::width(g) == 0 {
        "\u{FFFD}"
    } else {
        g
    }
}

/// Renders the state. Pure: the same state always gives the same frame.
pub fn view(state: &State) -> Frame {
    render(&state.doc, &state.view)
}

/// Renders one view of a document, then runs the host's frame passes over it in the order
/// they were registered ([`crate::Host::frame_pass`]). Pure.
pub fn render(doc: &Document, view: &View) -> Frame {
    render_skipping(doc, view, &[])
}

/// [`render`] without the frame passes named in `skip` (a runtime that draws one of them
/// another way, in pixels say).
pub fn render_skipping(doc: &Document, view: &View, skip: &[&str]) -> Frame {
    let mut frame = render_plain(doc, view);
    crate::host::run_frame_passes(doc, view, &mut frame, skip);
    frame
}

/// Renders one view of a document without any frame pass: the text, the decorations and the
/// status bar only. Pure.
pub fn render_plain(doc: &Document, view: &View) -> Frame {
    let width = view.viewport.width.max(1);
    let height = view.viewport.height.max(1);
    let mut frame = Frame::new(width, height);
    frame.cell_px = view.cell_px;
    let text_rows = view.text_rows().min(height as usize);
    let layout = Layout::of(doc, view);
    let top = layout.top(&view.scroll);
    let hscroll = if layout.wraps() { 0 } else { view.scroll.col };
    let caret = view.focused.then(|| view.caret());
    let ranges: Vec<(usize, usize)> = view
        .selection
        .iter()
        .filter(|r| !r.is_empty())
        .map(|r| (r.from(), r.to()))
        .collect();
    let selected = |pos: usize| ranges.iter().any(|&(f, t)| f <= pos && pos < t);
    let outline = layout.outline().cloned();
    let geometry = layout.geometry().cloned();

    let mut y = 0usize;
    let mut line = Some(top.line);
    let mut first_row = top.row;
    while let Some(l) = line.filter(|_| y < text_rows) {
        let lf = layout.line_format(l);
        let block = outline.as_ref().map(|o| o.block_of_line(l));
        let id = block.map(|b| b.id);
        let text_n = layout.text_rows_of(l);
        let mut r = first_row;
        // The blank row before a block.
        while r < lf.before && y < text_rows {
            frame.rows.push(RowInfo::Gap {
                before: id.unwrap_or(MarkId(u64::MAX)),
            });
            y += 1;
            r += 1;
        }
        // Its text.
        let tr0 = r.saturating_sub(lf.before);
        if y < text_rows && tr0 < text_n {
            let shown = (text_n - tr0).min(text_rows - y);
            let y0 = y;
            let x = lf.x.min(u16::MAX as usize) as u16;
            for k in 0..shown {
                let row = tr0 + k;
                let first = block.is_some_and(|b| b.first_line == l) && row == 0;
                let last = block.is_some_and(|b| b.last_line() == l) && row + 1 == text_n;
                frame.rows.push(RowInfo::Text {
                    block: id,
                    line: l,
                    row: row as u16,
                    first,
                    last,
                    chars: 0..0,
                    x,
                });
                if let (true, Some(g), Some(b)) = (first, &geometry, block) {
                    let from =
                        lf.x.saturating_sub(g.hang as usize)
                            .max(g.gutter as usize)
                            .min(lf.x);
                    for cx in from..lf.x.min(width as usize) {
                        frame.cells[(y0 + k) * width as usize + cx].role = Role::Hang;
                    }
                    let deco = decoration(doc, view, g, b);
                    if let Some(d) = &deco.hang {
                        let role = frame.role(&d.role);
                        frame.put_str(from, y0 + k, &d.text, lf.x.min(width as usize), role);
                    }
                    if let Some(d) = &deco.gutter {
                        let role = frame.role(&d.role);
                        frame.put_str(
                            0,
                            y0 + k,
                            &d.text,
                            (g.gutter as usize).min(from).min(width as usize),
                            role,
                        );
                    }
                }
            }
            let offset = layout.line_offset(l);
            let limit = match &geometry {
                Some(_) => lf.x + layout.line_text_format(l).viewport_width as usize,
                None => width as usize,
            };
            let hangs = layout.hangs(l);
            let mut seen = vec![false; shown];
            let formatter = layout.formatter_at_row(RowPos {
                line: l,
                row: lf.before + tr0,
            });
            for g in formatter {
                if g.line_idx != l {
                    break;
                }
                let vr = g.visual_pos.row;
                if vr < tr0 {
                    continue;
                }
                if vr >= tr0 + shown {
                    break;
                }
                let sy = y0 + vr - tr0;
                if let RowInfo::Text { chars, .. } = &mut frame.rows[sy] {
                    if !seen[sy - y0] {
                        seen[sy - y0] = true;
                        *chars = g.char_idx..g.char_idx;
                    }
                    if !g.is_virtual() && !g.source.is_eof() && g.raw != Grapheme::Newline {
                        chars.end = g.char_idx + g.doc_chars();
                    }
                }
                if g.is_virtual() {
                    continue;
                }
                let col = (lf.x + g.visual_pos.col) as isize - hscroll as isize - offset as isize;
                let w = g.width();
                let role = if selected(g.char_idx) {
                    Role::Selection
                } else {
                    Role::Text
                };
                // The caret may sit one past the column (after a word that fills the row, on
                // the space that hangs there), never past the frame. On a prose row with a
                // hang it may sit after whitespace hanging further: it's drawn there, clamped
                // to the frame's last column on the same row (`Layout::hangs`).
                if Some(g.char_idx) == caret && col >= 0 {
                    let c = col as usize;
                    if hangs && c >= lf.x {
                        frame.cursor = Some((c.min(width as usize - 1) as u16, sy as u16));
                    } else if c <= limit && c < width as usize {
                        frame.cursor = Some((c as u16, sy as u16));
                    }
                }
                if col < lf.x as isize {
                    continue;
                }
                let cx = col as usize;
                let idx = Some(g.char_idx as u32);
                match &g.raw {
                    Grapheme::Newline => {
                        if role == Role::Selection {
                            frame.put_at(cx, sy, " ", 1, role, idx, limit);
                        }
                    }
                    Grapheme::Tab { width: tw } => {
                        for i in 0..*tw {
                            frame.put_at(cx + i, sy, " ", 1, role, idx, limit);
                        }
                    }
                    Grapheme::Other { g: s } => {
                        if g.source.is_eof() {
                            continue;
                        }
                        frame.put_at(cx, sy, printable(s), w, role, idx, limit);
                    }
                }
            }
            y += shown;
            r = lf.before + tr0 + shown;
        }
        // A host's rows after it.
        let mut k = r.saturating_sub(lf.before + text_n);
        while k < lf.after && y < text_rows {
            frame.rows.push(RowInfo::Extra {
                block: id.unwrap_or(MarkId(u64::MAX)),
                index: k as u16,
            });
            y += 1;
            k += 1;
        }
        line = (l < layout.last_line())
            .then(|| layout.visible_at_or_after(l + 1))
            .flatten();
        first_row = 0;
    }
    while frame.rows.len() < text_rows {
        frame.rows.push(RowInfo::Past);
    }
    while frame.rows.len() < height as usize {
        frame.rows.push(RowInfo::Status);
    }

    if view.config.status_bar {
        draw_status(doc, view, &mut frame, height as usize - 1);
    }
    frame
}

/// What is drawn beside block `b`: the host's decoration, else (with `hang_glyphs`) the plain
/// Markdown glyph of its marker, else nothing.
pub fn decoration(
    doc: &Document,
    view: &View,
    g: &crate::layout::OutlineLayout,
    b: &crate::outline::BlockInfo,
) -> crate::host::Decoration {
    use crate::host::{Ctx, Deco, Decoration};
    if let Some(d) = doc.host().decorate(&Ctx::new(doc, view), b) {
        return d;
    }
    if g.hang_glyphs {
        let text = match b.tag {
            Some(c) => format!("[{c}]"),
            None => hang_glyph(b.hang),
        };
        if !text.is_empty() {
            return Decoration {
                hang: Some(Deco {
                    text,
                    role: "hang".into(),
                    id: None,
                }),
                gutter: None,
            };
        }
    }
    Decoration::default()
}

/// A plain glyph for a hang, for a host that draws none.
fn hang_glyph(hang: Hang) -> String {
    match hang {
        Hang::None | Hang::Fence => String::new(),
        Hang::Bullet => "•".into(),
        Hang::Number(n) => format!("{n}."),
        Hang::Heading(n) => "#".repeat(n as usize),
        Hang::Quote => "│".into(),
    }
}

/// What screen cell (`col`, `row`) of `view` is: text (and where a click there lands), a
/// block's hang or marks column, a blank row, a host's row, or nothing.
pub fn hit(doc: &Document, view: &View, col: u16, row: u16) -> Hit {
    if row as usize >= view.text_rows() {
        return Hit::Past;
    }
    let layout = Layout::of(doc, view);
    let top = layout.top(&view.scroll);
    let (at, moved) = layout.step_rows(top, row as isize);
    if moved < row as isize {
        return Hit::Past;
    }
    let lf = layout.line_format(at.line);
    let block = layout.outline().map(|o| o.block_of_line(at.line).clone());
    if let Some(b) = &block {
        if at.row < lf.before {
            return Hit::Gap { block: b.id };
        }
        let text_n = layout.text_rows_of(at.line);
        if at.row >= lf.before + text_n {
            return Hit::Extra {
                block: b.id,
                index: (at.row - lf.before - text_n) as u16,
            };
        }
        if let Some(g) = layout.geometry() {
            let c = col as usize;
            if c < lf.x {
                // Only a block's first row carries its decoration.
                let deco = (at.row == lf.before && b.first_line == at.line)
                    .then(|| decoration(doc, view, g, b))
                    .unwrap_or_default();
                if c < g.gutter as usize {
                    return Hit::Gutter {
                        block: b.id,
                        deco: deco.gutter.and_then(|d| d.id),
                    };
                }
                return Hit::Hang {
                    block: b.id,
                    deco: deco.hang.and_then(|d| d.id),
                };
            }
        }
    }
    let c = col as usize + if layout.wraps() { 0 } else { view.scroll.col };
    Hit::Text {
        pos: layout.click_at(at, c),
    }
}

/// Where char position `pos` is in `view`: the cell its grapheme starts at, or which way it
/// lies when it isn't on screen ([`locate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Locate {
    /// On screen, at this cell of the text area: where [`hit`] there finds `pos` again.
    At { x: u16, y: u16 },
    /// Scrolled out of view above.
    Above,
    /// Scrolled out of view below.
    Below,
    /// On a visible row, scrolled out of view to the left (a line that doesn't wrap).
    Left,
    /// On a visible row, past the right edge (a line that doesn't wrap).
    Right,
    /// Hidden in a folded block: `block` is the outermost fold that hides it.
    Folded { block: MarkId },
}

/// Where char position `pos` (clamped to the text) is in `view`: the inverse of [`hit`],
/// through the same layout. For a position on screen, `hit` at the cell it returns gives the
/// position back (a position inside a block's marker is placed at its content's start, as a
/// caret there would be). Off screen, it says which way: above or below the rows shown, left
/// or right of a row that doesn't wrap, or inside a folded block. Pure; it costs what the
/// caret's own placement does.
pub fn locate(doc: &Document, view: &View, pos: usize) -> Locate {
    let layout = Layout::of(doc, view);
    let text = layout.text();
    let pos = pos.min(text.len_chars());
    let line = text.char_to_line(pos);
    if layout.is_hidden(line) {
        if let (Some(o), Some(owner)) = (layout.outline(), layout.visible_at_or_before(line)) {
            return Locate::Folded {
                block: o.block_of_line(owner).id,
            };
        }
    }
    let (at, col) = layout.pos_coords(pos);
    let top = layout.top(&view.scroll);
    if at < top {
        return Locate::Above;
    }
    let rows = view.text_rows().min(view.viewport.height as usize);
    let y = layout.rows_between(top, at, rows);
    if y < 0 {
        return Locate::Above;
    }
    if y as usize >= rows {
        return Locate::Below;
    }
    let lf = layout.line_format(at.line);
    let hscroll = if layout.wraps() { 0 } else { view.scroll.col };
    let x = col as isize - hscroll as isize - layout.line_offset(at.line) as isize;
    if x < lf.x as isize {
        return Locate::Left;
    }
    let width = view.viewport.width.max(1) as usize;
    let (x, limit) = if layout.hangs(at.line) {
        // After whitespace hanging past the column, as the caret is drawn: there, clamped
        // to the frame's last column (`Layout::hangs`).
        (x.min(width as isize - 1), width)
    } else {
        let limit = match layout.geometry() {
            // A block's content stops at its column; the caret may sit one past it.
            Some(_) => lf.x + layout.line_text_format(at.line).viewport_width as usize + 1,
            None => width,
        };
        (x, limit)
    };
    if x as usize >= width.min(limit) {
        return Locate::Right;
    }
    Locate::At {
        x: x as u16,
        y: y as u16,
    }
}

/// The status bar: the file name and dirty marker on the left, the message in the middle,
/// and `line:col` (plus the selection size) on the right.
fn draw_status(doc: &Document, view: &View, frame: &mut Frame, y: usize) {
    let width = frame.width as usize;
    for x in 0..width {
        frame.put(x, y, " ", 1, Role::Status);
    }
    let text = doc.text.slice(..);
    let caret = view.caret();
    let line = text.char_to_line(caret);
    let line_start = text.line_to_char(line);
    let prefix = text.slice(line_start..caret);
    // Within a line, ASCII text has one grapheme per char (CRLF only ends a line), so a
    // long ASCII line needs no segmentation.
    let col = if prefix.len_bytes() == prefix.len_chars() {
        prefix.len_chars()
    } else {
        prefix.to_string().graphemes(true).count()
    };
    let selected: usize = view.selection.iter().map(|r| r.len()).sum();
    let mut right = format!("{}:{} ", line + 1, col + 1);
    if selected > 0 {
        right = format!("{selected} sel  {right}");
    }
    let right_w: usize = right.graphemes(true).map(display_width).sum();
    let right_x = width.saturating_sub(right_w);
    frame.put_str(right_x, y, &right, width, Role::Status);

    let mut x = frame.put_str(1, y, &doc.name(), right_x.saturating_sub(1), Role::Status);
    if doc.dirty {
        x = frame.put_str(x, y, " [+]", right_x.saturating_sub(1), Role::StatusAccent);
    }
    if let Some(msg) = &view.status {
        frame.put_str(x + 2, y, msg, right_x.saturating_sub(1), Role::Status);
    }
}
