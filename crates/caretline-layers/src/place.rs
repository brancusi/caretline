//! Placement and tracking: where each layer's box, strip, edge chip, arrow, holes and click
//! regions go this frame. Pure geometry in cell coordinates; what a box shows and how it looks
//! is the host's.
//!
//! The host registers a [`Renderer`] per content kind, which measures its content; [`plan`]
//! resolves anchors, picks a side (flip, shift, the sliver rule, clamp), avoids other layers'
//! boxes and holes and the protected cells, falls back to a one-row strip when the area is
//! narrow or nothing fits, puts an edge chip where an off-screen anchor lies, and routes the
//! arrow round words.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

use crate::geom::{Rect, Side};
use crate::model::{Anchor, Layer, Layers, Part, Pulse, ScreenPos};
use crate::resolve::{Off, Resolve, Resolved};
use crate::route::{self, Dir, Field};

/// A size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Size {
    pub w: u16,
    pub h: u16,
}

impl Size {
    pub const fn new(w: u16, h: u16) -> Size {
        Size { w, h }
    }
}

/// What a host registers per content kind. Placement asks it how big a box is; drawing the box
/// is the host's, outside this crate. `measure` must be pure: the same data and room, the same
/// size.
pub trait Renderer {
    /// The box for `data`, borders included, at most `avail`. A zero size means no box.
    fn measure(&self, data: &Value, avail: Size) -> Size;
    /// The edge chip shown where an off-screen anchor lies (default 8 by 1).
    fn chip(&self, _data: &Value) -> Size {
        Size::new(8, 1)
    }
}

impl<F: Fn(&Value, Size) -> Size> Renderer for F {
    fn measure(&self, data: &Value, avail: Size) -> Size {
        self(data, avail)
    }
}

/// The host's renderers, by content kind.
#[derive(Default)]
pub struct Renderers {
    kinds: BTreeMap<String, Box<dyn Renderer>>,
}

impl Renderers {
    pub fn new() -> Renderers {
        Renderers::default()
    }

    /// Registers the renderer for a kind (replacing any before it).
    pub fn register(mut self, kind: &str, r: impl Renderer + 'static) -> Renderers {
        self.kinds.insert(kind.to_string(), Box::new(r));
        self
    }

    pub fn get(&self, kind: &str) -> Option<&dyn Renderer> {
        self.kinds.get(kind).map(|b| b.as_ref())
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

/// A layer placed. The host draws its content in `rect` (a box or a one-row strip), the chip,
/// the route and the ring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Planned {
    pub id: String,
    pub z: i16,
    /// An agent's layer: the host styles it as one.
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
    /// Layers whose content kind has no renderer registered (placed without a box).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unrendered: Vec<String>,
}

impl Plan {
    /// The topmost region at a cell.
    pub fn hit(&self, x: u16, y: u16) -> Option<&Region> {
        hit_regions(&self.regions, x, y)
    }

