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
use crate::model::{Anchor, Layer, Layers, Owner, Part, Pulse, ScreenPos};
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
    /// The edge chip shown where an off-screen anchor lies: its width (it is one row high;
    /// default 8). It gets the layer's data, the anchor that lies off screen and which way, so
    /// a host can size a label such as "↓ 2/11 here" to fit. Pure, like `measure`.
    fn chip(&self, _data: &Value, _anchor: &Anchor, _off: Off) -> Size {
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

/// An edge of a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Where something meets a box's border: which edge, and how many cells along it from the
/// box's left (top and bottom edges) or top (left and right edges), the corner being 0. A host
/// lays out its border round it: a title never sits where an arrow attaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Attach {
    pub edge: Edge,
    pub offset: u16,
}

impl Attach {
    /// Where the cell at (`x`, `y`) on `r`'s border is.
    fn at(r: &Rect, edge: Edge, x: u16, y: u16) -> Attach {
        let offset = match edge {
            Edge::Top | Edge::Bottom => x.saturating_sub(r.x).min(r.w.saturating_sub(1)),
            Edge::Left | Edge::Right => y.saturating_sub(r.y).min(r.h.saturating_sub(1)),
        };
        Attach { edge, offset }
    }
}

/// An arrow's geometry: where it leaves the box's border (`junction`, and as `attach`), and
/// its cells to the head. On a left or right edge it never attaches beside the title row (the
/// first row inside the border) of a box taller than three rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub junction: (u16, u16),
    pub attach: Attach,
    pub steps: Vec<Step>,
}

/// Why a layer that asked for an arrow has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoArrow {
    /// The anchor is off screen: the box docks against its edge chip (`Planned::dock`), which
    /// points the way.
    Docked,
    /// The layer is at a screen position, not an anchor.
    Screen,
    /// No box to start from: a strip, or no box at all.
    NoBox,
    /// No way from any candidate box to the anchor round the other layers, holes, protected
    /// cells and wide graphemes.
    NoWay,
}

