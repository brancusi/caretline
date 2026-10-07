//! Layout: layers and resolved anchors in, a [`Scene`] out. Pure, and all geometry is in cell
//! coordinates, so a cell renderer ([`compose`](crate::compose)) and a later pixel renderer
//! draw the same thing.

use serde::{Deserialize, Serialize};

use crate::compose::CellGrid;
use crate::geom::{Rect, Side};
use crate::model::{Anchor, Item, Layer, Layers, Part, Pulse, ScreenPos};
use crate::resolve::{Off, Resolve, Resolved};
use crate::route::{self, Field};
use crate::text::{self, KeyLabels, NoKeys, Piece, Tok};

/// The glyph set. `Ascii` is for terminals that draw box drawing or ambiguous-width symbols
/// wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Glyphs {
    #[default]
    Rounded,
    Square,
    Ascii,
}

impl Glyphs {
    /// Corners and edges: top-left, top-right, bottom-left, bottom-right, horizontal, vertical.
    pub fn frame(self) -> [&'static str; 6] {
        match self {
            Glyphs::Rounded => ["╭", "╮", "╰", "╯", "─", "│"],
            Glyphs::Square => ["┌", "┐", "└", "┘", "─", "│"],
            Glyphs::Ascii => ["+", "+", "+", "+", "-", "|"],
        }
    }

    fn dots(self) -> (&'static str, &'static str) {
        match self {
            Glyphs::Ascii => ("*", "-"),
            _ => ("●", "○"),
        }
    }

    fn pointer(self, off: &Off) -> &'static str {
        match (off, self == Glyphs::Ascii) {
            (Off::Above { .. }, false) => "↑",
            (Off::Below { .. }, false) => "↓",
            (Off::Left { .. }, false) => "◀",
            (Off::Right { .. }, false) => "▶",
            (Off::Above { .. }, true) => "^",
            (Off::Below { .. }, true) => "v",
            (Off::Left { .. }, true) => "<",
            (Off::Right { .. }, true) => ">",
        }
    }

    fn sep(self) -> &'static str {
        match self {
            Glyphs::Ascii => " - ",
            _ => " · ",
        }
    }
}

/// What a cell of the host's screen holds, for placement and routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Blank,
    Text,
    /// Either half of a wide grapheme: covered like text, never crossed by an arrow.
    Wide,
}

/// The host's screen as layout sees it: its size, where overlays may go (`area`: the text
/// rows, never the status row), the caret, cells no callout may cover (a selection, a
/// prompt), and which cells hold text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub width: u16,
    pub height: u16,
    pub area: Rect,
    pub caret: Option<(u16, u16)>,
    pub protect: Vec<Rect>,
    kinds: Vec<CellKind>,
}

impl Grid {
    /// A blank screen; overlays may use all of it.
    pub fn new(width: u16, height: u16) -> Grid {
        Grid {
            width,
            height,
            area: Rect::new(0, 0, width, height),
            caret: None,
            protect: Vec::new(),
            kinds: vec![CellKind::Blank; width as usize * height as usize],
        }
    }

    /// The screen as drawn so far: which cells hold text, read from the grid's symbols.
    pub fn scan(cells: &impl CellGrid) -> Grid {
        let (w, h) = cells.size();
        let mut g = Grid::new(w, h);
        for y in 0..h {
            let mut x = 0;
            while x < w {
                let s = cells.symbol(x, y);
                if text::width(s) >= 2 {
                    g.set_kind(x, y, CellKind::Wide);
                    if x + 1 < w {
                        g.set_kind(x + 1, y, CellKind::Wide);
                    }
                    x += 2;
                    continue;
                }
                if !s.trim().is_empty() {
                    g.set_kind(x, y, CellKind::Text);
                }
                x += 1;
            }
        }
        g
    }

    pub fn with_area(mut self, area: Rect) -> Grid {
        self.area = area.intersection(&Rect::new(0, 0, self.width, self.height));
        self
    }

    pub fn with_caret(mut self, caret: Option<(u16, u16)>) -> Grid {
        self.caret = caret;
        self
    }

    pub fn with_protect(mut self, rects: Vec<Rect>) -> Grid {
        self.protect = rects;
        self
    }

    pub fn kind(&self, x: u16, y: u16) -> CellKind {
        if x >= self.width || y >= self.height {
            return CellKind::Blank;
        }
        self.kinds[y as usize * self.width as usize + x as usize]
    }

    pub fn set_kind(&mut self, x: u16, y: u16, k: CellKind) {
        if x < self.width && y < self.height {
            self.kinds[y as usize * self.width as usize + x as usize] = k;
        }
    }
}

/// How layout draws: the glyph set and the key labels for `{{key:…}}`.
pub struct Opts<'a> {
    pub glyphs: Glyphs,
    pub keys: &'a dyn KeyLabels,
}

impl Opts<'static> {
    pub fn new(glyphs: Glyphs) -> Opts<'static> {
        Opts {
            glyphs,
            keys: &NoKeys,
        }
    }
}

impl Default for Opts<'static> {
    fn default() -> Self {
        Opts::new(Glyphs::Rounded)
    }
}

