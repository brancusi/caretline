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

use crate::explain::{Arrow, Candidate, Explained, Explanation, why_won};
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

/// What a renderer measures: the layer's content data, the room it has on one side, and whose
/// layer it is, so a host can size what it draws for the owner (an agent's name in the
/// border, an attribution line) inside the box.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct MeasureCtx<'a> {
    /// The layer's content data.
    pub data: &'a Value,
    /// The most the box may be, borders included.
    pub avail: Size,
    /// Whose layer it is (`Owner::actor` for an agent's name).
    pub owner: &'a Owner,
}

impl<'a> MeasureCtx<'a> {
    pub fn new(data: &'a Value, avail: Size, owner: &'a Owner) -> MeasureCtx<'a> {
        MeasureCtx { data, avail, owner }
    }
}

/// What a host registers per content kind. Placement asks it how big a box is; drawing the box
/// is the host's, outside this crate. `measure` must be pure: the same data, room and owner,
/// the same size.
pub trait Renderer {
    /// The box for `cx.data`, borders included, at most `cx.avail`. A zero size means no box.
    fn measure(&self, cx: &MeasureCtx<'_>) -> Size;
    /// The edge chip shown where an off-screen anchor lies: its width (it is one row high;
    /// default 8). It gets the layer's data, the anchor that lies off screen and which way, so
    /// a host can size a label such as "↓ 2/11 here" to fit. Pure, like `measure`.
    fn chip(&self, _data: &Value, _anchor: &Anchor, _off: Off) -> Size {
        Size::new(8, 1)
    }
}

/// A closure of the data and the room is a renderer that ignores the owner.
impl<F: Fn(&Value, Size) -> Size> Renderer for F {
    fn measure(&self, cx: &MeasureCtx<'_>) -> Size {
        self(cx.data, cx.avail)
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
    /// Every way to the anchor ends with its head on text (a letter, a hyphen inside a word,
    /// or a word gap such as the space between `[ ]` and the word after it): every cell beside
    /// the anchor is text or between words. The arrow is dropped rather than drawn over the
    /// text; the box and the ring still mark the anchor.
    HeadOnText,
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
    /// How many of its box's cells are avoid cells (the host's [`Grid::avoid`] or the layer's
    /// own [`Layer::avoid`]): nonzero only when no box in reach kept off them all, so a host's
    /// lint can flag it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub covers_avoid: u16,
    /// The anchor's cells, if it has a ring.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ring: Vec<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse: Option<Pulse>,
}

fn is_zero(n: &u16) -> bool {
    *n == 0
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
    plan_with(layers, anchors, grid, renderers, &Opts::default(), None)
}

/// [`plan`], and why each box landed where it did: every fallback anchor tried, the size
/// measured for each side, every candidate box weighed (side, cells, text and avoid weight
/// covered, distance, its arrow's cost) and why the winner won ([`Explanation`], with a
/// `Display` for people). The plan is the one [`plan`] gives. Gathering costs a little, so
/// only an inspector or a test asks for it; [`plan`] keeps none of it.
pub fn plan_explained(
    layers: &Layers,
    anchors: &dyn Resolve,
    grid: &Grid,
    renderers: &Renderers,
) -> (Plan, Explanation) {
    let mut e = Explanation {
        width: grid.width,
        height: grid.height,
        layers: Vec::new(),
    };
    let p = plan_with(
        layers,
        anchors,
        grid,
        renderers,
        &Opts::default(),
        Some(&mut e),
    );
    (p, e)
}

/// How a caller other than a host's frame wants a plan made.
#[derive(Default)]
pub(crate) struct Opts<'a> {
    /// This layer's box covers no avoid cell: a candidate that would is never weighed. The
    /// conformance kit re-plans with it to learn whether a clear box was in reach.
    #[cfg_attr(not(feature = "conformance"), allow(dead_code))]
    pub(crate) hard_avoid: Option<&'a str>,
}

pub(crate) fn plan_with(
    layers: &Layers,
    anchors: &dyn Resolve,
    grid: &Grid,
    renderers: &Renderers,
    opts: &Opts<'_>,
    mut explain: Option<&mut Explanation>,
) -> Plan {
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
    let screen = Rect::new(0, 0, grid.width, grid.height);
    for (_, t) in &targets {
        if let Some(Target::At(r)) = t {
            taken.anchors.extend(r.rects.iter().copied());
        }
    }
    for (layer, t) in targets {
        let trace = explain.as_deref_mut().map(|e| {
            e.layers.push(Explained::begin(layer, anchors, grid));
            e.layers.last_mut().expect("just pushed")
        });
        let Some(t) = t else {
            if let Some(tr) = trace {
                tr.why = "missing: none of its anchors resolved".into();
            }
            out.missing.push(layer.id.clone());
            continue;
        };
        let hard = opts.hard_avoid.is_some_and(|id| id == layer.id);
        // The cells this layer keeps off: every avoid anchor that shows.
        let avoid: Vec<Rect> = layer
            .avoid
            .iter()
            .filter_map(|a| anchors.resolve(a))
            .flat_map(|r| r.rects)
            .map(|r| r.intersection(&screen))
            .filter(|r| !r.is_empty())
            .collect();
        plan_one(
            layer, t, &avoid, grid, renderers, &mut taken, &mut out, hard, trace,
        );
    }
    out
}

/// The widest a box may be by default, borders included.
pub const MAX_WIDTH: u16 = 52;

#[allow(clippy::too_many_arguments)]
fn plan_one(
    layer: &Layer,
    t: Target,
    avoid: &[Rect],
    grid: &Grid,
    renderers: &Renderers,
    taken: &mut Taken,
    out: &mut Plan,
    hard: bool,
    mut trace: Option<&mut Explained>,
) {
    let area = grid.area;
    let agent = layer.owner.is_agent();
    let narrow = area.w < NARROW_COLS || area.h < NARROW_ROWS;
    if let Some(tr) = trace.as_deref_mut() {
        tr.narrow = narrow && layer.content.is_some();
    }
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
        covers_avoid: 0,
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

    // Set when the box's arrow could only end on text (`NoArrow::HeadOnText`).
    let mut head_on_text = false;
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
            let m = rend.measure(&MeasureCtx::new(data, avail, &layer.owner));
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
        if let Some(tr) = trace.as_deref_mut() {
            tr.measured = sides
                .iter()
                .map(|&(s, (w, h))| (s, Size::new(w, h)))
                .collect();
        }
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
                (_, Some((chip, _))) => place(
                    &sides,
                    &[chip],
                    avoid,
                    grid,
                    taken,
                    agent,
                    false,
                    true,
                    hard,
                    trace.as_deref_mut(),
                )
                .map(|(r, s, _)| (r, Some(s), None)),
                (Target::At(_), None) => place(
                    &sides,
                    &anchor,
                    avoid,
                    grid,
                    taken,
                    agent,
                    layer.arrow,
                    false,
                    hard,
                    trace.as_deref_mut(),
                )
                .map(|(r, s, path)| (r, Some(s), path)),
                _ => None,
            };
        }
        if let Some(tr) = trace {
            match (&chosen, &t) {
                (Some(_), Target::Screen(pos)) => {
                    tr.why = format!(
                        "at the screen position {}",
                        format!("{pos:?}").to_lowercase()
                    );
                }
                (None, _) if !has_box => {
                    tr.why = "no box: the renderer measured nothing for it".into();
                }
                (None, _) if narrow => {
                    tr.why = format!(
                        "a strip: the area is {}x{}, under {NARROW_COLS} columns or {NARROW_ROWS} rows",
                        area.w, area.h
                    );
                }
                (None, _) if sides.is_empty() && !matches!(t, Target::Screen(_)) => {
                    tr.why = "a strip: no side had room for a box".into();
                }
                (None, _) => {
                    tr.why = "a strip: no box fits beside the anchor, clear of its anchor, \
                              other layers, holes, protected cells and wide graphemes"
                        .into();
                }
                _ => {}
            }
        }
        match chosen {
            Some((r, side, path)) => {
                p.rect = Some(r);
                p.mode = Some(Mode::Box);
                p.side = side;
                p.covers_avoid = avoid_cells(grid, avoid, &r);
                out.regions.push(Region {
                    rect: r,
                    id: layer.id.clone(),
                });
                if let (true, Some(side)) = (routed, side) {
                    // The route placement found for this box, else (the sliver rule moved
                    // it) a fresh one.
                    let to = nearest(&anchor, &r);
                    let path = path.or_else(|| {
                        let field = CostField::new(grid, taken, &anchor, avoid, r);
                        route::route(
                            &field,
                            r,
                            side,
                            to,
                            area,
                            u32::MAX,
                            None,
                            &mut route::Scratch::default(),
                        )
                        .path()
                    });
                    // None: would a way with its head on text have reached it?
                    if path.is_none() {
                        let field = CostField {
                            loose: true,
                            ..CostField::new(grid, taken, &anchor, avoid, r)
                        };
                        head_on_text = route::route(
                            &field,
                            r,
                            side,
                            to,
                            area,
                            u32::MAX,
                            None,
                            &mut route::Scratch::default(),
                        )
                        .path()
                        .is_some();
                    }
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
                let r = strip_rect(&anchor, off.as_ref(), grid, taken, agent);
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
        } else if head_on_text {
            NoArrow::HeadOnText
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

/// The strip's row, on the edge nearest the anchor: the area's top for an anchor that lies
/// above, its bottom for one below; for an anchor on screen (or left or right), the top,
/// unless the anchor is on the top row. Failing that (its chip, another layer, a hole or
/// protected cells there, or for an agent's layer the caret), the nearest free row inward
/// from that edge.
fn strip_rect(anchor: &[Rect], off: Option<&Off>, grid: &Grid, taken: &Taken, agent: bool) -> Rect {
    let area = grid.area;
    let row = |y: u16| Rect::new(area.x, y, area.w, 1);
    let top_first = match off {
        Some(Off::Above { .. }) => true,
        Some(Off::Below { .. }) => false,
        _ => !anchor.iter().any(|a| a.intersects(&row(area.y))),
    };
    let ys: Vec<u16> = if top_first {
        (area.y..area.bottom()).collect()
    } else {
        (area.y..area.bottom()).rev().collect()
    };
    let mut blocked = blockers(grid, taken, anchor, None);
    // An agent's strip keeps off the caret, as its box does.
    if let (true, Some((x, y))) = (agent, grid.caret) {
        blocked.push(Rect::new(x, y, 1, 1));
    }
    if let Some(r) = ys
        .iter()
        .map(|&y| row(y))
        .find(|r| !blocked.iter().any(|b| b.intersects(r)))
    {
        return r;
    }
    // No whole row is free: the widest free run of any row, in the same order (the nearer
    // the edge, the better on a tie), its ends off the halves of a wide grapheme.
    let mut best: Option<Rect> = None;
    for &y in &ys {
        let free = |x: u16| !blocked.iter().any(|b| b.contains(x, y));
        let mut x = area.x;
        while x < area.right() {
            if !free(x) {
                x += 1;
                continue;
            }
            let start = x;
            while x < area.right() && free(x) {
                x += 1;
            }
            let (mut a, mut b) = (start, x);
            if grid.kind(a, y) == CellKind::WideTail {
                a += 1;
            }
            if b < area.right() && grid.kind(b, y) == CellKind::WideTail && b > a {
                b -= 1;
            }
            if b > a && best.is_none_or(|r| b - a > r.w) {
                best = Some(Rect::new(a, y, b - a, 1));
            }
        }
    }
    best.unwrap_or_else(|| row(ys[0]))
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
/// cells boxes and arrows keep off when they can ([`Grid::avoid`]), which cells hold text,
/// and how far from its anchor a box may go to keep clear ([`Grid::with_reach`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub width: u16,
    pub height: u16,
    pub area: Rect,
    pub caret: Option<(u16, u16)>,
    pub protect: Vec<Rect>,
    kinds: Vec<CellKind>,
    /// Each cell's avoid weight (empty while nothing is avoided).
    avoid: Vec<u16>,
    reach: Reach,
}

/// The default weight of an avoid cell ([`Grid::avoid`], [`Layer::avoid`]): what covering it
/// costs a box, in text cells. Covering a text cell costs 1.
pub const AVOID: u16 = 20;

/// How far from its anchor a box may sit to keep off text and avoid cells, past the nearest
/// few places it always tries: `rows` for a box above or below, `cols` for one beside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Reach {
    pub rows: u16,
    pub cols: u16,
}

impl Default for Reach {
    /// 12 rows, 40 columns.
    fn default() -> Reach {
        Reach { rows: 12, cols: 40 }
    }
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
            avoid: Vec::new(),
            reach: Reach::default(),
        }
    }

    /// Asks boxes and arrows to keep off `r`'s cells (a highlighted band, the rows a step talks
    /// about): soft, unlike `protect`. A box covers one only when no box in reach keeps off
    /// them all, and then the fewest and lightest; an arrow crosses one only round nothing
    /// cheaper. `weight` is what covering a cell costs, in text cells ([`AVOID`] is a good
    /// default); overlapping rects keep the heavier. A weight of 0 does nothing.
    pub fn avoid(&mut self, r: Rect, weight: u16) {
        let r = r.intersection(&Rect::new(0, 0, self.width, self.height));
        if r.is_empty() || weight == 0 {
            return;
        }
        if self.avoid.is_empty() {
            self.avoid = vec![0; self.width as usize * self.height as usize];
        }
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let i = y as usize * self.width as usize + x as usize;
                self.avoid[i] = self.avoid[i].max(weight);
            }
        }
    }

    /// [`Grid::avoid`], as a builder.
    pub fn with_avoid(mut self, r: Rect, weight: u16) -> Grid {
        self.avoid(r, weight);
        self
    }

    /// A cell's avoid weight (0: not avoided).
    pub fn avoid_weight(&self, x: u16, y: u16) -> u16 {
        if self.avoid.is_empty() || x >= self.width || y >= self.height {
            return 0;
        }
        self.avoid[y as usize * self.width as usize + x as usize]
    }

    /// How far a box may go from its anchor to keep clear ([`Reach`]; default 12 rows, 40
    /// columns). Farther candidates are tried only for a side none of whose nearer ones is
    /// clear of text and avoid cells, and cost more the farther they are.
    pub fn with_reach(mut self, reach: Reach) -> Grid {
        self.reach = reach;
        self
    }

    pub fn reach(&self) -> Reach {
        self.reach
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
                // An avoid cell costs its weight on top, so arrows go round when they can.
                if let Some(&a) = grid.avoid.get(i)
                    && a > 0
                    && costs[i] != NO_WAY
                {
                    costs[i] += ROUTE_AVOID * u32::from(a);
                }
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
    /// Avoid weights; `None` while the grid avoids nothing.
    avoid: Option<Vec<u32>>,
}