    /// Whether a cell is dimmed by any spotlight (dims don't compound).
    pub fn dimmed(&self, x: u16, y: u16) -> bool {
        self.spots
            .iter()
            .any(|s| s.area.contains(x, y) && !s.holes.iter().any(|h| h.contains(x, y)))
    }
}

/// The topmost (last) region containing a cell.
pub fn hit_regions(regions: &[Region], x: u16, y: u16) -> Option<&Region> {
    regions.iter().rev().find(|r| r.rect.contains(x, y))
}

/// Places every visible layer. Pure: the same layers, resolved anchors, grid and measured
/// sizes give the same plan.
pub fn plan(layers: &Layers, anchors: &dyn Resolve, grid: &Grid, renderers: &Renderers) -> Plan {
    let mut out = Plan {
        width: grid.width,
        height: grid.height,
        layers: Vec::new(),
        spots: Vec::new(),
        regions: Vec::new(),
        missing: Vec::new(),
        unrendered: Vec::new(),
    };
    if layers.hidden || grid.area.is_empty() {
        return out;
    }
    let mut taken = Taken {
        panels: Vec::new(),
        holes: Vec::new(),
        dimmed: Vec::new(),
        any_dim: false,
        sums: None,
        costs: None,
    };
    for layer in layers.in_order() {
        let Some(t) = target(layer, anchors, grid) else {
            out.missing.push(layer.id.clone());
            continue;
        };
        plan_one(layer, t, grid, renderers, &mut taken, &mut out);
    }
    out
}

/// How many of the best boxes by cover are re-scored with their arrow's route.
const ROUTED: usize = 6;

/// The widest a box may be by default, borders included.
pub const MAX_WIDTH: u16 = 52;

fn plan_one(
    layer: &Layer,
    t: Target,
    grid: &Grid,
    renderers: &Renderers,
    taken: &mut Taken,
    out: &mut Plan,
) {
    let area = grid.area;
    let agent = layer.owner.is_agent();
    let narrow = area.w < NARROW_COLS || area.h < NARROW_ROWS;
    let renderer = layer.content.as_ref().and_then(|c| renderers.get(&c.kind));
    if layer.content.is_some() && renderer.is_none() {
        out.unrendered.push(layer.id.clone());
    }
    let data = layer.content.as_ref().map(|c| &c.data);
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
    if let Some(o) = off {
        let w = match (renderer, data) {
            (Some(r), Some(d)) => r.chip(d).w,
            _ => 8,
        };
        if let Some(r) = chip_rect(&o, w, grid, taken) {
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
            taken.panel(r, grid);
            p.chip = Some(r);
        }
    }

    if let (Some(rend), Some(data), false) =
        (renderer, data, off.is_some() && layer.hide_off_screen)
    {
        let max_w = layer.max_width.unwrap_or(MAX_WIDTH).min(area.w * 2 / 3);
        let avail = Size::new(max_w, area.h);
        let m = rend.measure(data, avail);
        let size = (m.w.min(avail.w), m.h.min(avail.h));
        let has_box = size.0 > 0 && size.1 > 0;
        let routed = layer.arrow && dock.is_none() && !anchor.is_empty();
        // (box, side, its route if placement already routed it)
        let mut chosen: Option<(Rect, Option<Side>, Option<route::Path>)> = None;
        if !narrow && has_box {
            if !matches!(t, Target::Screen(_)) {
                taken.ensure_sums(grid);
            }
            if routed {
                taken.ensure_costs(grid);
            }
            chosen = match (&t, dock) {
                (Target::Screen(pos), _) => {
                    screen_box(*pos, size, area, grid, taken, agent).map(|r| (r, None, None))
                }
                (_, Some((chip, side))) => place(size, &[chip], &[side], grid, taken, agent, false)
                    .map(|(r, s, _)| (r, Some(s), None)),
                (Target::At(_), None) => place(
                    size,
                    &anchor,
                    &layer.sides(),
                    grid,
                    taken,
                    agent,
                    layer.arrow,
                )
                .map(|(r, s, path)| (r, Some(s), path)),
                _ => None,
            };
        }
        match chosen {
            Some((r, side, path)) => {
                p.rect = Some(r);
                p.mode = Some(Mode::Box);
                p.side = side;
                out.regions.push(Region {
                    rect: r,
                    id: layer.id.clone(),
                });
                if let (true, Some(side)) = (routed, side) {
                    // The route placement found for this box, else (the sliver rule moved
                    // it) a fresh one.
                    let path = path.or_else(|| {
                        let field = CostField::new(grid, taken, &anchor, r);
                        route::route(&field, r, side, nearest(&anchor, &r), area, u32::MAX, None)
                            .path()
                    });
                    p.route = path.map(|path| Route {
                        junction: path.junction,
                        steps: path
                            .cells
                            .iter()
                            .map(|&(x, y, enter, leave)| Step { x, y, enter, leave })
                            .collect(),
                    });
                }
                taken.panel(r, grid);
            }
            None if has_box => {
                let r = strip_rect(&anchor, off.as_ref(), grid, taken);
                p.rect = Some(r);
                p.mode = Some(Mode::Strip);
                out.regions.push(Region {
                    rect: r,
                    id: layer.id.clone(),
                });
                taken.panel(r, grid);
            }
            None => {}
        }
    }

    if let Some(ring) = &layer.ring {
        p.ring = anchor.clone();
        p.pulse = ring.pulse;
    }
    if let Some(spot) = &layer.spotlight {
        let mut holes = Vec::new();
        if spot.holes.contains(&Part::Anchor) {
            holes.extend(anchor.iter().map(|r| r.grow(1, 0, &area)));
        }
        if spot.holes.contains(&Part::Box) {
            holes.extend(p.rect);
        }
        // The edge chip is never dimmed.
        holes.extend(p.chip);
        taken.dim(area, &holes, grid);
        for h in &holes {
            taken.hole(*h, grid);
        }
        out.spots.push(Spot {
            layer: layer.id.clone(),
            area,
            holes,
        });
    }
    out.layers.push(p);
}

/// The strip's row: the area's top, or its bottom when the anchor is on the top row or lies
/// above; failing that (another layer, a hole or protected cells there), the nearest free row
/// inward from it.
fn strip_rect(anchor: &[Rect], off: Option<&Off>, grid: &Grid, taken: &Taken) -> Rect {
    let area = grid.area;
    let row = |y: u16| Rect::new(area.x, y, area.w, 1);
    let top_first = !(anchor.iter().any(|a| a.intersects(&row(area.y)))
        || matches!(off, Some(Off::Above { .. })));
    let ys: Vec<u16> = if top_first {
        (area.y..area.bottom()).collect()
    } else {
        (area.y..area.bottom()).rev().collect()
    };
    let blocked = blockers(grid, taken, anchor, None);
    ys.iter()
        .map(|&y| row(y))
        .find(|r| !blocked.iter().any(|b| b.intersects(r)))
        .unwrap_or_else(|| row(ys[0]))
}

/// The edge chip's cells: on the edge the anchor lies beyond, at its column or row if known,
/// else at the right; nudged left off a wide grapheme's second half.
fn chip_rect(off: &Off, w: u16, grid: &Grid, taken: &Taken) -> Option<Rect> {
    let area = grid.area;
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
    let mut r = Rect::new(x, y, w, 1);
    while r.x > area.x && grid.splits(&r) {
        r.x -= 1;
    }
    let _ = taken;
    (!grid.splits(&r)).then_some(r)
}

/// What a cell of the host's screen holds, for placement and routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellKind {
    Blank,
    Text,
    /// The first cell of a wide grapheme: covered like text, never crossed by an arrow.
    Wide,
    /// Its second cell. No box edge lands between the two.
    WideTail,
}