/// A run of text in one role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    pub role: String,
}

/// Spans drawn from (`x`, `y`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub x: u16,
    pub y: u16,
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    /// A bordered box.
    Callout,
    /// One row across the area (narrow mode).
    Strip,
    /// A small tag at the edge an off-screen anchor lies beyond.
    Chip,
}

/// A box, strip or chip: its cells are filled in `fill`, a callout's border drawn in `border`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    pub kind: PanelKind,
    pub rect: Rect,
    pub fill: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    /// The side of the anchor it's on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    /// Where an arrow leaves the border, and its glyph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub junction: Option<(u16, u16, String)>,
    pub runs: Vec<Run>,
}

/// One cell of an arrow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteCell {
    pub x: u16,
    pub y: u16,
    pub g: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arrowed {
    pub role: String,
    /// From the callout to the head.
    pub cells: Vec<RouteCell>,
}

/// A layer laid out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub id: String,
    pub z: i16,
    pub agent: bool,
    /// Where its anchor resolved (`None` for a screen-placed layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Resolved>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow: Option<Arrowed>,
    pub panels: Vec<Panel>,
}

/// A spotlight: everything in `area` outside `holes` is dimmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spot {
    pub layer: String,
    pub area: Rect,
    pub holes: Vec<Rect>,
}

/// A ring around an anchor's cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RingMark {
    pub layer: String,
    pub agent: bool,
    pub rects: Vec<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse: Option<Pulse>,
}

/// A click target: a callout (swallows the click), a chip (runs its command) or an edge chip
/// (reveals the anchor). `id` is `<layer>`, `<layer>/chip/<chip id>` or `<layer>/reveal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub rect: Rect,
    pub id: String,
}

/// Everything the overlays draw this frame, in cell coordinates. Serializable: a pixel
/// renderer, a test or another process can draw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scene {
    pub width: u16,
    pub height: u16,
    pub glyphs: Glyphs,
    pub spots: Vec<Spot>,
    pub rings: Vec<RingMark>,
    /// In draw order (z, then push order).
    pub layers: Vec<Placed>,
    pub regions: Vec<Region>,
    /// Layers none of whose anchors resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

impl Scene {
    pub fn is_empty(&self) -> bool {
        self.spots.is_empty() && self.rings.is_empty() && self.layers.is_empty()
    }
}

/// The topmost region at a cell.
pub fn hit(scene: &Scene, x: u16, y: u16) -> Option<&Region> {
    scene.regions.iter().rev().find(|r| r.rect.contains(x, y))
}

/// The full name of an overlay role: `overlay.<base>`, or `overlay.agent.<base>` for an agent's.
pub fn role(base: &str, agent: bool) -> String {
    if agent {
        format!("overlay.agent.{base}")
    } else {
        format!("overlay.{base}")
    }
}

/// Below this many columns or rows of area, a layer draws as a strip.
pub const NARROW_COLS: u16 = 48;
pub const NARROW_ROWS: u16 = 12;
/// Below this many columns a strip drops its dots.
const DOTS_COLS: u16 = 24;
/// A callout's default widest, borders included.
const MAX_WIDTH: u16 = 52;
/// The sliver rule: fewer columns than this between a box and the area's edge, with text in
/// them, and the box reaches the edge.
const SLIVER: u16 = 8;

/// What the layout has claimed so far, across layers.
struct Taken {
    panels: Vec<Rect>,
    holes: Vec<Rect>,
    dimmed: Vec<bool>,
    any_dim: bool,
}

/// Summed-area tables of text cells and dimmed cells, for scoring a box in O(1).
struct Sums {
    w: usize,
    text: Vec<u32>,
    dim: Vec<u32>,
}

impl Sums {
    fn new(grid: &Grid, taken: &Taken) -> Sums {
        let (w, h) = (grid.width as usize + 1, grid.height as usize + 1);
        let mut text = vec![0u32; w * h];
        let mut dim = vec![0u32; w * h];
        for y in 0..grid.height as usize {
            for x in 0..grid.width as usize {
                let t = u32::from(grid.kind(x as u16, y as u16) != CellKind::Blank);
                let d = u32::from(taken.any_dim && taken.dimmed[y * grid.width as usize + x]);
                let i = (y + 1) * w + x + 1;
                text[i] = t + text[i - 1] + text[i - w] - text[i - w - 1];
                dim[i] = d + dim[i - 1] + dim[i - w] - dim[i - w - 1];
            }
        }
        Sums { w, text, dim }
    }

    fn sum(t: &[u32], w: usize, r: &Rect) -> u32 {
        let (x0, y0, x1, y1) = (
            r.x as usize,
            r.y as usize,
            r.right() as usize,
            r.bottom() as usize,
        );
        t[y1 * w + x1] + t[y0 * w + x0] - t[y0 * w + x1] - t[y1 * w + x0]
    }

    fn text(&self, r: &Rect) -> u32 {
        Self::sum(&self.text, self.w, r)
    }

    fn dim(&self, r: &Rect) -> u32 {
        Self::sum(&self.dim, self.w, r)
    }
}