/// A layer placed. The host draws its content in `rect` (a box or a one-row strip), the chip,
/// the route and the ring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Planned {
    pub id: String,
    pub z: i16,
    /// An agent's layer: the host styles it as one.
    pub agent: bool,
    /// Whose layer it is (`agent:<actor>` for an agent's, [`Owner::actor`]). The crate draws
    /// nothing and writes no attribution into the content; attributing an agent's layers to
    /// it, so they can't pass as the host's own, is recommended and the host's choice.
    pub owner: Owner,
    /// Where its anchor resolved: cells, or which way it lies (`None` for a screen position),
    /// and the view it resolved in (`Resolved::view`).
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
    /// Why there's no route, when the layer asked for an arrow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_arrow: Option<NoArrow>,
    /// A docked box (its anchor off screen): where on its border the edge chip touches it.
    /// The host joins the two there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<Attach>,
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
        anchors: Vec::new(),
        routes: Vec::new(),
        dimmed: Vec::new(),
        any_dim: false,
        sums: None,
        costs: None,
    };
    // Every layer's anchor first: no box, chip or strip covers another layer's anchor, whichever
    // is placed first.
    let targets: Vec<(&Layer, Option<Target>)> = layers
        .in_order()
        .into_iter()
        .map(|l| (l, target(l, anchors, grid)))
        .collect();
    for (_, t) in &targets {
        if let Some(Target::At(r)) = t {
            taken.anchors.extend(r.rects.iter().copied());
        }
    }
    for (layer, t) in targets {
        let Some(t) = t else {
            out.missing.push(layer.id.clone());
            continue;
        };
        plan_one(layer, t, grid, renderers, &mut taken, &mut out);
    }
    out
}

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
        owner: layer.owner.clone(),
        anchor: None,
        rect: None,
        mode: None,
        side: None,
        chip: None,
        route: None,
        no_arrow: None,
        dock: None,
        ring: Vec::new(),
        pulse: None,
    };
    let (anchor, off) = match &t {
        Target::At(r) => (r.rects.clone(), None),
        Target::Off(o, _, _) => (Vec::new(), Some(*o)),
        Target::Screen(_) => (Vec::new(), None),
    };
    p.anchor = match &t {
        Target::At(r) => Some(r.clone()),
        Target::Off(o, _, view) => Some(Resolved::off(*o).in_view(view.as_deref())),
        Target::Screen(_) => None,
    };

    // An off-screen anchor gets an edge chip; the box docks against it.
    let mut dock: Option<(Rect, Side)> = None;
    if let (Some(o), Target::Off(_, which, _)) = (off, &t) {
        let w = match (renderer, data) {
            (Some(r), Some(d)) => r.chip(d, which, o).w,
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
        let whole = Size::new(max_w, area.h);
        // What the renderer gives for `avail`, never over the whole room; zero: no box.
        let measure = |avail: Size| {
            let m = rend.measure(data, avail);
            let s = (m.w.min(whole.w), m.h.min(whole.h));
            (s.0 > 0 && s.1 > 0).then_some(s)
        };
        let routed = layer.arrow && dock.is_none() && !anchor.is_empty();
        // Each candidate side measured with the room it really has (design §2.4).
        let sized = |beside: &[Rect], order: &[Side], flush: bool| -> Vec<(Side, (u16, u16))> {
            order
                .iter()
                .filter_map(|&s| {
                    let avail = room(beside, s, max_w, area, flush);
                    if avail.w == 0 || avail.h == 0 {
                        return None;
                    }
                    Some((s, measure(avail)?))
                })
                .collect()
        };
        let sides: Vec<(Side, (u16, u16))> = if narrow {
            Vec::new()
        } else {
            match (&t, dock) {
                (Target::Screen(_), _) => Vec::new(),
                (_, Some((chip, side))) => sized(&[chip], &[side], true),
                (Target::At(_), None) => sized(&anchor, &layer.sides(), false),
                _ => Vec::new(),
            }
        };
        let screen_size = match &t {
            Target::Screen(_) if !narrow => measure(whole),
            _ => None,
        };
        // Whether the content has a box at all (else nothing is drawn, not even a strip).
        let has_box = !sides.is_empty() || screen_size.is_some() || measure(whole).is_some();
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
                (Target::Screen(pos), _) => screen_size
                    .and_then(|size| screen_box(*pos, size, area, grid, taken, agent))
                    .map(|r| (r, None, None)),
                (_, Some((chip, _))) => place(&sides, &[chip], grid, taken, agent, false, true)
                    .map(|(r, s, _)| (r, Some(s), None)),
                (Target::At(_), None) => {
                    place(&sides, &anchor, grid, taken, agent, layer.arrow, false)
                        .map(|(r, s, path)| (r, Some(s), path))
                }
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
                    let edge = match side {
                        Side::Below => Edge::Top,
                        Side::Above => Edge::Bottom,
                        Side::Right => Edge::Left,
                        Side::Left => Edge::Right,
                    };
                    p.route = path.map(|path| Route {
                        junction: path.junction,
                        attach: Attach::at(&r, edge, path.junction.0, path.junction.1),
                        steps: path
                            .cells
                            .iter()
                            .map(|&(x, y, enter, leave)| Step { x, y, enter, leave })
                            .collect(),
                    });
                }
                if let (Some((chip, side)), Some(_)) = (dock, side) {
                    // The box's edge facing its chip, at the chip's middle where they meet.
                    let (lo, hi) = (r.x.max(chip.x), r.right().min(chip.right()));
                    let x = (chip.x + chip.w / 2)
                        .max(lo)
                        .min(hi.saturating_sub(1).max(lo));
                    let (edge, x, y) = match side {
                        Side::Above => (Edge::Bottom, x, chip.y),
                        Side::Below => (Edge::Top, x, chip.y),
                        Side::Right => (Edge::Left, chip.x, chip.y),
                        Side::Left => (Edge::Right, chip.x, chip.y),
                    };
                    p.dock = Some(Attach::at(&r, edge, x, y));
                }
                taken.panel(r, grid);
                if let Some(route) = &p.route {
                    taken.route(route, grid);
                }
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

    if layer.arrow && p.route.is_none() {
        p.no_arrow = Some(if matches!(t, Target::Screen(_)) {
            NoArrow::Screen
        } else if p.mode != Some(Mode::Box) {
            NoArrow::NoBox
        } else if off.is_some() {
            NoArrow::Docked
        } else {
            NoArrow::NoWay
        });
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
/// else at the right. Where that's taken (another chip or box, a hole, protected cells, a
/// layer's anchor or arrow) or would split a wide grapheme, the nearest free place along the
/// same edge, nearer the start first. `None` if the edge has no room.
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
    let free = |r: &Rect| !grid.splits(r) && !taken.covers(r, grid);
    let along_row = matches!(off, Off::Above { .. } | Off::Below { .. });
    let (lo, hi, at) = if along_row {
        (area.x, right_x, x)
    } else {
        (area.y, area.bottom() - 1, y)
    };
    let put = |v: u16| {
        if along_row {
            Rect::new(v, y, w, 1)
        } else {
            Rect::new(x, v, w, 1)
        }
    };
    (0..=hi - lo).find_map(|d| {
        [at.checked_sub(d), at.checked_add(d)]
            .into_iter()
            .flatten()
            .filter(|v| (lo..=hi).contains(v))
            .map(put)
            .find(&free)
    })
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
    /// Every layer's anchor cells (all of them, before any is placed).
    anchors: Vec<Rect>,
    /// The cells of the arrows placed so far.
    routes: Vec<Rect>,
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
        for r in self
            .panels
            .iter()
            .chain(&self.holes)
            .chain(&self.routes)
            .chain(&grid.protect)
        {
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

    /// Claims an arrow's cells: no later box covers them and no later arrow crosses them.
    fn route(&mut self, route: &Route, grid: &Grid) {
        for s in &route.steps {
            let r = Rect::new(s.x, s.y, 1, 1);
            self.routes.push(r);
            if let Some(c) = &mut self.costs {
                block(c, grid, &r);
            }
        }
    }

    /// Whether `r` meets anything claimed or protected: a box, chip or strip, a hole, a layer's
    /// anchor or arrow.
    fn covers(&self, r: &Rect, grid: &Grid) -> bool {
        self.panels
            .iter()
            .chain(&self.holes)
            .chain(&self.anchors)
            .chain(&self.routes)
            .chain(&grid.protect)
            .any(|b| b.intersects(r))
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
    /// Which way it lies, the anchor that does, and the view it lies off.
    Off(Off, Anchor, Option<String>),
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
            return Some(Target::At(Resolved::at(rects).in_view(r.view.as_deref())));
        }
        if let Some(off) = r.off {
            return Some(Target::Off(off, a.clone(), r.view));
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
    v.extend(taken.anchors.iter().copied());
    v.extend(taken.routes.iter().copied());
    v.extend(grid.protect.iter().copied());
    v.extend(own);
    v
}

/// Whether a box may go at `r`: inside the area, off its anchor and every other layer's,
/// other boxes and chips, arrows, holes and protected cells, and (for agents) off the caret.
fn allowed(r: &Rect, grid: &Grid, taken: &Taken, anchor: &[Rect], agent: bool) -> bool {
    let area = grid.area;
    if r.x < area.x || r.y < area.y || r.right() > area.right() || r.bottom() > area.bottom() {
        return false;
    }
    if grid.splits(r) {
        return false;
    }
    if anchor.iter().any(|b| b.intersects(r)) || taken.covers(r, grid) {
        return false;
    }
    !(agent && grid.caret.is_some_and(|(x, y)| r.contains(x, y)))
}

/// The room on one side of `beside` for a box: the area's cells past the gap an arrow needs
/// (none when `flush`), at most `max_w` wide. The renderer measures for it.
fn room(beside: &[Rect], side: Side, max_w: u16, area: Rect, flush: bool) -> Size {
    let a = Rect::bounds(beside);
    let gap_v = u16::from(!flush);
    let gap_h = if flush { 0 } else { 2 };
    match side {
        Side::Below => Size::new(
            max_w,
            area.bottom()
                .saturating_sub(a.bottom().saturating_add(gap_v)),
        ),
        Side::Above => Size::new(max_w, a.y.saturating_sub(area.y.saturating_add(gap_v))),
        Side::Right => Size::new(
            max_w.min(area.right().saturating_sub(a.right().saturating_add(gap_h))),
            area.h,
        ),
        Side::Left => Size::new(
            max_w.min(a.x.saturating_sub(area.x.saturating_add(gap_h))),
            area.h,
        ),
    }
}

/// Picks a box beside `anchor` (design §2.4) from each side in order, with the size the
/// renderer measured for that side's room: candidates shifted along the side to fit, scored
/// by the text they cover, dimmed cells, the caret and the distance (and, with an arrow, the
/// arrow's route); ties go to the earlier side. Then the sliver rule. `None` if nothing fits.
/// With an arrow, also the chosen box's route when the sliver rule left the box where it was
/// routed. `flush`: a docked box against its chip, touching it (next to it, and sharing at
/// least one cell of its edge), not a cell or more away for an arrow.
fn place(
    sides: &[(Side, (u16, u16))],
    anchor: &[Rect],
    grid: &Grid,
    taken: &Taken,
    agent: bool,
    arrow: bool,
    flush: bool,
) -> Option<(Rect, Side, Option<route::Path>)> {
    let area = grid.area;
    if anchor.is_empty() {
        return None;
    }
    let a = Rect::bounds(anchor);
    let sums = taken.sums();
    // Where along the side: the junction over the anchor's middle, or the box at either edge.
    let mid = a.x as i32 + (a.w.min(16) as i32 - 1) / 2;
    let mut cands: Vec<(u32, usize, usize, Rect, Side, u32)> = Vec::new();
    let mut seq = 0;
    for (si, &(side, (bw, bh))) in sides.iter().enumerate() {
        if bw > area.w || bh > area.h {
            continue;
        }
        let clamp_x = |x: i32| x.clamp(area.x as i32, (area.right() - bw) as i32) as u16;
        let clamp_y = |y: i32| y.clamp(area.y as i32, (area.bottom() - bh) as i32) as u16;
        let xs = [
            mid - 4,
            area.x as i32,
            (area.right() - bw) as i32,
            mid - bw as i32 + 5,
            // Docked: the box's middle on the chip's, so they meet.
            a.x as i32 + a.w as i32 / 2 - bw as i32 / 2,
        ];
        let xs = &xs[..if flush { 5 } else { 4 }];
        let ys = [
            a.y as i32 - 1,
            a.y as i32 - (bh as i32 - 2),
            a.y as i32 - bh as i32 / 2,
        ];
        let mut add = |r: Rect, k: u16| {
            seq += 1;
            if (r.right() > area.right())
                || (r.bottom() > area.bottom())
                || !allowed(&r, grid, taken, anchor, agent)
            {
                return;
            }
            // A docked box shares an edge with its chip.
            if flush
                && match side {
                    Side::Below | Side::Above => !(r.x < a.right() && a.x < r.right()),
                    Side::Right | Side::Left => !(r.y < a.bottom() && a.y < r.bottom()),
                }
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
                let near = u16::from(!flush);
                for k in near..=near + if flush { 0 } else { 3 } {
                    let y = if side == Side::Below {
                        a.bottom() as i32 + k as i32
                    } else {
                        a.y as i32 - k as i32 - bh as i32
                    };
                    if y < area.y as i32 || y + bh as i32 > area.bottom() as i32 {
                        continue;
                    }
                    let mut seen = Vec::new();
                    for &x in xs {
                        let x = clamp_x(x);
                        if !seen.contains(&x) {
                            seen.push(x);
                            add(Rect::new(x, y as u16, bw, bh), k - near);
                        }
                    }
                }
            }
            Side::Right | Side::Left => {
                let near = if flush { 0 } else { 2 };
                for k in near..=near + if flush { 0 } else { 3 } {
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
                            add(Rect::new(x as u16, y, bw, bh), k - near);
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
        // Every box re-scored with the arrow it would need: a box whose arrow must cross words
        // loses to one with a blank way, and any box whose arrow routes beats one whose arrow
        // can't. The winner is the least of them all, whatever order they're routed in, so
        // it doesn't depend on which boxes a first guess ranked together (`route_all`).
        let b = route_all(&cands, anchor, grid, taken)?;
        let r = sliver(b.rect, b.side, sums, grid, taken, anchor, agent);
        return Some((r, b.side, b.path.filter(|_| r == b.rect)));
    }
    let c = cands[0];
    let r = sliver(c.3, c.4, sums, grid, taken, anchor, agent);
    // A docked box the sliver rule moved off its chip stays where it touched it.
    let r = if flush && !touches(&r, &a, c.4) {
        c.3
    } else {
        r
    };
    Some((r, c.4, None))
}

/// Whether a box on `side` of `a` is next to it and shares at least one cell of its edge.
fn touches(r: &Rect, a: &Rect, side: Side) -> bool {
    match side {
        Side::Below => r.y == a.bottom() && r.x < a.right() && a.x < r.right(),
        Side::Above => r.bottom() == a.y && r.x < a.right() && a.x < r.right(),
        Side::Right => r.x == a.right() && r.y < a.bottom() && a.y < r.bottom(),
        Side::Left => r.right() == a.x && r.y < a.bottom() && a.y < r.bottom(),
    }
}

/// A candidate box: (score with a first guess at its arrow, side index, order, box, side,
/// score of the box alone).
type Cand = (u32, usize, usize, Rect, Side, u32);

/// A routed candidate.
struct Scored {
    /// No route: ranks after every box whose arrow routes.
    miss: bool,
    score: u32,
    si: usize,
    seq: usize,
    rect: Rect,
    side: Side,
    path: Option<route::Path>,
}

impl Scored {
    fn key(&self) -> (bool, u32, usize, usize) {
        (self.miss, self.score, self.si, self.seq)
    }
}

/// Routes candidate boxes and returns the least by (no route, score, side, order), whatever
/// order they come in: every box gets a lower bound on its arrow from one search back from
/// the anchor per (side, anchor rect), over the cells any of that pair's routes may use, and
/// they're routed cheapest bound first. A box whose bound can't win isn't routed, and a
/// route's search stops once it can't win.
fn route_all(cands: &[Cand], anchor: &[Rect], grid: &Grid, taken: &Taken) -> Option<Scored> {
    let near: Vec<Rect> = cands.iter().map(|c| nearest(anchor, &c.3)).collect();
    let mut best: Option<Scored> = None;
    let all: Vec<usize> = (0..cands.len()).collect();
    route_group(&all, cands, &near, anchor, grid, taken, &mut best);
    best
}

/// Whether a box whose arrow costs at least `lb` in all could still beat `best`.
fn can_win(lb: u32, c: &Cand, best: &Option<Scored>) -> bool {
    best.as_ref()
        .is_none_or(|b| (false, lb, c.1, c.2) < b.key())
}

/// Routes the candidates `group` (indices into `cands`) into `best` ([`route_all`]).
fn route_group(
    group: &[usize],
    cands: &[Cand],
    near: &[Rect],
    anchor: &[Rect],
    grid: &Grid,
    taken: &Taken,
    best: &mut Option<Scored>,
) {
    if group.is_empty() {
        return;
    }
    let area = grid.area;
    // (side, anchor rect, the region its routes may use), and each box's pair.
    let mut keys: Vec<(Side, Rect, Rect)> = Vec::new();
    let mut key: Vec<usize> = Vec::with_capacity(group.len());
    for &i in group {
        let c = &cands[i];
        let reach = route::reach(c.3, c.4, near[i], area);
        match keys.iter().position(|k| k.0 == c.4 && k.1 == near[i]) {
            Some(k) => {
                keys[k].2 = keys[k].2.union(&reach);
                key.push(k);
            }
            None => {
                key.push(keys.len());
                keys.push((c.4, near[i], reach));
            }
        }
    }
    let open = CostField::new(grid, taken, anchor, Rect::default());
    let goals: Vec<route::ToGoal> = keys
        .iter()
        .map(|&(side, n, region)| route::ToGoal::new(&open, side, n, region))
        .collect();
    // A bound on each arrow's cost (at least one cell when the facing side is out of reach:
    // the arrow may still end on another side).
    let mut few: Vec<(u32, usize, usize)> = group
        .iter()
        .zip(&key)
        .map(|(&i, &k)| {
            let c = &cands[i];
            let field = CostField::new(grid, taken, anchor, c.3);
            let lb = goals[k].bound(&field, c.3, c.4, near[i]).unwrap_or(1) * 5;
            (c.5 + lb, i, k)
        })
        .collect();
    few.sort_by_key(|&(lb, i, _)| (lb, cands[i].1, cands[i].2));
    for (lb, i, k) in few {
        let c = &cands[i];
        if !can_win(lb, c, best) {
            continue;
        }
        // Against a box with a route, a route that would lose costs more than the room left.
        let limit = match best.as_ref() {
            Some(b) if !b.miss => b.score.saturating_sub(c.5) / 5,
            _ => u32::MAX,
        };
        let field = CostField::new(grid, taken, anchor, c.3);
        // Crossing a word is worse than covering one: a box hides text, an arrow mangles it.
        let (miss, rc, path) =
            match route::route(&field, c.3, c.4, near[i], area, limit, Some(&goals[k])) {
                route::Routed::Over => continue,
                route::Routed::NoWay => (true, 0, None),
                route::Routed::Found(p) => (
                    false,
                    p.cost * 5
                        + 300
                            * p.cells
                                .iter()
                                .filter(|c| grid.kind(c.0, c.1) == CellKind::Text)
                                .count() as u32,
                    Some(p),
                ),
            };
        let scored = Scored {
            miss,
            score: c.5 + rc,
            si: c.1,
            seq: c.2,
            rect: c.3,
            side: c.4,
            path,
        };
        if best.as_ref().is_none_or(|b| scored.key() < b.key()) {
            *best = Some(scored);
        }
    }
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