impl Sums {
    fn new(grid: &Grid, taken: &Taken) -> Sums {
        let gw = grid.width as usize;
        let (w, h) = (gw + 1, grid.height as usize + 1);
        // Each row's running count plus the row above's table.
        let table = |cell: &dyn Fn(usize) -> u32| {
            let mut t = vec![0u32; w * h];
            for y in 0..h - 1 {
                let mut run = 0u32;
                let (above, row) = t.split_at_mut((y + 1) * w);
                let above = &above[y * w..];
                for x in 0..gw {
                    run += cell(y * gw + x);
                    row[x + 1] = run + above[x + 1];
                }
            }
            t
        };
        let text = table(&|i| u32::from(grid.kinds[i] != CellKind::Blank));
        let dim = taken
            .any_dim
            .then(|| table(&|i| u32::from(taken.dimmed[i])));
        let avoid = (!grid.avoid.is_empty()).then(|| table(&|i| u32::from(grid.avoid[i])));
        Sums {
            w,
            text,
            dim,
            avoid,
        }
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

    /// The avoid weight a box at `r` covers: the grid's cells, and the layer's own (`avoid`,
    /// at [`AVOID`] each).
    fn avoid(&self, r: &Rect, avoid: &[Rect]) -> u32 {
        let grid = self.avoid.as_ref().map_or(0, |a| Self::sum(a, self.w, r));
        let own: u32 = avoid
            .iter()
            .map(|a| {
                let i = a.intersection(r);
                i.w as u32 * i.h as u32 * AVOID as u32
            })
            .sum();
        grid + own
    }
}

/// What an arrow pays per unit of avoid weight for each cell it crosses, on top of the cell's
/// own cost: at [`AVOID`], 80, five text cells.
const ROUTE_AVOID: u32 = 4;

/// The avoid weight of one cell: the grid's, or the layer's own.
fn avoid_at(grid: &Grid, avoid: &[Rect], x: u16, y: u16) -> u32 {
    let own = if avoid.iter().any(|a| a.contains(x, y)) {
        AVOID
    } else {
        0
    };
    u32::from(grid.avoid_weight(x, y).max(own))
}

/// How many of `r`'s cells are avoid cells.
fn avoid_cells(grid: &Grid, avoid: &[Rect], r: &Rect) -> u16 {
    let mut n = 0u16;
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            if avoid_at(grid, avoid, x, y) > 0 {
                n = n.saturating_add(1);
            }
        }
    }
    n
}