/// The router's view of the screen for one layer.
struct RouteField<'a> {
    grid: &'a Grid,
    taken: &'a Taken,
    blocked: Vec<Rect>,
}

impl Field for RouteField<'_> {
    fn cost(&self, x: u16, y: u16) -> Option<u32> {
        if !self.grid.area.contains(x, y) || self.blocked.iter().any(|r| r.contains(x, y)) {
            return None;
        }
        match self.grid.kind(x, y) {
            CellKind::Wide => None,
            CellKind::Text => Some(route::TEXT),
            CellKind::Blank
                if x > 0 && self.grid.kind(x - 1, y) != CellKind::Blank
                    || self.grid.kind(x + 1, y) != CellKind::Blank =>
            {
                Some(route::GAP)
            }
            CellKind::Blank
                if self.taken.any_dim
                    && self.taken.dimmed[y as usize * self.grid.width as usize + x as usize] =>
            {
                Some(route::DIMMED)
            }
            CellKind::Blank => Some(route::BLANK),
        }
    }
}

/// A callout's content, measured.
struct Content {
    /// Lines of pieces; the first is the title line when there's a title or steps.
    lines: Vec<Vec<Piece>>,
    /// For each chip on the chips line: its id and its columns within the line.
    chips: Vec<(String, u16, u16)>,
    chips_line: Option<usize>,
    width: u16,
}

fn pieces(s: &str, role: &'static str) -> Vec<Piece> {
    use unicode_segmentation::UnicodeSegmentation;
    s.graphemes(true)
        .map(|g| {
            let p = text::printable(g);
            Piece {
                g: p.to_string(),
                w: text::width(p) as u16,
                role,
            }
        })
        .collect()
}

fn dots(of: u16, at: u16, glyphs: Glyphs) -> Vec<Piece> {
    let (on, off) = glyphs.dots();
    let mut v = pieces(&format!("{at} of {of}  "), "dots");
    for i in 1..=of.min(24) {
        v.extend(pieces(
            if i <= at { on } else { off },
            if i <= at { "dots.on" } else { "dots" },
        ));
    }
    v
}

fn content(layer: &Layer, inner: u16, opts: &Opts) -> Content {
    let c = layer.callout().cloned().unwrap_or_default();
    let steps = layer.items.iter().find_map(|i| match i {
        Item::Steps { of, at } => Some((*of, *at)),
        _ => None,
    });
    let mut lines: Vec<Vec<Piece>> = Vec::new();
    if c.title.is_some() || steps.is_some() {
        let d = steps
            .map(|(of, at)| dots(of, at, opts.glyphs))
            .unwrap_or_default();
        let dw = text::line_width(&d);
        let room = inner.saturating_sub(if dw > 0 { dw + 2 } else { 0 }) as usize;
        let mut line = pieces(
            &text::truncate(c.title.as_deref().unwrap_or(""), room, opts.glyphs),
            "callout.title",
        );
        if dw > 0 {
            let tw = text::line_width(&line);
            let gap = inner.saturating_sub(tw + dw).max(2);
            line.extend(std::iter::repeat_n(
                Piece {
                    g: " ".into(),
                    w: 1,
                    role: "callout",
                },
                gap as usize,
            ));
            line.extend(d);
        }
        lines.push(line);
    }
    if !c.body.is_empty() {
        lines.extend(text::wrap(
            &text::tokens(&c.body, "callout", opts.keys, opts.glyphs),
            inner,
        ));
    }
    for i in &layer.items {
        if let Item::Keys(ids) = i {
            let mut toks = Vec::new();
            for (k, id) in ids.iter().enumerate() {
                if k > 0 {
                    toks.push(Tok::Space);
                }
                toks.push(Tok::Badge(text::badge(id, opts.keys, opts.glyphs)));
            }
            lines.extend(text::wrap(&toks, inner));
        }
    }
    let mut chips = Vec::new();
    let mut chips_line = None;
    if !c.chips.is_empty() {
        let mut line: Vec<Piece> = Vec::new();
        for (k, ch) in c.chips.iter().enumerate() {
            if k > 0 {
                line.extend(pieces(opts.glyphs.sep(), "callout"));
            }
            let x0 = text::line_width(&line);
            for t in text::tokens(&ch.text, "chip", opts.keys, opts.glyphs) {
                match t {
                    Tok::Word(ps) | Tok::Badge(ps) => line.extend(ps),
                    Tok::Space => line.push(Piece {
                        g: " ".into(),
                        w: 1,
                        role: "chip",
                    }),
                    Tok::Break => {}
                }
            }
            chips.push((ch.id.clone(), x0, text::line_width(&line)));
        }
        // Chips that don't fit are cut, never wrapped: each must stay one click target.
        let mut w = 0;
        line.retain(|p| {
            w += p.w;
            w <= inner
        });
        chips.retain(|c| c.2 <= inner);
        chips_line = Some(lines.len());
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    let width = lines.iter().map(|l| text::line_width(l)).max().unwrap_or(0);
    Content {
        lines,
        chips,
        chips_line,
        width,
    }
}

fn spans(line: &[Piece], agent: bool) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for p in line {
        let r = role(p.role, agent);
        match out.last_mut() {
            Some(s) if s.role == r => s.text.push_str(&p.g),
            _ => out.push(Span {
                text: p.g.clone(),
                role: r,
            }),
        }
    }
    out
}