/// The host's screen as placement sees it: its size, where layers may go (`area`: the text
/// rows, never the status row), the caret, cells no box may cover (a selection, a prompt),
/// and which cells hold text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub width: u16,
    pub height: u16,
    pub area: Rect,
    pub caret: Option<(u16, u16)>,
    pub protect: Vec<Rect>,
    kinds: Vec<CellKind>,
}

/// A grapheme's width in cells, as caretline measures it.
pub fn width(g: &str) -> usize {
    if g.is_empty() {
        return 0;
    }
    if g.is_ascii() {
        return 1;
    }
    let emoji = g
        .chars()
        .any(|c| c == '\u{200D}' || c == '\u{FE0F}' || ('\u{1F3FB}'..='\u{1F3FF}').contains(&c))
        || (g.chars().count() == 2 && g.chars().all(|c| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)));
    if emoji {
        2
    } else {
        unicode_width::UnicodeWidthStr::width(g).max(1)
    }
}

impl Grid {
    /// A blank screen; layers may use all of it.
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

    /// Marks the cells a string drawn at (`x`, `y`) covers: text, and wide graphemes as such.
    pub fn mark_text(&mut self, x: u16, y: u16, s: &str) {
        let mut cx = x;
        for g in s.graphemes(true) {
            let w = width(g) as u16;
            match (w, g.trim().is_empty()) {
                (2, _) => {
                    self.set_kind(cx, y, CellKind::Wide);
                    self.set_kind(cx + 1, y, CellKind::WideTail);
                }
                (_, true) => {}
                _ => self.set_kind(cx, y, CellKind::Text),
            }
            cx = cx.saturating_add(w);
        }
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

    /// Whether a rect's left or right edge falls between the halves of a wide grapheme.
    pub fn splits(&self, r: &Rect) -> bool {
        (r.y..r.bottom()).any(|y| {
            self.kind(r.x, y) == CellKind::WideTail || self.kind(r.right(), y) == CellKind::WideTail
        })
    }
}

/// A spotlight: everything in `area` outside `holes` is dimmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spot {
    pub layer: String,
    pub area: Rect,
    pub holes: Vec<Rect>,
}