/// The router's view of the screen for one layer and one candidate box: the plan's costs,
/// with the anchor's cells and the box blocked.
struct CostField<'a> {
    costs: &'a [u32],
    grid: &'a Grid,
    anchor: &'a [Rect],
    /// The layer's own avoid cells.
    avoid: &'a [Rect],
    own: Rect,
    /// Any cell it may enter may hold the head (only to tell `head_on_text` from `no_way`).
    loose: bool,
}

impl<'a> CostField<'a> {
    fn new(
        grid: &'a Grid,
        taken: &'a Taken,
        anchor: &'a [Rect],
        avoid: &'a [Rect],
        own: Rect,
    ) -> CostField<'a> {
        CostField {
            costs: taken.costs.as_deref().expect("costs built before routing"),
            grid,
            anchor,
            avoid,
            own,
            loose: false,
        }
    }
}

impl Field for CostField<'_> {
    fn cost(&self, x: u16, y: u16) -> Option<u32> {
        let g = self.grid;
        if x >= g.width || y >= g.height {
            return None;
        }
        let c = self.costs[y as usize * g.width as usize + x as usize];
        if c == NO_WAY || self.own.contains(x, y) || self.anchor.iter().any(|r| r.contains(x, y)) {
            return None;
        }
        // The layer's own avoid cells, where the grid's weight (already in `c`) is lighter.
        if !self.avoid.is_empty() && self.avoid.iter().any(|r| r.contains(x, y)) {
            let gw = g.avoid_weight(x, y);
            if gw < AVOID {
                return Some(c + ROUTE_AVOID * u32::from(AVOID - gw));
            }
        }
        Some(c)
    }

    /// A head goes on a clear cell: blank, with no text beside it on its row but the anchor's
    /// own. Never on a letter or a hyphen inside a word, nor in the gap between two words
    /// (`[ ]▶item`, `is▲quick`), where it reads as part of the text.
    fn head(&self, x: u16, y: u16) -> bool {
        let g = self.grid;
        if self.loose {
            return true;
        }
        if g.kind(x, y) != CellKind::Blank {
            return false;
        }
        let clear = |nx: Option<u16>| {
            nx.is_none_or(|nx| {
                g.kind(nx, y) == CellKind::Blank || self.anchor.iter().any(|r| r.contains(nx, y))
            })
        };
        clear(x.checked_sub(1)) && clear(x.checked_add(1))
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
/// by the text and avoid cells they cover, dimmed cells, the caret and the distance (and,
/// with an arrow, the arrow's route); ties go to the earlier side. Then the sliver rule.
/// `None` if nothing fits. With an arrow, also the chosen box's route when the sliver rule
/// left the box where it was routed. `flush`: a docked box against its chip, touching it
/// (next to it, and sharing at least one cell of its edge), not a cell or more away for an
/// arrow.
///
/// A box covering no avoid cell beats every box that covers one. Each side tries the nearest
/// few places first; a side none of which is clear (of text and avoid cells) keeps going out,
/// up to the grid's [`Reach`], until one is, each step farther costing more.
#[allow(clippy::too_many_arguments)]
fn place(
    sides: &[(Side, (u16, u16))],
    anchor: &[Rect],
    avoid: &[Rect],
    grid: &Grid,
    taken: &Taken,
    agent: bool,
    arrow: bool,
    flush: bool,
    hard: bool,
    trace: Option<&mut Explained>,
) -> Option<(Rect, Side, Option<route::Path>)> {
    let area = grid.area;
    if anchor.is_empty() {
        return None;
    }
    let a = Rect::bounds(anchor);
    let sums = taken.sums();
    let reach = grid.reach;
    // Where along the side: the junction over the anchor's middle, or the box at either edge.
    let mid = a.x as i32 + (a.w.min(16) as i32 - 1) / 2;
    let mut cands: Vec<Cand> = Vec::new();
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
        // Whether a candidate clear of text and avoid cells was found on this side.
        let clear = std::cell::Cell::new(false);
        let mut add = |r: Rect, k: u16, far: bool| {
            seq += 1;
            if (r.right() > area.right()) || (r.bottom() > area.bottom()) {
                return;
            }
            // Past the nearest places, only a box clear of text and avoid cells is worth its
            // distance.
            if far && (sums.text(&r) > 0 || sums.avoid(&r, avoid) > 0) {
                return;
            }
            if !allowed(&r, grid, taken, anchor, agent) {
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
            // Scores are in tenths: text 10, avoid 10 per unit of weight, dimmed 3, caret 500,
            // distance 5 per cell.
            let text = sums.text(&r);
            let covered = sums.avoid(&r, avoid);
            let dim = sums.dim(&r);
            let caret = if grid.caret.is_some_and(|(x, y)| r.contains(x, y)) {
                500
            } else {
                0
            };
            // Kept off avoid cells altogether (the conformance kit's re-plan).
            if hard && covered > 0 {
                return;
            }
            if text == 0 && covered == 0 {
                clear.set(true);
            }
            let score = text * 10 + covered * 10 + dim * 3 + caret + (gap(&r, &a) + k as u32) * 5;
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
            cands.push(Cand {
                guess: score + guess,
                si,
                seq,
                rect: r,
                side,
                score,
                avoid: covered > 0,
                far,
            });
        };
        // The nearest four places always; past them, while nothing on this side is clear, the
        // clear places only.
        let vertical = matches!(side, Side::Below | Side::Above);
        let near: u16 = match (flush, vertical) {
            (true, _) => 0,
            (false, true) => 1,
            (false, false) => 2,
        };
        let last = match (flush, vertical) {
            (true, _) => near,
            (false, true) => near + 3.max(reach.rows),
            (false, false) => near + 3.max(reach.cols),
        };
        for k in near..=last {
            let far = k > near + 3;
            if far && clear.get() {
                break;
            }
            if vertical {
                let y = if side == Side::Below {
                    a.bottom() as i32 + k as i32
                } else {
                    a.y as i32 - k as i32 - bh as i32
                };
                if y < area.y as i32 || y + bh as i32 > area.bottom() as i32 {
                    if far {
                        break;
                    }
                    continue;
                }
                for (j, &x) in xs.iter().enumerate() {
                    let x = clamp_x(x);
                    if !xs[..j].iter().any(|&o| clamp_x(o) == x) {
                        add(Rect::new(x, y as u16, bw, bh), k - near, far);
                    }
                }
            } else {
                let x = if side == Side::Right {
                    a.right() as i32 + k as i32
                } else {
                    a.x as i32 - k as i32 - bw as i32
                };
                if x < area.x as i32 || x + bw as i32 > area.right() as i32 {
                    if far {
                        break;
                    }
                    continue;
                }
                for (j, &y) in ys.iter().enumerate() {
                    let y = clamp_y(y);
                    if !ys[..j].iter().any(|&o| clamp_y(o) == y) {
                        add(Rect::new(x as u16, y, bw, bh), k - near, far);
                    }
                }
            }
        }
    }
    if cands.is_empty() {
        return None;
    }
    cands.sort_by_key(|c| (c.avoid, c.guess, c.si, c.seq));
    let keep = |r: Rect, side: Side| sliver(r, side, sums, grid, taken, anchor, avoid, agent);
    if arrow {
        // Every box re-scored with the arrow it would need: a box whose arrow must cross words
        // loses to one with a blank way, and any box whose arrow routes beats one whose arrow
        // can't. The winner is the least of them all, whatever order they're routed in, so
        // it doesn't depend on which boxes a first guess ranked together (`route_all`).
        let mut arrows = trace.as_ref().map(|_| Vec::new());
        let b = route_all(&cands, anchor, avoid, grid, taken, arrows.as_mut());
        let r = b.as_ref().map(|b| keep(b.rect, b.side));
        if let Some(tr) = trace {
            let winner = b
                .as_ref()
                .and_then(|b| cands.iter().position(|c| c.seq == b.seq));
            explain_cands(tr, &cands, sums, grid, avoid, arrows, winner, r);
        }
        let (b, r) = (b?, r?);
        return Some((r, b.side, b.path.filter(|_| r == b.rect)));
    }
    let c = cands[0];
    let r = keep(c.rect, c.side);
    // A docked box the sliver rule moved off its chip stays where it touched it.
    let r = if flush && !touches(&r, &a, c.side) {
        c.rect
    } else {
        r
    };
    if let Some(tr) = trace {
        explain_cands(tr, &cands, sums, grid, avoid, None, Some(0), Some(r));
    }
    Some((r, c.side, None))
}

/// Records the candidates weighed, best first, with their score's parts, their arrows, the
/// winner and why it won.
#[allow(clippy::too_many_arguments)]
fn explain_cands(
    tr: &mut Explained,
    cands: &[Cand],
    sums: &Sums,
    grid: &Grid,
    avoid: &[Rect],
    arrows: Option<Vec<(usize, Arrow)>>,
    winner: Option<usize>,
    placed: Option<Rect>,
) {
    tr.candidates = cands
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let r = &c.rect;
            let (text, covered, dimmed) = (sums.text(r), sums.avoid(r, avoid), sums.dim(r));
            let caret = grid.caret.is_some_and(|(x, y)| r.contains(x, y));
            let rest = text * 10 + covered * 10 + dimmed * 3 + if caret { 500 } else { 0 };
            Candidate {
                side: c.side,
                rect: c.rect,
                far: c.far,
                text,
                avoid: covered,
                dimmed,
                caret,
                distance: c.score.saturating_sub(rest) / 5,
                score: c.score,
                arrow: arrows.as_ref().map(|a| {
                    a.iter()
                        .find(|(k, _)| *k == i)
                        .map_or(Arrow::Pruned, |(_, x)| *x)
                }),
            }
        })
        .collect();
    tr.winner = winner;
    if let (Some(w), Some(p)) = (winner, placed) {
        if p != cands[w].rect {
            tr.sliver_from = Some(cands[w].rect);
        }
        tr.why = why_won(&tr.candidates, w);
    }
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

