//! Placement and tracking: where each layer's box, strip, edge chip, arrow, holes and click
//! regions go this frame. Pure geometry in cell coordinates; what a box shows and how it looks
//! is the host's (or the built-in cell renderer's, [`layout`](crate::layout)).
//!
//! The host says how big a layer's content is ([`Measure`]); [`plan`] resolves anchors, picks
//! a side (flip, shift, the sliver rule, clamp), avoids other layers' boxes and holes and the
//! protected cells, falls back to a one-row strip when the area is narrow or nothing fits,
//! puts an edge chip where an off-screen anchor lies, and routes the arrow round words.

use serde::{Deserialize, Serialize};

use crate::compose::CellGrid;
use crate::geom::{Rect, Side};
use crate::model::{Anchor, Item, Layer, Layers, Part, Pulse, ScreenPos};
use crate::resolve::{Off, Resolve, Resolved};
use crate::route::{self, Dir, Field};
use crate::text;

/// How big a layer's box is, in cells, borders included. The host measures its own content.
pub trait Measure {
    /// The box for `layer`, at most `max` (columns, rows); `None` draws no box.
    fn size(&self, layer: &Layer, max: (u16, u16)) -> Option<(u16, u16)>;
    /// The width of the edge chip for an anchor that lies `off` screen.
    fn chip(&self, _layer: &Layer, _off: &Off) -> u16 {
        10
    }
}

impl<F: Fn(&Layer, (u16, u16)) -> Option<(u16, u16)>> Measure for F {
    fn size(&self, layer: &Layer, max: (u16, u16)) -> Option<(u16, u16)> {
        self(layer, max)
    }
}

/// How a layer's box was placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// A box beside the anchor (`side`), or at a screen position (no side).
    Box,
    /// One row across the area: the area is narrow, or no box fits.
    Strip,
}

/// One cell of an arrow's route: entered going `enter`, left going `leave`. The last cell is
/// the head, next to the anchor, and leaves the way it points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub x: u16,
    pub y: u16,
    pub enter: Dir,
    pub leave: Dir,
}

/// An arrow's geometry: where it leaves the box's border, and its cells to the head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub junction: (u16, u16),
    pub steps: Vec<Step>,
}

/// A layer placed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Planned {
    pub id: String,
    pub z: i16,
    pub agent: bool,
    /// Where its anchor resolved: cells, or which way it lies (`None` for a screen position).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Resolved>,
    /// Its box (or strip) and how it was placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    /// The side of the anchor the box is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    /// The edge chip of an off-screen anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chip: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<Route>,
    /// The anchor's cells, if it has a ring.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ring: Vec<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse: Option<Pulse>,
}

/// Every layer placed, in draw order (z, then push order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub width: u16,
    pub height: u16,
    pub layers: Vec<Planned>,
    pub spots: Vec<Spot>,
    pub regions: Vec<Region>,
    /// Layers none of whose anchors resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

impl Plan {
    /// The topmost region at a cell.
    pub fn hit(&self, x: u16, y: u16) -> Option<&Region> {
        hit_regions(&self.regions, x, y)
    }
}

/// The topmost (last) region containing a cell.
pub fn hit_regions(regions: &[Region], x: u16, y: u16) -> Option<&Region> {
    regions.iter().rev().find(|r| r.rect.contains(x, y))
}

/// Places every visible layer. Pure: the same layers, anchors, grid and sizes give the same plan.
pub fn plan(layers: &Layers, anchors: &dyn Resolve, grid: &Grid, measure: &dyn Measure) -> Plan {
    let mut out = Plan {
        width: grid.width,
        height: grid.height,
        layers: Vec::new(),
        spots: Vec::new(),
        regions: Vec::new(),
        missing: Vec::new(),
    };
    if layers.hidden || grid.area.is_empty() {
        return out;
    }
    let mut taken = Taken {
        panels: Vec::new(),
        holes: Vec::new(),
        dimmed: Vec::new(),
        any_dim: false,
    };
    for layer in layers.in_order() {
        let Some(t) = target(layer, anchors, grid) else {
            out.missing.push(layer.id.clone());
            continue;
        };
        plan_one(layer, t, grid, measure, &mut taken, &mut out);
    }
    out
}

/// The widest a box may be by default, borders included.
pub const MAX_WIDTH: u16 = 52;