/// A click target: a box or strip (`<layer>`: the host decides, typically swallowing the
/// click), or an edge chip (`<layer>/reveal`: reveal the anchor). A host adds its own regions
/// inside a box (buttons) as `<layer>/<name>`.
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

/// What the layout has claimed so far, across layers, and the tables built from it once per
/// plan (kept in step as layers claim more): box scores and routing costs.
struct Taken {
    panels: Vec<Rect>,
    holes: Vec<Rect>,
    dimmed: Vec<bool>,
    any_dim: bool,
    /// Text and dimmed cells, for scoring boxes; rebuilt when a spotlight dims.
    sums: Option<Sums>,
    /// What entering each cell costs an arrow ([`NO_WAY`]: it can't), before this layer's
    /// anchor and box block their cells. Built on the first arrow.
    costs: Option<Vec<u32>>,
}

/// A cell no arrow may enter.
const NO_WAY: u32 = u32::MAX;

impl Taken {
    fn ensure_sums(&mut self, grid: &Grid) {
        if self.sums.is_none() {
            self.sums = Some(Sums::new(grid, self));
        }
    }

    fn sums(&self) -> &Sums {
        self.sums.as_ref().expect("sums built before placing")
    }

    /// Builds the routing costs once: outside the area, wide graphemes and claimed cells
    /// block; a text cell costs [`route::TEXT`], a blank one between words [`route::GAP`], a
    /// dimmed blank one [`route::DIMMED`], any other blank one [`route::BLANK`].
    fn ensure_costs(&mut self, grid: &Grid) {
        if self.costs.is_some() {
            return;
        }
        let w = grid.width as usize;
        let mut costs = vec![NO_WAY; w * grid.height as usize];
        let area = grid.area;
        let blank = |k: Option<&CellKind>| k.is_none_or(|k| *k == CellKind::Blank);
        for y in area.y as usize..area.bottom() as usize {
            let row = &grid.kinds[y * w..(y + 1) * w];
            for x in area.x as usize..area.right() as usize {
                let i = y * w + x;
                costs[i] = match row[x] {
                    CellKind::Wide | CellKind::WideTail => NO_WAY,
                    CellKind::Text => route::TEXT,
                    CellKind::Blank
                        if x > 0 && !blank(row.get(x - 1)) || !blank(row.get(x + 1)) =>
                    {
                        route::GAP
                    }
                    CellKind::Blank if self.any_dim && self.dimmed[i] => route::DIMMED,
                    CellKind::Blank => route::BLANK,
                };
            }
        }
        for r in self.panels.iter().chain(&self.holes).chain(&grid.protect) {
            block(&mut costs, grid, r);
        }
        self.costs = Some(costs);
    }

    /// Claims a box, strip or chip.
    fn panel(&mut self, r: Rect, grid: &Grid) {
        self.panels.push(r);
        if let Some(c) = &mut self.costs {
            block(c, grid, &r);
        }
    }

    /// Claims a spotlight hole.
    fn hole(&mut self, r: Rect, grid: &Grid) {
        self.holes.push(r);
        if let Some(c) = &mut self.costs {
            block(c, grid, &r);
        }
    }