/// A candidate box.
#[derive(Debug, Clone, Copy)]
struct Cand {
    /// Its score with a first guess at its arrow.
    guess: u32,
    /// Its side's index in the order tried, and its order among all.
    si: usize,
    seq: usize,
    rect: Rect,
    side: Side,
    /// The score of the box alone.
    score: u32,
    /// It covers avoid cells: it ranks after every box that covers none.
    avoid: bool,
    /// Past the nearest four places on its side (routed after the near ones).
    far: bool,
}

/// A routed candidate.
struct Scored {
    /// Covers avoid cells: ranks after every box that doesn't.
    avoid: bool,
    /// No route: ranks after every box whose arrow routes (that covers as much).
    miss: bool,
    score: u32,
    si: usize,
    seq: usize,
    rect: Rect,
    side: Side,
    path: Option<route::Path>,
}

impl Scored {
    fn key(&self) -> (bool, bool, u32, usize, usize) {
        (self.avoid, self.miss, self.score, self.si, self.seq)
    }
}

/// Routes candidate boxes (sorted best first by box score and a guess at the arrow) and
/// returns the least by (covers avoid cells, no route, score, side, order), whatever order
/// they come in.
///
/// Boxes go by (side, anchor rect, near or far), the near ones and sides an arrow can end on
/// first. An arrow costs at least one blank cell per cell between its box and the anchor, so a
/// box that couldn't beat the best so far even then is left out, and a group with none left is
/// skipped whole. For the rest, one search back from the anchor over the cells their routes may
/// use gives each a closer bound (the search's cells cover each route's corridor, so it never
/// overestimates); they're routed cheapest bound first, a box whose bound can't win isn't
/// routed, and a route's search stops once it can't win.
fn route_all(
    cands: &[Cand],
    anchor: &[Rect],
    avoid: &[Rect],
    grid: &Grid,
    taken: &Taken,
    mut trace: Option<&mut Vec<(usize, Arrow)>>,
) -> Option<Scored> {
    let area = grid.area;
    let near: Vec<Rect> = cands.iter().map(|c| nearest(anchor, &c.rect)).collect();
    // (side, anchor rect, far, its boxes), in order of their first box.
    let mut groups: Vec<(Side, Rect, bool, Vec<usize>)> = Vec::new();
    for (i, c) in cands.iter().enumerate() {
        match groups
            .iter_mut()
            .find(|k| k.0 == c.side && k.1 == near[i] && k.2 == c.far)
        {
            Some(k) => k.3.push(i),
            None => groups.push((c.side, near[i], c.far, vec![i])),
        }
    }
    let open = CostField::new(grid, taken, anchor, avoid, Rect::default());
    // Where a head may go beside each anchor rect, on any side.
    let mut heads: Vec<(Rect, Vec<(u16, u16)>)> = Vec::new();
    for g in &groups {
        if !heads.iter().any(|h| h.0 == g.1) {
            heads.push((g.1, route::heads(&open, g.1)));
        }
    }
    let heads_of = |n: Rect| &heads.iter().find(|h| h.0 == n).expect("found above").1;
    // Near boxes before far ones, and sides an arrow can end on first: a side with no head on
    // it (every cell there is text, say) needs longer arrows round to another, which the boxes
    // routed first usually beat. (The winner doesn't depend on the order; only how much is
    // routed.)
    groups.sort_by_key(|(side, n, far, _)| {
        let no_head = !heads_of(*n)
            .iter()
            .any(|&(x, y)| route::faces(*side, *n, x, y));
        (*far, no_head)
    });
    let mut scratch = route::Scratch::default();
    let mut best: Option<Scored> = None;
    for (side, n, far, boxes) in groups {
        let boxes: Vec<usize> = boxes
            .into_iter()
            .filter(|&i| {
                let c = &cands[i];
                if !can_win(
                    c.score + gap(&c.rect, &n).max(1) * route::BLANK * 5,
                    c,
                    &best,
                ) {
                    return false;
                }
                // A far box's corridor is long: before searching it, the cells to the nearest
                // head (one each, at least) must leave it a chance.
                !far || {
                    let field = CostField::new(grid, taken, anchor, avoid, c.rect);
                    route::heads_bound(&field, c.rect, c.side, n, heads_of(n))
                        .is_none_or(|lb| can_win(c.score + lb * 5, c, &best))
                }
            })
            .collect();
        if boxes.is_empty() {
            continue;
        }
        let region = boxes.iter().fold(Rect::default(), |r, &i| {
            r.union(&route::reach(cands[i].rect, side, n, area))
        });
        let goal = route::ToGoal::new(&open, side, n, region);
        // When the facing side is out of reach, the least way to a head on any side (the
        // arrow may end there), else at least one cell.
        let mut few: Vec<(u32, usize)> = boxes
            .iter()
            .map(|&i| {
                let c = &cands[i];
                let field = CostField::new(grid, taken, anchor, avoid, c.rect);
                let lb = goal
                    .bound(&field, c.rect, c.side, n)
                    .or_else(|| route::heads_bound(&field, c.rect, c.side, n, heads_of(n)));
                (c.score + lb.unwrap_or(1) * 5, i)
            })
            .collect();
        few.sort_by_key(|&(lb, i)| (lb, cands[i].si, cands[i].seq));
        for (lb, i) in few {
            let c = &cands[i];
            if !can_win(lb, c, &best) {
                continue;
            }
            // Against a box with a route (that covers as much), a route that would lose costs
            // more than the room left.
            let limit = match best.as_ref() {
                Some(b) if !b.miss && b.avoid == c.avoid => b.score.saturating_sub(c.score) / 5,
                _ => u32::MAX,
            };
            let field = CostField::new(grid, taken, anchor, avoid, c.rect);
            let routed = route::route(
                &field,
                c.rect,
                c.side,
                n,
                area,
                limit,
                Some(&goal),
                &mut scratch,
            );
            // Crossing a word is worse than covering one: a box hides text, an arrow mangles
            // it. Crossing an avoid cell costs what covering it would, on top.
            let (miss, rc, path) = match routed {
                route::Routed::Over => {
                    if let Some(t) = trace.as_deref_mut() {
                        t.push((i, Arrow::Over));
                    }
                    continue;
                }
                route::Routed::NoWay => (true, 0, None),
                route::Routed::Found(p) => {
                    let (mut words, mut avoided) = (0u32, 0u32);
                    for c in &p.cells {
                        words += u32::from(grid.kind(c.0, c.1) == CellKind::Text);
                        avoided += avoid_at(grid, avoid, c.0, c.1);
                    }
                    (false, p.cost * 5 + 300 * words + 10 * avoided, Some(p))
                }
            };
            if let Some(t) = trace.as_deref_mut() {
                t.push((
                    i,
                    match &path {
                        Some(p) => Arrow::Routed {
                            cost: rc,
                            cells: p.cells.len() as u16,
                        },
                        None => Arrow::NoWay,
                    },
                ));
            }
            let scored = Scored {
                avoid: c.avoid,
                miss,
                score: c.score + rc,
                si: c.si,
                seq: c.seq,
                rect: c.rect,
                side: c.side,
                path,
            };
            if best.as_ref().is_none_or(|b| scored.key() < b.key()) {
                best = Some(scored);
            }
        }
    }
    best
}