/// Where a layer points this frame.
enum Target {
    At(Resolved),
    Off(Off),
    Screen(ScreenPos),
}

fn target(layer: &Layer, anchors: &dyn Resolve, grid: &Grid) -> Option<Target> {
    let screen = Rect::new(0, 0, grid.width, grid.height);
    for a in &layer.anchor {
        if let Anchor::Screen(p) = a {
            return Some(Target::Screen(*p));
        }
        let Some(r) = anchors.resolve(a) else {
            continue;
        };
        let rects: Vec<Rect> = r
            .rects
            .iter()
            .map(|x| x.intersection(&screen))
            .filter(|x| !x.is_empty())
            .collect();
        if !rects.is_empty() {
            return Some(Target::At(Resolved::at(rects)));
        }
        if let Some(off) = r.off {
            return Some(Target::Off(off));
        }
    }
    None
}

/// Lays out every visible layer. Pure: the same layers, anchors and grid give the same scene.
pub fn layout(layers: &Layers, anchors: &dyn Resolve, grid: &Grid, opts: &Opts) -> Scene {
    let mut scene = Scene {
        width: grid.width,
        height: grid.height,
        glyphs: opts.glyphs,
        spots: Vec::new(),
        rings: Vec::new(),
        layers: Vec::new(),
        regions: Vec::new(),
        missing: Vec::new(),
    };
    if layers.hidden || grid.area.is_empty() {
        return scene;
    }
    let mut taken = Taken {
        panels: Vec::new(),
        holes: Vec::new(),
        dimmed: Vec::new(),
        any_dim: false,
    };
    for layer in layers.in_order() {
        let Some(t) = target(layer, anchors, grid) else {
            scene.missing.push(layer.id.clone());
            continue;
        };
        lay_one(layer, t, grid, opts, &mut taken, &mut scene);
    }
    scene
}