    /// Dims `area` outside `holes` (dims don't compound).
    fn dim(&mut self, area: Rect, holes: &[Rect], grid: &Grid) {
        let w = grid.width as usize;
        if self.dimmed.is_empty() {
            self.dimmed = vec![false; w * grid.height as usize];
        }
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if !holes.iter().any(|h| h.contains(x, y)) {
                    let i = y as usize * w + x as usize;
                    self.dimmed[i] = true;
                    if let Some(c) = &mut self.costs
                        && c[i] == route::BLANK
                    {
                        c[i] = route::DIMMED;
                    }
                }
            }
        }
        self.any_dim = true;
        self.sums = None;
    }
}

/// Marks a rect's cells as ones no arrow may enter.
fn block(costs: &mut [u32], grid: &Grid, r: &Rect) {
    let r = r.intersection(&Rect::new(0, 0, grid.width, grid.height));
    for y in r.y..r.bottom() {
        let row = y as usize * grid.width as usize;
        costs[row + r.x as usize..row + r.right() as usize].fill(NO_WAY);
    }
}

/// Summed-area tables of text cells and dimmed cells, for scoring a box in O(1).
struct Sums {
    w: usize,
    text: Vec<u32>,
    /// `None` while nothing is dimmed.
    dim: Option<Vec<u32>>,
}

impl Sums {
    fn new(grid: &Grid, taken: &Taken) -> Sums {
        let gw = grid.width as usize;
        let (w, h) = (gw + 1, grid.height as usize + 1);
        // Each row's running count plus the row above's table.
        let table = |cell: &dyn Fn(usize) -> bool| {
            let mut t = vec![0u32; w * h];
            for y in 0..h - 1 {
                let mut run = 0u32;
                let (above, row) = t.split_at_mut((y + 1) * w);
                let above = &above[y * w..];
                for x in 0..gw {
                    run += u32::from(cell(y * gw + x));
                    row[x + 1] = run + above[x + 1];
                }
            }
            t
        };
        let text = table(&|i| grid.kinds[i] != CellKind::Blank);
        let dim = taken.any_dim.then(|| table(&|i| taken.dimmed[i]));
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
        self.dim.as_ref().map_or(0, |d| Self::sum(d, self.w, r))
    }
}

/// The router's view of the screen for one layer and one candidate box: the plan's costs,
/// with the anchor's cells and the box blocked.
struct CostField<'a> {
    costs: &'a [u32],
    width: u16,
    height: u16,
    anchor: &'a [Rect],
    own: Rect,
}

impl<'a> CostField<'a> {
    fn new(grid: &Grid, taken: &'a Taken, anchor: &'a [Rect], own: Rect) -> CostField<'a> {
        CostField {
            costs: taken.costs.as_deref().expect("costs built before routing"),
            width: grid.width,
            height: grid.height,
            anchor,
            own,
        }
    }
}

impl Field for CostField<'_> {
    fn cost(&self, x: u16, y: u16) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let c = self.costs[y as usize * self.width as usize + x as usize];
        if c == NO_WAY || self.own.contains(x, y) || self.anchor.iter().any(|r| r.contains(x, y)) {
            return None;
        }
        Some(c)
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
    if grid.splits(r) {
        return false;
    }
    if anchor
        .iter()
        .chain(&taken.panels)
        .chain(&taken.holes)
        .chain(&grid.protect)
        .any(|b| b.intersects(r))
    {
        return false;
    }
    !(agent && grid.caret.is_some_and(|(x, y)| r.contains(x, y)))
}