/// Whether a box whose arrow costs at least `lb` in all could still beat `best`.
fn can_win(lb: u32, c: &Cand, best: &Option<Scored>) -> bool {
    best.as_ref()
        .is_none_or(|b| (c.avoid, false, lb, c.si, c.seq) < b.key())
}

/// The sliver rule: a narrow gap with text between the box and the area's edge is closed by
/// stretching the box to the edge, or else by moving it there, unless that covers more avoid
/// weight.
#[allow(clippy::too_many_arguments)]
fn sliver(
    r: Rect,
    side: Side,
    sums: &Sums,
    grid: &Grid,
    taken: &Taken,
    anchor: &[Rect],
    avoid: &[Rect],
    agent: bool,
) -> Rect {
    let _ = side;
    let area = grid.area;
    let mut r = r;
    let ok = |n: &Rect, r: &Rect| {
        allowed(n, grid, taken, anchor, agent) && sums.avoid(n, avoid) <= sums.avoid(r, avoid)
    };
    let left = r.x - area.x;
    if left > 0 && left < SLIVER && sums.text(&Rect::new(area.x, r.y, left, r.h)) > 0 {
        let wide = Rect::new(area.x, r.y, r.w + left, r.h);
        let moved = Rect::new(area.x, r.y, r.w, r.h);
        if ok(&wide, &r) {
            r = wide;
        } else if ok(&moved, &r) {
            r = moved;
        }
    }
    let right = area.right() - r.right();
    if right > 0 && right < SLIVER && sums.text(&Rect::new(r.right(), r.y, right, r.h)) > 0 {
        let wide = Rect::new(r.x, r.y, r.w + right, r.h);
        let moved = Rect::new(r.x + right, r.y, r.w, r.h);
        if ok(&wide, &r) {
            r = wide;
        } else if ok(&moved, &r) {
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