fn lay_one(
    layer: &Layer,
    t: Target,
    grid: &Grid,
    opts: &Opts,
    taken: &mut Taken,
    scene: &mut Scene,
) {
    let area = grid.area;
    let agent = layer.owner.is_agent();
    let ascii = opts.glyphs == Glyphs::Ascii;
    let narrow = area.w < NARROW_COLS || area.h < NARROW_ROWS;
    let has_callout = layer.callout().is_some()
        || layer
            .items
            .iter()
            .any(|i| matches!(i, Item::Keys(_) | Item::Steps { .. }));
    let wants_arrow = layer.items.iter().any(|i| matches!(i, Item::Arrow(_)));
    let mut placed = Placed {
        id: layer.id.clone(),
        z: layer.z,
        agent,
        anchor: None,
        arrow: None,
        panels: Vec::new(),
    };

    // The rects the callout must not cover and the arrow points at.
    let (anchor_rects, off) = match &t {
        Target::At(r) => (r.rects.clone(), None),
        Target::Off(o) => (Vec::new(), Some(*o)),
        Target::Screen(_) => (Vec::new(), None),
    };
    if let Target::At(r) = &t {
        placed.anchor = Some(r.clone());
    }
    if let Some(o) = off {
        placed.anchor = Some(Resolved::off(o));
    }

    // An off-screen anchor gets an edge chip; the callout docks beside it.
    let mut dock: Option<(Rect, Side)> = None;
    if let Some(o) = off {
        if let Some(chip) = edge_chip(layer, &o, grid, opts, agent) {
            let r = chip.rect;
            dock = Some((
                r,
                match o {
                    Off::Above { .. } => Side::Below,
                    Off::Below { .. } => Side::Above,
                    Off::Left { .. } => Side::Right,
                    Off::Right { .. } => Side::Left,
                },
            ));
            scene.regions.push(Region {
                rect: r,
                id: format!("{}/reveal", layer.id),
            });
            taken.panels.push(r);
            placed.panels.push(chip);
        }
    }

    let show_callout = has_callout && !(off.is_some() && layer.hide_off_screen);
    let mut callout_rect: Option<Rect> = None;
    if show_callout {
        let inner_max = layer
            .callout()
            .and_then(|c| c.max_width)
            .unwrap_or(MAX_WIDTH)
            .min(area.w * 2 / 3)
            .saturating_sub(4)
            .max(8);
        let ct = content(layer, inner_max, opts);
        let size = (ct.width.max(8) + 4, ct.lines.len() as u16 + 2);
        let mut chosen: Option<(Rect, Option<Side>)> = None;
        if !narrow {
            chosen = match (&t, dock) {
                (Target::Screen(p), _) => {
                    screen_box(*p, size, area, grid, taken, agent).map(|r| (r, None))
                }
                (_, Some((chip, side))) => {
                    place(layer, size, &[chip], &[side], grid, taken, agent, false)
                        .map(|(r, s)| (r, Some(s)))
                }
                (Target::At(_), None) => place(
                    layer,
                    size,
                    &anchor_rects,
                    &sides(layer),
                    grid,
                    taken,
                    agent,
                    wants_arrow,
                )
                .map(|(r, s)| (r, Some(s))),
                _ => None,
            };
        }
        match chosen {
            Some((r, side)) => {
                let panel = callout_panel(r, side, &ct, agent, &layer.id, scene);
                callout_rect = Some(r);
                taken.panels.push(r);
                // The arrow, to the anchor's nearest rect.
                if wants_arrow && dock.is_none() {
                    if let (Some(side), false) = (side, anchor_rects.is_empty()) {
                        let a = nearest(&anchor_rects, &r);
                        let field = RouteField {
                            grid,
                            taken,
                            blocked: blockers(grid, taken, &anchor_rects, None),
                        };
                        if let Some(p) = route::route(&field, r, side, a, area) {
                            let n = p.cells.len();
                            let cells = p
                                .cells
                                .iter()
                                .enumerate()
                                .map(|(k, &(x, y, din, dout))| RouteCell {
                                    x,
                                    y,
                                    g: route::glyph(din, dout, k + 1 == n, ascii).to_string(),
                                })
                                .collect();
                            placed.arrow = Some(Arrowed {
                                role: role("arrow", agent),
                                cells,
                            });
                            let mut panel = panel;
                            panel.junction = Some((
                                p.junction.0,
                                p.junction.1,
                                route::junction(side, ascii).to_string(),
                            ));
                            placed.panels.push(panel);
                        } else {
                            placed.panels.push(panel);
                        }
                    } else {
                        placed.panels.push(panel);
                    }
                } else {
                    placed.panels.push(panel);
                }
            }
            None => {
                let strip = strip(layer, &anchor_rects, off.as_ref(), grid, opts, agent);
                scene.regions.push(Region {
                    rect: strip.rect,
                    id: layer.id.clone(),
                });
                callout_rect = Some(strip.rect);
                taken.panels.push(strip.rect);
                placed.panels.push(strip);
            }
        }
    }

    // Rings and spotlights.
    for i in &layer.items {
        if let Item::Ring(r) = i {
            if !anchor_rects.is_empty() {
                scene.rings.push(RingMark {
                    layer: layer.id.clone(),
                    agent,
                    rects: anchor_rects.clone(),
                    pulse: r.pulse,
                });
            }
        }
    }
    if layer.dims() {
        let parts = layer
            .items
            .iter()
            .find_map(|i| match i {
                Item::Spotlight(s) => Some(s.holes.clone()),
                _ => None,
            })
            .unwrap_or_else(|| vec![Part::Anchor, Part::Callout]);
        let mut holes = Vec::new();
        if parts.contains(&Part::Anchor) {
            holes.extend(anchor_rects.iter().map(|r| r.grow(1, 0, &area)));
        }
        if parts.contains(&Part::Callout) {
            holes.extend(callout_rect);
        }
        // The edge chip is never dimmed.
        holes.extend(
            placed
                .panels
                .iter()
                .filter(|p| p.kind == PanelKind::Chip)
                .map(|p| p.rect),
        );
        if taken.dimmed.is_empty() {
            taken.dimmed = vec![false; grid.width as usize * grid.height as usize];
        }
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if !holes.iter().any(|h| h.contains(x, y)) {
                    taken.dimmed[y as usize * grid.width as usize + x as usize] = true;
                }
            }
        }
        taken.any_dim = true;
        taken.holes.extend(holes.iter().copied());
        scene.spots.push(Spot {
            layer: layer.id.clone(),
            area,
            holes,
        });
    }
    if !placed.panels.is_empty() || placed.arrow.is_some() {
        scene.layers.push(placed);
    }
}

fn sides(layer: &Layer) -> Vec<Side> {
    match layer.callout() {
        Some(c) if !c.place.is_empty() => c.place.clone(),
        _ => Side::DEFAULT.to_vec(),
    }
}

/// The rect of `rects` nearest to `to`.
fn nearest(rects: &[Rect], to: &Rect) -> Rect {
    *rects.iter().min_by_key(|r| gap(r, to)).unwrap_or(&rects[0])
}

/// Cells between two rects, along both axes.
fn gap(a: &Rect, b: &Rect) -> u32 {
    let dx = if a.right() <= b.x {
        b.x - a.right()
    } else if b.right() <= a.x {
        a.x - b.right()
    } else {
        0
    };
    let dy = if a.bottom() <= b.y {
        b.y - a.bottom()
    } else if b.bottom() <= a.y {
        a.y - b.bottom()
    } else {
        0
    };
    dx as u32 + dy as u32
}

/// Rects nothing of this layer may cover or cross.
fn blockers(grid: &Grid, taken: &Taken, anchor: &[Rect], own: Option<Rect>) -> Vec<Rect> {
    let mut v: Vec<Rect> = anchor.to_vec();
    v.extend(taken.panels.iter().copied());
    v.extend(taken.holes.iter().copied());
    v.extend(grid.protect.iter().copied());
    v.extend(own);
    v
}