/// Picks a box for a callout of `size` beside `anchor` (design §2.4): candidates on each side
/// in order, shifted to fit, scored by the text they cover, dimmed cells, the caret and the
/// distance (and, with an arrow, the arrow's route); ties go to the earlier side. Then the
/// sliver rule. `None` if nothing fits. With an arrow, also the chosen box's route when the
/// sliver rule left the box where it was routed.
#[allow(clippy::too_many_arguments)]
fn place(
    size: (u16, u16),
    anchor: &[Rect],
    order: &[Side],
    grid: &Grid,
    taken: &Taken,
    agent: bool,
    arrow: bool,
) -> Option<(Rect, Side, Option<route::Path>)> {
    let area = grid.area;
    let (bw, bh) = size;
    if bw > area.w || bh > area.h || anchor.is_empty() {
        return None;
    }
    let a = Rect::bounds(anchor);
    let sums = taken.sums();
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
        // must cross words loses to one with a blank way. The winner is the least (score,
        // side, order) of these, whatever order they're routed in, so they go cheapest bound
        // first and a box whose bound can't win isn't routed at all.
        // The least each arrow can cost, from one search back from the anchor per (side,
        // anchor rect), over the cells any of their routes may use.
        let open = CostField::new(grid, taken, anchor, Rect::default());
        let mut goals: Vec<(Side, Rect, route::ToGoal)> = Vec::new();
        for c in cands.iter().take(ROUTED) {
            let near = nearest(anchor, &c.3);
            if goals.iter().any(|g| g.0 == c.4 && g.1 == near) {
                continue;
            }
            let region = cands
                .iter()
                .take(ROUTED)
                .filter(|o| o.4 == c.4 && nearest(anchor, &o.3) == near)
                .map(|o| route::reach(o.3, o.4, near, area))
                .fold(
                    Rect::default(),
                    |r, o| if r.is_empty() { o } else { r.union(&o) },
                );
            goals.push((c.4, near, route::ToGoal::new(&open, c.4, near, region)));
        }
        let goal_of = |c: &(u32, usize, usize, Rect, Side, u32)| {
            let near = nearest(anchor, &c.3);
            &goals
                .iter()
                .find(|g| g.0 == c.4 && g.1 == near)
                .expect("a search per side")
                .2
        };
        let mut few: Vec<(u32, &(u32, usize, usize, Rect, Side, u32))> = cands
            .iter()
            .take(ROUTED)
            .map(|c| {
                let field = CostField::new(grid, taken, anchor, c.3);
                let near = nearest(anchor, &c.3);
                let lb = goal_of(c)
                    .bound(&field, c.3, c.4, near)
                    .map_or(NO_ROUTE, |b| (b * 5).min(NO_ROUTE));
                (c.5 + lb, c)
            })
            .collect();
        few.sort_by_key(|(lb, c)| (*lb, c.1, c.2));
        let mut best: Option<(u32, usize, usize, Rect, Side, Option<route::Path>)> = None;
        for (lb, c) in few {
            if best
                .as_ref()
                .is_some_and(|b| (lb, c.1, c.2) >= (b.0, b.1, b.2))
            {
                continue;
            }
            // A route that would lose costs more than the room left (and when even no way, at
            // 600, would lose, the search can stop there).
            let limit = match &best {
                Some(b) if b.0 - c.5 < NO_ROUTE => (b.0 - c.5) / 5,
                _ => u32::MAX,
            };
            let near = nearest(anchor, &c.3);
            let field = CostField::new(grid, taken, anchor, c.3);
            // Crossing a word is worse than covering one: a box hides text, an arrow mangles it.
            let (rc, path) =
                match route::route(&field, c.3, c.4, near, area, limit, Some(goal_of(c))) {
                    route::Routed::Over => continue,
                    route::Routed::NoWay => (NO_ROUTE, None),
                    route::Routed::Found(p) => (
                        p.cost * 5
                            + 300
                                * p.cells
                                    .iter()
                                    .filter(|c| grid.kind(c.0, c.1) == CellKind::Text)
                                    .count() as u32,
                        Some(p),
                    ),
                };
            let s = c.5 + rc;
            if best
                .as_ref()
                .is_none_or(|b| (s, c.1, c.2) < (b.0, b.1, b.2))
            {
                best = Some((s, c.1, c.2, c.3, c.4, path));
            }
        }
        let b = best?;
        let r = sliver(b.3, b.4, sums, grid, taken, anchor, agent);
        return Some((r, b.4, b.5.filter(|_| r == b.3)));
    }
    let c = cands[0];
    Some((
        sliver(c.3, c.4, sums, grid, taken, anchor, agent),
        c.4,
        None,
    ))
}

/// What a box with no route for its arrow pays.
const NO_ROUTE: u32 = 600;

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