fn plan_one(
    layer: &Layer,
    t: Target,
    grid: &Grid,
    measure: &dyn Measure,
    taken: &mut Taken,
    out: &mut Plan,
) {
    let area = grid.area;
    let agent = layer.owner.is_agent();
    let narrow = area.w < NARROW_COLS || area.h < NARROW_ROWS;
    let wants_arrow = layer.items.iter().any(|i| matches!(i, Item::Arrow(_)));
    let mut p = Planned {
        id: layer.id.clone(),
        z: layer.z,
        agent,
        anchor: None,
        rect: None,
        mode: None,
        side: None,
        chip: None,
        route: None,
        ring: Vec::new(),
        pulse: None,
    };
    let (anchor, off) = match &t {
        Target::At(r) => (r.rects.clone(), None),
        Target::Off(o) => (Vec::new(), Some(*o)),
        Target::Screen(_) => (Vec::new(), None),
    };
    p.anchor = match &t {
        Target::At(r) => Some(r.clone()),
        Target::Off(o) => Some(Resolved::off(*o)),
        Target::Screen(_) => None,
    };

    // An off-screen anchor gets an edge chip; the box docks beside it.
    let mut dock: Option<(Rect, Side)> = None;
    if let Some(o) = off
        && let Some(r) = chip_rect(&o, measure.chip(layer, &o), area)
    {
        let side = match o {
            Off::Above { .. } => Side::Below,
            Off::Below { .. } => Side::Above,
            Off::Left { .. } => Side::Right,
            Off::Right { .. } => Side::Left,
        };
        dock = Some((r, side));
        out.regions.push(Region {
            rect: r,
            id: format!("{}/reveal", layer.id),
        });
        taken.panels.push(r);
        p.chip = Some(r);
    }

    if layer.has_box() && !(off.is_some() && layer.hide_off_screen) {
        let max_w = layer
            .callout()
            .and_then(|c| c.max_width)
            .unwrap_or(MAX_WIDTH)
            .min(area.w * 2 / 3);
        let size = measure.size(layer, (max_w, area.h));
        let mut chosen: Option<(Rect, Option<Side>)> = None;
        if let (false, Some(size)) = (narrow, size) {
            chosen = match (&t, dock) {
                (Target::Screen(pos), _) => {
                    screen_box(*pos, size, area, grid, taken, agent).map(|r| (r, None))
                }
                (_, Some((chip, side))) => place(size, &[chip], &[side], grid, taken, agent, false)
                    .map(|(r, s)| (r, Some(s))),
                (Target::At(_), None) => place(
                    size,
                    &anchor,
                    &layer.sides(),
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
                p.rect = Some(r);
                p.mode = Some(Mode::Box);
                p.side = side;
                out.regions.push(Region {
                    rect: r,
                    id: layer.id.clone(),
                });
                taken.panels.push(r);
                if let (true, None, Some(side), false) =
                    (wants_arrow, dock, side, anchor.is_empty())
                {
                    let a = nearest(&anchor, &r);
                    let field = RouteField {
                        grid,
                        taken,
                        blocked: blockers(grid, taken, &anchor, None),
                    };
                    p.route = route::route(&field, r, side, a, area).map(|path| Route {
                        junction: path.junction,
                        steps: path
                            .cells
                            .iter()
                            .map(|&(x, y, enter, leave)| Step { x, y, enter, leave })
                            .collect(),
                    });
                }
            }
            None if size.is_some() => {
                let r = strip_rect(&anchor, off.as_ref(), area);
                p.rect = Some(r);
                p.mode = Some(Mode::Strip);
                out.regions.push(Region {
                    rect: r,
                    id: layer.id.clone(),
                });
                taken.panels.push(r);
            }
            None => {}
        }
    }

    if let Some(ring) = layer.items.iter().find_map(|i| match i {
        Item::Ring(r) => Some(r),
        _ => None,
    }) {
        p.ring = anchor.clone();
        p.pulse = ring.pulse;
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
            holes.extend(anchor.iter().map(|r| r.grow(1, 0, &area)));
        }
        if parts.contains(&Part::Callout) {
            holes.extend(p.rect);
        }
        // The edge chip is never dimmed.
        holes.extend(p.chip);
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
        out.spots.push(Spot {
            layer: layer.id.clone(),
            area,
            holes,
        });
    }
    out.layers.push(p);
}

/// The strip's row: the area's top, or its bottom when the anchor is on the top row or lies above.
fn strip_rect(anchor: &[Rect], off: Option<&Off>, area: Rect) -> Rect {
    let top = Rect::new(area.x, area.y, area.w, 1);
    let bottom = Rect::new(area.x, area.bottom() - 1, area.w, 1);
    if anchor.iter().any(|a| a.intersects(&top)) || matches!(off, Some(Off::Above { .. })) {
        bottom
    } else {
        top
    }
}

/// The edge chip's cells: on the edge the anchor lies beyond, at its column or row if known,
/// else at the right.
fn chip_rect(off: &Off, w: u16, area: Rect) -> Option<Rect> {
    if w == 0 || w > area.w || area.h == 0 {
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
    Some(Rect::new(x, y, w, 1))
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

/// A spotlight: everything in `area` outside `holes` is dimmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spot {
    pub layer: String,
    pub area: Rect,
    pub holes: Vec<Rect>,
}

/// A click target: a callout (swallows the click), a chip (runs its command) or an edge chip
/// (reveals the anchor). `id` is `<layer>`, `<layer>/chip/<chip id>` or `<layer>/reveal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub rect: Rect,
    pub id: String,
}

/// Below this many columns or rows of area, a layer draws as a strip.
pub const NARROW_COLS: u16 = 48;
pub const NARROW_ROWS: u16 = 12;

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
    size: (u16, u16),
    anchor: &[Rect],
    order: &[Side],
    grid: &Grid,
    taken: &Taken,
    agent: bool,
    arrow: bool,
) -> Option<(Rect, Side)> {
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