/// Whether a box may go at `r`: inside the area, off the anchor, other overlays, holes and
/// protected cells, and (for agents) off the caret.
fn allowed(r: &Rect, grid: &Grid, taken: &Taken, anchor: &[Rect], agent: bool) -> bool {
    let area = grid.area;
    if r.x < area.x || r.y < area.y || r.right() > area.right() || r.bottom() > area.bottom() {
        return false;
    }
    if blockers(grid, taken, anchor, None)
        .iter()
        .any(|b| b.intersects(r))
    {
        return false;
    }
    !(agent && grid.caret.is_some_and(|(x, y)| r.contains(x, y)))
}

/// Picks a box for a callout of `size` beside `anchor` (design §2.4): candidates on each side
/// in order, shifted to fit, scored by the text they cover, dimmed cells, the caret and the
/// distance (and, with an arrow, the arrow's route); ties go to the earlier side. Then the
/// sliver rule. `None` if nothing fits.
#[allow(clippy::too_many_arguments)]
fn place(
    layer: &Layer,
    size: (u16, u16),
    anchor: &[Rect],
    order: &[Side],
    grid: &Grid,
    taken: &Taken,
    agent: bool,
    arrow: bool,
) -> Option<(Rect, Side)> {
    let _ = layer;
    let area = grid.area;
    let (bw, bh) = size;
    if bw > area.w || bh > area.h || anchor.is_empty() {
        return None;
    }
    let a = Rect::bounds(anchor);
    let sums = Sums::new(grid, taken);
    let clamp_x = |x: i32| x.clamp(area.x as i32, (area.right() - bw) as i32) as u16;
    let clamp_y = |y: i32| y.clamp(area.y as i32, (area.bottom() - bh) as i32) as u16;
    // Where along the side: the junction over the anchor's middle, or the box at either edge.
    let mid = a.x as i32 + (a.w.min(16) as i32 - 1) / 2;
    let xs = [
        mid - 4,
        area.x as i32,
        (area.right() - bw) as i32,
        mid - bw as i32 + 5,
    ];
    let ys = [
        a.y as i32 - 1,
        a.y as i32 - (bh as i32 - 2),
        a.y as i32 - bh as i32 / 2,
    ];
    let mut cands: Vec<(u32, usize, usize, Rect, Side, u32)> = Vec::new();
    let mut seq = 0;
    for (si, &side) in order.iter().enumerate() {
        let mut add = |r: Rect, k: u16| {
            seq += 1;
            if (r.right() > area.right())
                || (r.bottom() > area.bottom())
                || !allowed(&r, grid, taken, anchor, agent)
            {
                return;
            }
            // Scores are in tenths: text 10, dimmed 3, caret 500, distance 5 per cell.
            let text = sums.text(&r);
            let dim = sums.dim(&r);
            let caret = if grid.caret.is_some_and(|(x, y)| r.contains(x, y)) {
                500
            } else {
                0
            };
            let score = text * 10 + dim * 3 + caret + (gap(&r, &a) + k as u32) * 5;
            // A first guess at the arrow: the words on the straight way from the box to the
            // anchor (the route itself is costed for the best few below).
            let way = match side {
                Side::Below => Rect::new(
                    mid.clamp(r.x as i32, r.right() as i32 - 1) as u16,
                    a.bottom(),
                    1,
                    r.y - a.bottom(),
                ),
                Side::Above => Rect::new(
                    mid.clamp(r.x as i32, r.right() as i32 - 1) as u16,
                    r.bottom(),
                    1,
                    a.y - r.bottom(),
                ),
                Side::Right => Rect::new(
                    a.right(),
                    a.y.clamp(r.y, r.bottom() - 1),
                    r.x - a.right(),
                    1,
                ),
                Side::Left => Rect::new(
                    r.right(),
                    a.y.clamp(r.y, r.bottom() - 1),
                    a.x - r.right(),
                    1,
                ),
            };
            let guess = sums.text(&way) * route::TEXT * 5;
            cands.push((score + guess, si, seq, r, side, score));
        };
        match side {
            Side::Below | Side::Above => {
                for k in 1..=4u16 {
                    let y = if side == Side::Below {
                        a.bottom() as i32 + k as i32
                    } else {
                        a.y as i32 - k as i32 - bh as i32
                    };
                    if y < area.y as i32 || y + bh as i32 > area.bottom() as i32 {
                        continue;
                    }
                    let mut seen = Vec::new();
                    for x in xs {
                        let x = clamp_x(x);
                        if !seen.contains(&x) {
                            seen.push(x);
                            add(Rect::new(x, y as u16, bw, bh), k - 1);
                        }
                    }
                }
            }
            Side::Right | Side::Left => {
                for k in 2..=5u16 {
                    let x = if side == Side::Right {
                        a.right() as i32 + k as i32
                    } else {
                        a.x as i32 - k as i32 - bw as i32
                    };
                    if x < area.x as i32 || x + bw as i32 > area.right() as i32 {
                        continue;
                    }
                    let mut seen = Vec::new();
                    for y in ys {
                        let y = clamp_y(y);
                        if !seen.contains(&y) {
                            seen.push(y);
                            add(Rect::new(x as u16, y, bw, bh), k - 2);
                        }
                    }
                }
            }
        }
    }
    if cands.is_empty() {
        return None;
    }
    cands.sort_by_key(|c| (c.0, c.1, c.2));
    if arrow {
        // The best few by box alone, re-scored with the arrow they'd need: a box whose arrow
        // must cross words loses to one with a blank way.
        let field = RouteField {
            grid,
            taken,
            blocked: blockers(grid, taken, anchor, None),
        };
        let mut best: Option<(u32, usize, usize, Rect, Side)> = None;
        for c in cands.iter().take(12) {
            let near = nearest(anchor, &c.3);
            let mut f = RouteField {
                grid,
                taken,
                blocked: field.blocked.clone(),
            };
            f.blocked.push(c.3);
            // Crossing a word is worse than covering one: a box hides text, an arrow mangles it.
            let rc = route::route(&f, c.3, c.4, near, area).map_or(600, |p| {
                p.cost * 5
                    + 300
                        * p.cells
                            .iter()
                            .filter(|c| grid.kind(c.0, c.1) == CellKind::Text)
                            .count() as u32
            });
            let s = c.5 + rc;
            if best
                .as_ref()
                .is_none_or(|b| (s, c.1, c.2) < (b.0, b.1, b.2))
            {
                best = Some((s, c.1, c.2, c.3, c.4));
            }
        }
        let b = best?;
        return Some((sliver(b.3, b.4, &sums, grid, taken, anchor, agent), b.4));
    }
    let c = cands[0];
    Some((sliver(c.3, c.4, &sums, grid, taken, anchor, agent), c.4))
}

/// The sliver rule: a narrow gap with text between the box and the area's edge is closed by
/// stretching the box to the edge, or else by moving it there.
#[allow(clippy::too_many_arguments)]
fn sliver(
    r: Rect,
    side: Side,
    sums: &Sums,
    grid: &Grid,
    taken: &Taken,
    anchor: &[Rect],
    agent: bool,
) -> Rect {
    let _ = side;
    let area = grid.area;
    let mut r = r;
    let left = r.x - area.x;
    if left > 0 && left < SLIVER && sums.text(&Rect::new(area.x, r.y, left, r.h)) > 0 {
        let wide = Rect::new(area.x, r.y, r.w + left, r.h);
        let moved = Rect::new(area.x, r.y, r.w, r.h);
        if allowed(&wide, grid, taken, anchor, agent) {
            r = wide;
        } else if allowed(&moved, grid, taken, anchor, agent) {
            r = moved;
        }
    }
    let right = area.right() - r.right();
    if right > 0 && right < SLIVER && sums.text(&Rect::new(r.right(), r.y, right, r.h)) > 0 {
        let wide = Rect::new(r.x, r.y, r.w + right, r.h);
        let moved = Rect::new(r.x + right, r.y, r.w, r.h);
        if allowed(&wide, grid, taken, anchor, agent) {
            r = wide;
        } else if allowed(&moved, grid, taken, anchor, agent) {
            r = moved;
        }
    }
    r
}

/// A screen-placed box: centred, or near the top or bottom.
fn screen_box(
    p: ScreenPos,
    (bw, bh): (u16, u16),
    area: Rect,
    grid: &Grid,
    taken: &Taken,
    agent: bool,
) -> Option<Rect> {
    if bw > area.w || bh > area.h {
        return None;
    }
    let x = area.x + (area.w - bw) / 2;
    let y = match p {
        ScreenPos::Center => area.y + (area.h - bh) / 2,
        ScreenPos::Top => area.y + u16::from(area.h > bh),
        ScreenPos::Bottom => area.bottom() - bh - u16::from(area.h > bh),
    };
    let r = Rect::new(x, y, bw, bh);
    allowed(&r, grid, taken, &[], agent).then_some(r)
}

fn callout_panel(
    r: Rect,
    side: Option<Side>,
    ct: &Content,
    agent: bool,
    id: &str,
    scene: &mut Scene,
) -> Panel {
    // The callout's region goes under its chips' (hit takes the topmost).
    scene.regions.push(Region {
        rect: r,
        id: id.to_string(),
    });
    let mut runs = Vec::new();
    for (k, line) in ct.lines.iter().enumerate() {
        let y = r.y + 1 + k as u16;
        if y + 1 >= r.bottom() {
            break;
        }
        if !line.is_empty() {
            runs.push(Run {
                x: r.x + 2,
                y,
                spans: spans(line, agent),
            });
        }
        if ct.chips_line == Some(k) {
            for (cid, x0, x1) in &ct.chips {
                let cx = r.x + 2 + x0;
                let w = (x1 - x0).min(r.right().saturating_sub(2).saturating_sub(cx));
                if w > 0 {
                    scene.regions.push(Region {
                        rect: Rect::new(cx, y, w, 1),
                        id: format!("{id}/chip/{cid}"),
                    });
                }
            }
        }
    }
    Panel {
        kind: PanelKind::Callout,
        rect: r,
        fill: role("callout", agent),
        border: Some(role("callout.border", agent)),
        side,
        junction: None,
        runs,
    }
}

/// The text of a layer for one row: title, or else the body's first line.
fn short_text(layer: &Layer, opts: &Opts) -> Vec<Piece> {
    let c = layer.callout().cloned().unwrap_or_default();
    let src = match c.title.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => return pieces(t, "strip"),
        None => c.body.lines().next().unwrap_or("").to_string(),
    };
    let mut out = Vec::new();
    for t in text::tokens(&src, "strip", opts.keys, opts.glyphs) {
        match t {
            Tok::Word(ps) | Tok::Badge(ps) => out.extend(ps),
            Tok::Space => out.push(Piece {
                g: " ".into(),
                w: 1,
                role: "strip",
            }),
            Tok::Break => break,
        }
    }
    out
}

/// One row across the area: steps, the text, where an off-screen anchor is, the chips.
fn strip(
    layer: &Layer,
    anchor: &[Rect],
    off: Option<&Off>,
    grid: &Grid,
    opts: &Opts,
    agent: bool,
) -> Panel {
    let area = grid.area;
    let top = Rect::new(area.x, area.y, area.w, 1);
    let bottom = Rect::new(area.x, area.bottom() - 1, area.w, 1);
    let rect =
        if anchor.iter().any(|a| a.intersects(&top)) || matches!(off, Some(Off::Above { .. })) {
            bottom
        } else {
            top
        };
    let mut line: Vec<Piece> = vec![Piece {
        g: " ".into(),
        w: 1,
        role: "strip",
    }];
    if let Some((of, at)) = layer.items.iter().find_map(|i| match i {
        Item::Steps { of, at } => Some((*of, *at)),
        _ => None,
    }) {
        line.extend(pieces(&format!("{at}/{of} "), "strip"));
        if area.w >= DOTS_COLS && off.is_none() {
            let (on, offg) = opts.glyphs.dots();
            for i in 1..=of.min(24) {
                line.extend(pieces(
                    if i <= at { on } else { offg },
                    if i <= at { "dots.on" } else { "dots" },
                ));
            }
            line.push(Piece {
                g: " ".into(),
                w: 1,
                role: "strip",
            });
        }
    }
    line.extend(short_text(layer, opts));
    if let Some(o) = off {
        line.extend(pieces(opts.glyphs.sep(), "strip"));
        let word = match o {
            Off::Above { .. } => "above",
            Off::Below { .. } => "below",
            Off::Left { .. } => "left",
            Off::Right { .. } => "right",
        };
        line.extend(pieces(
            &format!("{} {word}", opts.glyphs.pointer(o)),
            "strip",
        ));
    }
    if let Some(c) = layer.callout() {
        for ch in &c.chips {
            line.extend(pieces(opts.glyphs.sep(), "strip"));
            for t in text::tokens(&ch.text, "strip", opts.keys, opts.glyphs) {
                match t {
                    Tok::Word(ps) | Tok::Badge(ps) => line.extend(ps),
                    Tok::Space => line.push(Piece {
                        g: " ".into(),
                        w: 1,
                        role: "strip",
                    }),
                    Tok::Break => {}
                }
            }
        }
    }
    let room = area.w.saturating_sub(1);
    if text::line_width(&line) > room {
        let ell = if opts.glyphs == Glyphs::Ascii {
            "..."
        } else {
            "…"
        };
        let ew = text::str_width(ell) as u16;
        let mut w = 0;
        line.retain(|p| {
            w += p.w;
            w + ew <= room
        });
        line.extend(pieces(ell, "strip"));
    }
    Panel {
        kind: PanelKind::Strip,
        rect,
        fill: role("strip", agent),
        border: None,
        side: None,
        junction: None,
        runs: vec![Run {
            x: rect.x,
            y: rect.y,
            spans: spans(&line, agent),
        }],
    }
}

/// The chip at the edge an off-screen anchor lies beyond: `↓ 2/11 here`.
fn edge_chip(layer: &Layer, off: &Off, grid: &Grid, opts: &Opts, agent: bool) -> Option<Panel> {
    let area = grid.area;
    let steps = layer.items.iter().find_map(|i| match i {
        Item::Steps { of, at } => Some(format!("{at}/{of} ")),
        _ => None,
    });
    let label = format!(
        " {} {}here ",
        opts.glyphs.pointer(off),
        steps.unwrap_or_default()
    );
    let w = text::str_width(&label) as u16;
    if w > area.w || area.h == 0 {
        return None;
    }
    let right_x = area.right() - w;
    let (x, y) = match *off {
        Off::Above { x } => (x.map_or(right_x, |x| x.clamp(area.x, right_x)), area.y),
        Off::Below { x } => (
            x.map_or(right_x, |x| x.clamp(area.x, right_x)),
            area.bottom() - 1,
        ),
        Off::Left { y } => (area.x, y.clamp(area.y, area.bottom() - 1)),
        Off::Right { y } => (right_x, y.clamp(area.y, area.bottom() - 1)),
    };
    let rect = Rect::new(x, y, w, 1);
    Some(Panel {
        kind: PanelKind::Chip,
        rect,
        fill: role("chip", agent),
        border: None,
        side: None,
        junction: None,
        runs: vec![Run {
            x,
            y,
            spans: vec![Span {
                text: label,
                role: role("chip", agent),
            }],
        }],
    })
}
