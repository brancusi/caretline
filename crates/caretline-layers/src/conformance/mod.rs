//! The host conformance kit (feature `conformance`): one shared definition of "layers work
//! correctly" that every host runs in its own CI, against its own screens.
//!
//! - [`check_plan`]: the invariants of a plan over a host's [`Scene`] (its layers, resolver,
//!   grid and renderers): what a box, strip or chip may cover, what may overlap, docks, arrow
//!   routes and heads, the area, avoid cells, and that every layer is accounted for.
//! - [`check_determinism`]: the same scene plans the same, through JSON too.
//! - [`check_replay`]: a recorded op log replays to the live layers, and again.
//! - [`check_mapping`] (feature `caretline`): an edit moves only the anchors in its own
//!   document.
//! - [`check_sizes`]: all of the per-scene checks at each size, summarised in a [`Report`]
//!   (strips used, missing and off-screen anchors, arrows dropped and why, avoid cells
//!   covered) whose `Display` is a compact table for a failing CI log.
//! - [`contract`]: canonical requests for the host's own protocol bridge, their replies
//!   checked against [`ops::schema`](crate::ops::schema) and against the host's resolver.
//! - [`snapshot`]: a stable text golden of a plan and the host's drawn frame.
//!
//! Pure: no I/O. A host writes its goldens itself (docs/layers.md, "Testing your
//! integration").

pub mod contract;
mod validate;

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::explain::Explanation;
use crate::geom::Rect;
use crate::model::{Anchor, Layer, LayerOp, Layers, Limits, apply};
use crate::place::{
    Attach, CellKind, Edge, Grid, MAX_WIDTH, MeasureCtx, Mode, NoArrow, Opts, Plan, Planned,
    Renderers, Size, plan_explained, plan_with,
};
use crate::resolve::{Off, Resolve};
use crate::route::Dir;

pub use validate::{KNOWN, check as check_schema, keywords_known};

/// Validates `v` against `$defs/<def>` of [`ops::schema`](crate::ops::schema) (`request`,
/// `reply`, `resolved`, `list`, `error`, `anchor`, `layer`, …). `Err` says where and why.
pub fn validate(def: &str, v: &Value) -> Result<(), String> {
    let root = crate::ops::schema();
    validate::check(
        &root,
        &serde_json::json!({"$ref": format!("#/$defs/{def}")}),
        v,
        def,
    )
}

/// What a host has when it plans a frame: its layers, its resolver (an
/// [`AnchorMap`](crate::AnchorMap), a [`Chain`](crate::Chain), or [`OwnedViews`] for caretline
/// frames built inside a closure), its grid and its renderers.
pub struct Scene<'a> {
    pub layers: Layers,
    pub anchors: Box<dyn Resolve + 'a>,
    pub grid: Grid,
    pub renderers: &'a Renderers,
}

impl<'a> Scene<'a> {
    pub fn new(
        layers: Layers,
        anchors: impl Resolve + 'a,
        grid: Grid,
        renderers: &'a Renderers,
    ) -> Scene<'a> {
        Scene {
            layers,
            anchors: Box::new(anchors),
            grid,
            renderers,
        }
    }

    /// The scene's plan, as the host's frame makes it.
    pub fn plan(&self) -> Plan {
        crate::plan(&self.layers, &*self.anchors, &self.grid, self.renderers)
    }

    /// The plan and why each box landed where it did ([`crate::plan_explained`]).
    pub fn plan_explained(&self) -> (Plan, Explanation) {
        plan_explained(&self.layers, &*self.anchors, &self.grid, self.renderers)
    }
}

/// What a check found wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Kind {
    /// A box, strip or chip covers its own anchor or another layer's.
    CoversAnchor,
    /// A box, strip or chip covers a protected rect.
    CoversProtected,
    /// An agent's box or strip covers the caret.
    CoversCaret,
    /// Two boxes, strips or chips overlap.
    Overlap,
    /// Two edge chips overlap.
    ChipOverlap,
    /// A box, strip or chip edge falls between the halves of a wide grapheme.
    SplitsWide,
    /// Something extends outside the grid's area.
    OutsideArea,
    /// A docked box doesn't touch its chip, or `dock` names no cell they share.
    DockApart,
    /// An arrow's cells aren't 4-connected, or their directions don't follow its cells.
    RouteBroken,
    /// An arrow doesn't start at `Route.attach` on its box's edge facing the anchor.
    RouteStart,
    /// An arrow's head isn't next to its anchor, pointing at it.
    RouteEnd,
    /// An arrow crosses a box, strip or chip, a protected rect, another arrow, its own anchor
    /// or half of a wide grapheme.
    RouteCrosses,
    /// An arrow's head is on text or in a gap between words.
    HeadOnText,
    /// `no_arrow` is missing, set without cause, or gives the wrong reason.
    NoArrowReason,
    /// A box covers avoid cells though a clear box was in reach, or `covers_avoid` miscounts.
    AvoidCovered,
    /// A layer is placed twice, or neither placed, missing nor unrendered, or has no box though
    /// its renderer measures one.
    Dropped,
    /// A layer is reported missing though one of its anchors resolves.
    Missing,
    /// `unrendered` disagrees with the renderers registered.
    Unrendered,
    /// A box, strip or chip has no click region, or one with the wrong id.
    Region,
    /// The same scene planned differently (twice, or explained).
    Nondeterministic,
    /// The layers or the plan changed through JSON.
    RoundTrip,
    /// A recorded op log doesn't replay to the live layers, or replays differently twice.
    Replay,
    /// An edit moved an anchor in another document, or failed to move one in its own.
    Mapping,
    /// A reply doesn't match `ops::schema()`.
    Schema,
    /// A reply's meaning is wrong: where it says an anchor resolved, what it popped, a
    /// refusal's reason.
    Contract,
}

impl Kind {
    fn name(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    }
}

/// One thing a check found wrong: the layer (if one), the kind and a sentence with the cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    pub kind: Kind,
    pub detail: String,
}

impl Violation {
    pub fn new(layer: Option<&str>, kind: Kind, detail: impl Into<String>) -> Violation {
        Violation {
            layer: layer.map(String::from),
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:<10} {:<17} {}",
            self.layer.as_deref().unwrap_or("-"),
            self.kind.name(),
            self.detail
        )
    }
}

fn r(r: &Rect) -> String {
    format!("({},{} {}x{})", r.x, r.y, r.w, r.h)
}

/// What a layer's part is called in a violation.
fn part(p: &Planned, chip: bool) -> &'static str {
    match (chip, p.mode) {
        (true, _) => "chip",
        (_, Some(Mode::Strip)) => "strip",
        _ => "box",
    }
}

fn step(d: Dir) -> (i32, i32) {
    match d {
        Dir::Up => (0, -1),
        Dir::Down => (0, 1),
        Dir::Left => (-1, 0),
        Dir::Right => (1, 0),
    }
}

fn within(inner: &Rect, outer: &Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.right() <= outer.right()
        && inner.bottom() <= outer.bottom()
}

/// The invariants of `plan` over `scene` (the plan the scene's inputs gave). Each is a
/// [`Violation`]; none is a pass.
///
/// - A box, strip or chip never covers its own anchor, another layer's, a protected rect or
///   (an agent's box or strip) the caret, never splits a wide grapheme, and stays inside the
///   grid's area. Boxes, strips and chips never overlap each other.
/// - A docked box touches its chip, and `dock` names a cell of the edge they share.
/// - An arrow's cells are 4-connected with directions that follow them; it starts at
///   `Route.attach` on its box's edge facing the anchor, ends next to the anchor pointing at
///   it, and never crosses a box, strip or chip, a protected rect, another arrow, its own
///   anchor or a wide grapheme. Its head is on a clear cell: blank, with no text beside it on
///   its row but the anchor's. A layer that asked for an arrow and has none says why
///   (`no_arrow`), with the right reason.
/// - `covers_avoid` counts the avoid cells under the box, and is 0 whenever the bounded
///   search had a box clear of them (checked by re-planning with that layer's avoid cells
///   hard).
/// - Every layer is placed, missing (none of its anchors resolves) or unrendered (no renderer
///   for its kind), once; one with a renderer that measures a box has a box or strip unless
///   it hides off screen. Every box, strip and chip has its click region.
pub fn check_plan(plan: &Plan, scene: &Scene<'_>) -> Vec<Violation> {
    let mut out = Vec::new();
    let grid = &scene.grid;
    let area = grid.area;
    let mut bad = |l: &str, k: Kind, d: String| out.push(Violation::new(Some(l), k, d));
    if scene.layers.hidden || area.is_empty() {
        for p in &plan.layers {
            bad(
                &p.id,
                Kind::Dropped,
                "placed though the layers are hidden or the area is empty".into(),
            );
        }
        return out;
    }
    let anchor_of = |p: &Planned| -> Vec<Rect> {
        p.anchor
            .as_ref()
            .map(|a| a.rects.clone())
            .unwrap_or_default()
    };
    // Every box, strip and chip: (layer, is a chip, rect).
    let mut panels: Vec<(&Planned, bool, Rect)> = Vec::new();
    for p in &plan.layers {
        if let Some(x) = p.rect {
            panels.push((p, false, x));
        }
        if let Some(c) = p.chip {
            panels.push((p, true, c));
        }
    }
    for &(p, chip, x) in &panels {
        let what = part(p, chip);
        if !within(&x, &area) {
            bad(
                &p.id,
                Kind::OutsideArea,
                format!("{what} {} leaves the area {}", r(&x), r(&area)),
            );
        }
        if grid.splits(&x) {
            bad(
                &p.id,
                Kind::SplitsWide,
                format!("{what} {} splits a wide grapheme", r(&x)),
            );
        }
        for q in &plan.layers {
            for a in anchor_of(q) {
                if x.intersects(&a) {
                    let whose = if q.id == p.id {
                        "its own anchor".to_string()
                    } else {
                        format!("{}'s anchor", q.id)
                    };
                    bad(
                        &p.id,
                        Kind::CoversAnchor,
                        format!("{what} {} covers {whose} {}", r(&x), r(&a)),
                    );
                }
            }
        }
        for pr in &grid.protect {
            if x.intersects(pr) {
                bad(
                    &p.id,
                    Kind::CoversProtected,
                    format!("{what} {} covers the protected {}", r(&x), r(pr)),
                );
            }
        }
        if let (true, false, Some((cx, cy))) = (p.agent, chip, grid.caret)
            && x.contains(cx, cy)
        {
            bad(
                &p.id,
                Kind::CoversCaret,
                format!("an agent's {what} {} covers the caret ({cx},{cy})", r(&x)),
            );
        }
    }
    for (i, &(p, pc, a)) in panels.iter().enumerate() {
        for &(q, qc, b) in &panels[i + 1..] {
            if a.intersects(&b) {
                let k = if pc && qc {
                    Kind::ChipOverlap
                } else {
                    Kind::Overlap
                };
                bad(
                    &q.id,
                    k,
                    format!(
                        "{} {} overlaps {}'s {} {}",
                        part(q, qc),
                        r(&b),
                        p.id,
                        part(p, pc),
                        r(&a)
                    ),
                );
            }
        }
    }
    // Every arrow's cells, by layer.
    let routes: Vec<(&str, (u16, u16))> = plan
        .layers
        .iter()
        .flat_map(|p| {
            p.route
                .iter()
                .flat_map(|rt| rt.steps.iter().map(|s| (p.id.as_str(), (s.x, s.y))))
        })
        .collect();
    for p in &plan.layers {
        let layer = scene.layers.get(&p.id);
        dock(p, &mut out);
        route(p, &panels, &routes, grid, &mut out);
        no_arrow(p, layer, &mut out);
        regions(p, plan, &mut out);
    }
    avoid(plan, scene, &mut out);
    accounted(plan, scene, &mut out);
    out
}

fn dock(p: &Planned, out: &mut Vec<Violation>) {
    let mut bad = |d: String| out.push(Violation::new(Some(&p.id), Kind::DockApart, d));
    let (Some(x), Some(Mode::Box)) = (p.rect, p.mode) else {
        if p.dock.is_some() {
            bad("a dock with no box".into());
        }
        return;
    };
    let Some(chip) = p.chip else {
        if p.dock.is_some() {
            bad("a dock with no chip".into());
        }
        return;
    };
    let Some(Attach { edge, offset }) = p.dock else {
        bad(format!(
            "box {} beside chip {} has no dock",
            r(&x),
            r(&chip)
        ));
        return;
    };
    // The box's border cell the dock names, and the cell across from it.
    let (cell, across) = match edge {
        Edge::Top => (
            (x.x + offset, x.y),
            (x.x as i32 + offset as i32, x.y as i32 - 1),
        ),
        Edge::Bottom => (
            (x.x + offset, x.bottom() - 1),
            (x.x as i32 + offset as i32, x.bottom() as i32),
        ),
        Edge::Left => (
            (x.x, x.y + offset),
            (x.x as i32 - 1, x.y as i32 + offset as i32),
        ),
        Edge::Right => (
            (x.right() - 1, x.y + offset),
            (x.right() as i32, x.y as i32 + offset as i32),
        ),
    };
    let on_chip = across.0 >= 0 && across.1 >= 0 && chip.contains(across.0 as u16, across.1 as u16);
    if !x.contains(cell.0, cell.1) || !on_chip {
        bad(format!(
            "box {} and chip {} don't meet at the dock ({:?} {offset})",
            r(&x),
            r(&chip),
            edge
        ));
    }
}

fn route(
    p: &Planned,
    panels: &[(&Planned, bool, Rect)],
    routes: &[(&str, (u16, u16))],
    grid: &Grid,
    out: &mut Vec<Violation>,
) {
    let Some(rt) = &p.route else { return };
    let mut bad = |k: Kind, d: String| out.push(Violation::new(Some(&p.id), k, d));
    let Some(first) = rt.steps.first() else {
        bad(Kind::RouteBroken, "an arrow with no cells".into());
        return;
    };
    let anchor: Vec<Rect> = p
        .anchor
        .as_ref()
        .map(|a| a.rects.clone())
        .unwrap_or_default();
    // Connected, with directions that follow the cells.
    for w in rt.steps.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let (dx, dy) = (b.x as i32 - a.x as i32, b.y as i32 - a.y as i32);
        if dx.abs() + dy.abs() != 1 || step(a.leave) != (dx, dy) || a.leave != b.enter {
            bad(
                Kind::RouteBroken,
                format!(
                    "({},{}) leaving {:?} isn't followed by ({},{}) entered {:?}",
                    a.x, a.y, a.leave, b.x, b.y, b.enter
                ),
            );
            break;
        }
    }
    // Starts at its attach, on the box's edge facing the anchor.
    if let Some(x) = p.rect {
        let want = match p.side {
            Some(crate::Side::Below) => Some(Edge::Top),
            Some(crate::Side::Above) => Some(Edge::Bottom),
            Some(crate::Side::Right) => Some(Edge::Left),
            Some(crate::Side::Left) => Some(Edge::Right),
            None => None,
        };
        let (jx, jy) = rt.junction;
        let at = match rt.attach.edge {
            Edge::Top => (x.x + rt.attach.offset, x.y),
            Edge::Bottom => (x.x + rt.attach.offset, x.bottom() - 1),
            Edge::Left => (x.x, x.y + rt.attach.offset),
            Edge::Right => (x.right() - 1, x.y + rt.attach.offset),
        };
        let (dx, dy) = step(first.enter);
        if want != Some(rt.attach.edge)
            || at != (jx, jy)
            || (jx as i32 + dx, jy as i32 + dy) != (first.x as i32, first.y as i32)
        {
            bad(
                Kind::RouteStart,
                format!(
                    "the arrow's first cell ({},{}) doesn't leave box {} at its attach ({:?} {}, junction ({jx},{jy}))",
                    first.x,
                    first.y,
                    r(&x),
                    rt.attach.edge,
                    rt.attach.offset
                ),
            );
        }
    } else {
        bad(Kind::RouteStart, "an arrow with no box".into());
    }
    // Ends next to the anchor, pointing at it.
    let head = rt.steps.last().expect("not empty");
    let (dx, dy) = step(head.leave);
    let (nx, ny) = (head.x as i32 + dx, head.y as i32 + dy);
    let points = nx >= 0
        && ny >= 0
        && anchor.iter().any(|a| a.contains(nx as u16, ny as u16))
        && head.enter == head.leave;
    if !points {
        bad(
            Kind::RouteEnd,
            format!(
                "the head ({},{}) pointing {:?} isn't beside its anchor",
                head.x, head.y, head.leave
            ),
        );
    }
    // Crosses nothing it may not.
    for s in &rt.steps {
        let c = (s.x, s.y);
        let what = if !grid.area.contains(s.x, s.y) {
            Some((Kind::OutsideArea, "leaves the area".to_string()))
        } else if let Some((q, chip, b)) = panels.iter().find(|(_, _, b)| b.contains(c.0, c.1)) {
            Some((
                Kind::RouteCrosses,
                format!("crosses {}'s {} {}", q.id, part(q, *chip), r(b)),
            ))
        } else if let Some(pr) = grid.protect.iter().find(|b| b.contains(c.0, c.1)) {
            Some((
                Kind::RouteCrosses,
                format!("crosses the protected {}", r(pr)),
            ))
        } else if matches!(grid.kind(c.0, c.1), CellKind::Wide | CellKind::WideTail) {
            Some((Kind::RouteCrosses, "crosses a wide grapheme".into()))
        } else if anchor.iter().any(|a| a.contains(c.0, c.1)) {
            Some((Kind::RouteCrosses, "crosses its own anchor".into()))
        } else {
            routes
                .iter()
                .find(|(id, rc)| *id != p.id && *rc == c)
                .map(|(id, _)| (Kind::RouteCrosses, format!("crosses {id}'s arrow")))
        };
        if let Some((k, d)) = what {
            bad(k, format!("the arrow at ({},{}) {d}", c.0, c.1));
        }
    }
    // The head on a clear cell.
    let blank = |x: Option<u16>| {
        x.is_none_or(|x| {
            grid.kind(x, head.y) == CellKind::Blank || anchor.iter().any(|a| a.contains(x, head.y))
        })
    };
    if grid.kind(head.x, head.y) != CellKind::Blank {
        bad(
            Kind::HeadOnText,
            format!("the head ({},{}) is on text", head.x, head.y),
        );
    } else if !blank(head.x.checked_sub(1)) || !blank(head.x.checked_add(1)) {
        bad(
            Kind::HeadOnText,
            format!("the head ({},{}) is in a gap between words", head.x, head.y),
        );
    }
}

fn no_arrow(p: &Planned, layer: Option<&Layer>, out: &mut Vec<Violation>) {
    let Some(layer) = layer else { return };
    let mut bad = |d: String| out.push(Violation::new(Some(&p.id), Kind::NoArrowReason, d));
    if !layer.arrow {
        if p.route.is_some() || p.no_arrow.is_some() {
            bad("an arrow or a reason for none, on a layer that asked for no arrow".into());
        }
        return;
    }
    match (&p.route, p.no_arrow) {
        (Some(_), Some(n)) => bad(format!("an arrow, and no_arrow {n:?}")),
        (Some(_), None) => {}
        (None, None) => bad("asked for an arrow, has none, and no_arrow doesn't say why".into()),
        (None, Some(n)) => {
            let off = p.anchor.as_ref().is_some_and(|a| a.rects.is_empty());
            let want = if p.anchor.is_none() {
                Some(NoArrow::Screen)
            } else if p.mode != Some(Mode::Box) {
                Some(NoArrow::NoBox)
            } else if off {
                Some(NoArrow::Docked)
            } else {
                None
            };
            let ok = match want {
                Some(w) => n == w,
                None => matches!(n, NoArrow::NoWay | NoArrow::HeadOnText),
            };
            if !ok {
                bad(format!(
                    "no_arrow is {n:?}, but the layer {}",
                    match want {
                        Some(NoArrow::Screen) => "is at a screen position",
                        Some(NoArrow::NoBox) => "has no box",
                        Some(NoArrow::Docked) => "is docked",
                        _ => "has a box at an anchor on screen (no_way or head_on_text)",
                    }
                ));
            }
        }
    }
}

fn regions(p: &Planned, plan: &Plan, out: &mut Vec<Violation>) {
    let has = |id: &str, x: Rect| plan.regions.iter().any(|g| g.id == id && g.rect == x);
    if let Some(x) = p.rect
        && !has(&p.id, x)
    {
        out.push(Violation::new(
            Some(&p.id),
            Kind::Region,
            format!("no region {:?} at {}", p.id, r(&x)),
        ));
    }
    if let Some(c) = p.chip {
        let id = format!("{}/reveal", p.id);
        if !has(&id, c) {
            out.push(Violation::new(
                Some(&p.id),
                Kind::Region,
                format!("no region {id:?} at {}", r(&c)),
            ));
        }
    }
}

/// `covers_avoid` counts the avoid cells under the box, and a box covers any only when the
/// bounded search had no clear one: re-planned with that layer's avoid cells hard, it has no
/// box beside its anchor.
fn avoid(plan: &Plan, scene: &Scene<'_>, out: &mut Vec<Violation>) {
    let grid = &scene.grid;
    let screen = Rect::new(0, 0, grid.width, grid.height);
    for p in &plan.layers {
        let (Some(x), Some(Mode::Box)) = (p.rect, p.mode) else {
            if p.covers_avoid > 0 {
                out.push(Violation::new(
                    Some(&p.id),
                    Kind::AvoidCovered,
                    format!("covers_avoid {} on a layer with no box", p.covers_avoid),
                ));
            }
            continue;
        };
        let own: Vec<Rect> = scene
            .layers
            .get(&p.id)
            .map(|l| {
                l.avoid
                    .iter()
                    .filter_map(|a| scene.anchors.resolve(a))
                    .flat_map(|r| r.rects)
                    .map(|r| r.intersection(&screen))
                    .filter(|r| !r.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let mut n = 0u32;
        for y in x.y..x.bottom() {
            for cx in x.x..x.right() {
                if grid.avoid_weight(cx, y) > 0 || own.iter().any(|a| a.contains(cx, y)) {
                    n += 1;
                }
            }
        }
        if n.min(u16::MAX as u32) as u16 != p.covers_avoid {
            out.push(Violation::new(
                Some(&p.id),
                Kind::AvoidCovered,
                format!(
                    "box {} covers {n} avoid cells, covers_avoid says {}",
                    r(&x),
                    p.covers_avoid
                ),
            ));
        }
        // A screen box has one place: nothing to choose from.
        if p.covers_avoid == 0 || p.side.is_none() {
            continue;
        }
        let hard = plan_with(
            &scene.layers,
            &*scene.anchors,
            grid,
            scene.renderers,
            &Opts {
                hard_avoid: Some(&p.id),
            },
            None,
        );
        if let Some(q) = hard.layers.iter().find(|q| q.id == p.id)
            && q.mode == Some(Mode::Box)
            && let Some(c) = q.rect
        {
            out.push(Violation::new(
                Some(&p.id),
                Kind::AvoidCovered,
                format!(
                    "box {} covers {} avoid cells, but {} ({:?}) in reach covers none",
                    r(&x),
                    p.covers_avoid,
                    r(&c),
                    q.side
                ),
            ));
        }
    }
}

/// Every layer is placed, missing or unrendered, once, for the right reason.
fn accounted(plan: &Plan, scene: &Scene<'_>, out: &mut Vec<Violation>) {
    let grid = &scene.grid;
    let screen = Rect::new(0, 0, grid.width, grid.height);
    let area = grid.area;
    for l in &scene.layers.layers {
        let mut bad = |k: Kind, d: String| out.push(Violation::new(Some(&l.id), k, d));
        let placed = plan.layers.iter().filter(|p| p.id == l.id).count();
        let missing = plan.missing.iter().filter(|m| **m == l.id).count();
        if placed + missing != 1 {
            bad(
                Kind::Dropped,
                format!("placed {placed} times and missing {missing} times"),
            );
            continue;
        }
        // Whether a fallback resolves, and which way the first that does lies.
        let mut resolves = None;
        for a in &l.anchor {
            if let Anchor::Screen(_) = a {
                resolves = Some((a, None));
                break;
            }
            if let Some(res) = scene.anchors.resolve(a) {
                let shows = res
                    .rects
                    .iter()
                    .any(|x| !x.intersection(&screen).is_empty());
                if shows || res.off.is_some() {
                    resolves = Some((a, if shows { None } else { res.off }));
                    break;
                }
            }
        }
        if missing == 1 {
            if let Some((a, _)) = resolves {
                bad(
                    Kind::Missing,
                    format!(
                        "reported missing, but {} resolves",
                        serde_json::to_string(a).unwrap_or_default()
                    ),
                );
            }
            continue;
        }
        let p = plan
            .layers
            .iter()
            .find(|p| p.id == l.id)
            .expect("placed once");
        let unrendered = plan.unrendered.contains(&l.id);
        let Some(content) = &l.content else {
            if unrendered {
                bad(Kind::Unrendered, "unrendered, but it has no content".into());
            }
            continue;
        };
        let Some(rend) = scene.renderers.get(&content.kind) else {
            if !unrendered {
                bad(
                    Kind::Unrendered,
                    format!("no renderer for {:?}, but not unrendered", content.kind),
                );
            }
            continue;
        };
        if unrendered {
            bad(
                Kind::Unrendered,
                format!("unrendered, but {:?} has a renderer", content.kind),
            );
        }
        let off: Option<Off> = resolves.and_then(|(_, o)| o);
        if off.is_some() && l.hide_off_screen {
            continue;
        }
        let max_w = l.max_width.unwrap_or(MAX_WIDTH).min(area.w * 2 / 3);
        let m = rend.measure(&MeasureCtx::new(
            &content.data,
            Size::new(max_w, area.h),
            &l.owner,
        ));
        if m.w > 0 && m.h > 0 && max_w > 0 && p.rect.is_none() {
            bad(
                Kind::Dropped,
                format!(
                    "its renderer measures {}x{}, but it has no box or strip",
                    m.w, m.h
                ),
            );
        }
    }
    for p in &plan.layers {
        if scene.layers.get(&p.id).is_none() {
            out.push(Violation::new(
                Some(&p.id),
                Kind::Dropped,
                "placed, but not in the layers",
            ));
        }
    }
}

/// The same scene plans the same: twice, through [`plan_explained`](crate::plan_explained),
/// with its layers round-tripped through JSON; and the plan survives JSON.
pub fn check_determinism(scene: &Scene<'_>) -> Vec<Violation> {
    let mut out = Vec::new();
    let a = scene.plan();
    let b = scene.plan();
    let json = |p: &Plan| serde_json::to_string(p).unwrap_or_default();
    if json(&a) != json(&b) {
        out.push(Violation::new(
            None,
            Kind::Nondeterministic,
            "planned twice, the plans differ",
        ));
    }
    let (e, _) = scene.plan_explained();
    if e != a {
        out.push(Violation::new(
            None,
            Kind::Nondeterministic,
            "plan_explained's plan differs from plan's",
        ));
    }
    match serde_json::to_string(&scene.layers).and_then(|s| serde_json::from_str::<Layers>(&s)) {
        Ok(back) => {
            if back != scene.layers {
                out.push(Violation::new(
                    None,
                    Kind::RoundTrip,
                    "the layers changed through JSON",
                ));
            }
            let p = crate::plan(&back, &*scene.anchors, &scene.grid, scene.renderers);
            if json(&p) != json(&a) {
                out.push(Violation::new(
                    None,
                    Kind::RoundTrip,
                    "the layers through JSON plan differently",
                ));
            }
        }
        Err(e) => out.push(Violation::new(
            None,
            Kind::RoundTrip,
            format!("the layers don't round-trip through JSON: {e}"),
        )),
    }
    match serde_json::from_str::<Plan>(&json(&a)) {
        Ok(p) if p == a => {}
        Ok(_) => out.push(Violation::new(
            None,
            Kind::RoundTrip,
            "the plan changed through JSON",
        )),
        Err(e) => out.push(Violation::new(
            None,
            Kind::RoundTrip,
            format!("the plan doesn't round-trip through JSON: {e}"),
        )),
    }
    out
}

/// Replays `ops` (each op, its actor and `now_ms`, as the host recorded them) on fresh
/// layers with the host's `limits`: the result is `live`, the same again, and the same with
/// every op round-tripped through JSON. A host that also calls `expire` or `observe` records
/// what they changed, or checks replay where they changed nothing.
pub fn check_replay(
    live: &Layers,
    ops: &[(LayerOp, Option<&str>, u64)],
    limits: &Limits,
) -> Vec<Violation> {
    let mut out = Vec::new();
    let run = |ops: &[(LayerOp, Option<&str>, u64)]| {
        let mut l = Layers::default();
        for (op, actor, now) in ops {
            let _ = apply(&mut l, op.clone(), *actor, *now, limits);
        }
        l
    };
    let first = run(ops);
    if first != *live {
        out.push(Violation::new(
            None,
            Kind::Replay,
            format!("the log replays to other layers: {}", diff(live, &first)),
        ));
    }
    if run(ops) != first {
        out.push(Violation::new(
            None,
            Kind::Replay,
            "the log replays differently the second time",
        ));
    }
    let mut back = Vec::new();
    for (i, (op, actor, now)) in ops.iter().enumerate() {
        match serde_json::to_string(op).and_then(|s| serde_json::from_str::<LayerOp>(&s)) {
            Ok(o) => {
                if o != *op {
                    out.push(Violation::new(
                        None,
                        Kind::RoundTrip,
                        format!("op {i} changed through JSON"),
                    ));
                }
                back.push((o, *actor, *now));
            }
            Err(e) => out.push(Violation::new(
                None,
                Kind::RoundTrip,
                format!("op {i} doesn't round-trip through JSON: {e}"),
            )),
        }
    }
    if back.len() == ops.len() && run(&back) != first {
        out.push(Violation::new(
            None,
            Kind::Replay,
            "the log through JSON replays differently",
        ));
    }
    out
}

/// The first difference between two layer sets, in words.
fn diff(want: &Layers, got: &Layers) -> String {
    for l in &want.layers {
        match got.get(&l.id) {
            None => return format!("{} is missing", l.id),
            Some(g) if g != l => return format!("{} differs", l.id),
            _ => {}
        }
    }
    for l in &got.layers {
        if want.get(&l.id).is_none() {
            return format!("{} is extra", l.id);
        }
    }
    if want.hidden != got.hidden {
        return "hidden differs".into();
    }
    if want
        .layers
        .iter()
        .map(|l| &l.id)
        .ne(got.layers.iter().map(|l| &l.id))
    {
        return "the order differs".into();
    }
    "next or the rate record differs".into()
}

/// One size's results ([`check_sizes`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeReport {
    pub size: Size,
    /// Layers planned (placed or missing).
    pub layers: usize,
    pub violations: Vec<Violation>,
    /// Layers drawn as a strip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strips: Vec<String>,
    /// Layers none of whose anchors resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
    /// Layers whose anchor lies off screen (an edge chip).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub off_screen: Vec<String>,
    /// Layers that asked for an arrow and have none, and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub no_arrow: Vec<(String, NoArrow)>,
    /// Layers whose box covers avoid cells, and how many.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers_avoid: Vec<(String, u16)>,
}

/// [`check_sizes`]' results, per size. `Display` prints a compact table, then every
/// violation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Report {
    pub sizes: Vec<SizeReport>,
}

impl Report {
    /// No violation at any size.
    pub fn ok(&self) -> bool {
        self.sizes.iter().all(|s| s.violations.is_empty())
    }

    /// Every violation, with its size.
    pub fn violations(&self) -> impl Iterator<Item = (Size, &Violation)> {
        self.sizes
            .iter()
            .flat_map(|s| s.violations.iter().map(move |v| (s.size, v)))
    }
}

fn list(v: &[String]) -> String {
    if v.is_empty() {
        "-".into()
    } else {
        v.join(" ")
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{:<9} {:>6} {:>10}  {:<12} {:<12} {:<12} {:<22} avoid covered",
            "size", "layers", "violations", "strips", "missing", "off screen", "arrows dropped"
        )?;
        for s in &self.sizes {
            let dropped: Vec<String> = s
                .no_arrow
                .iter()
                .map(|(id, n)| {
                    let why = serde_json::to_value(n)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    format!("{id}:{why}")
                })
                .collect();
            let covered: Vec<String> = s
                .covers_avoid
                .iter()
                .map(|(id, n)| format!("{id}:{n}"))
                .collect();
            writeln!(
                f,
                "{:<9} {:>6} {:>10}  {:<12} {:<12} {:<12} {:<22} {}",
                format!("{}x{}", s.size.w, s.size.h),
                s.layers,
                s.violations.len(),
                list(&s.strips),
                list(&s.missing),
                list(&s.off_screen),
                list(&dropped),
                list(&covered)
            )?;
        }
        if !self.ok() {
            writeln!(f, "violations:")?;
            for (size, v) in self.violations() {
                writeln!(f, "  {:<9} {v}", format!("{}x{}", size.w, size.h))?;
            }
        }
        Ok(())
    }
}

/// Runs [`check_plan`] and [`check_determinism`] on the scene the host builds at each size
/// (lint at the sizes people use: 140×40, 100×30, 80×24, and a narrow one), and summarises
/// each plan: strips, missing and off-screen anchors, arrows dropped and why, avoid cells
/// covered.
///
/// ```ignore
/// let renderers = Renderers::new().register(HINT, MyHint);
/// let report = check_sizes(&|size| my_host.scene(size, &renderers), &[Size::new(80, 24)]);
/// assert!(report.ok(), "{report}");
/// ```
pub fn check_sizes<'a>(scene: &dyn Fn(Size) -> Scene<'a>, sizes: &[Size]) -> Report {
    let mut report = Report::default();
    for &size in sizes {
        let s = scene(size);
        let p = s.plan();
        let mut violations = check_plan(&p, &s);
        violations.extend(check_determinism(&s));
        report.sizes.push(SizeReport {
            size,
            layers: p.layers.len() + p.missing.len(),
            violations,
            strips: p
                .layers
                .iter()
                .filter(|l| l.mode == Some(Mode::Strip))
                .map(|l| l.id.clone())
                .collect(),
            missing: p.missing.clone(),
            off_screen: p
                .layers
                .iter()
                .filter(|l| l.anchor.as_ref().is_some_and(|a| a.off.is_some()))
                .map(|l| l.id.clone())
                .collect(),
            no_arrow: p
                .layers
                .iter()
                .filter_map(|l| l.no_arrow.map(|n| (l.id.clone(), n)))
                .collect(),
            covers_avoid: p
                .layers
                .iter()
                .filter(|l| l.covers_avoid > 0)
                .map(|l| (l.id.clone(), l.covers_avoid))
                .collect(),
        });
    }
    report
}

/// A value with every object's keys in order, whatever serde_json's features.
fn sorted(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<(String, Value)> = m.into_iter().collect();
            keys.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(keys.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
        v => v,
    }
}

/// A stable text golden of one screen: the host's drawn frame (`frame_text`, its rows as
/// lines), a map of what the plan put on each cell, and the plan as pretty JSON with sorted
/// keys. A host commits one per canonical screen; a change that moves a box shows up as a
/// diff in its CI.
///
/// The map: `#` a box, `=` a strip, `c` an edge chip, `>` an arrow (`^ v < >` heads are in
/// the plan), `*` a ringed cell, `.` a dimmed one, blank otherwise. Trailing spaces are
/// trimmed from every line.
pub fn snapshot(plan: &Plan, frame_text: &str) -> String {
    let (w, h) = (plan.width as usize, plan.height as usize);
    let mut map: Vec<Vec<char>> = (0..h)
        .map(|y| {
            (0..w)
                .map(|x| {
                    if plan.dimmed(x as u16, y as u16) {
                        '.'
                    } else {
                        ' '
                    }
                })
                .collect()
        })
        .collect();
    let mut put = |x: u16, y: u16, c: char| {
        if let Some(cell) = map
            .get_mut(y as usize)
            .and_then(|row| row.get_mut(x as usize))
        {
            *cell = c;
        }
    };
    let fill = |x: Rect, c: char, put: &mut dyn FnMut(u16, u16, char)| {
        for y in x.y..x.bottom() {
            for cx in x.x..x.right() {
                put(cx, y, c);
            }
        }
    };
    for p in &plan.layers {
        for x in &p.ring {
            fill(*x, '*', &mut put);
        }
    }
    for p in &plan.layers {
        for s in p.route.iter().flat_map(|rt| &rt.steps) {
            put(s.x, s.y, '>');
        }
        if let Some(c) = p.chip {
            fill(c, 'c', &mut put);
        }
        if let Some(x) = p.rect {
            let c = if p.mode == Some(Mode::Strip) {
                '='
            } else {
                '#'
            };
            fill(x, c, &mut put);
        }
    }
    let mut out = format!("--- frame {w}x{h}\n");
    for line in frame_text.lines() {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.push_str("--- map: # box, = strip, c chip, > arrow, * ring, . dimmed\n");
    for row in &map {
        out.push_str(row.iter().collect::<String>().trim_end());
        out.push('\n');
    }
    out.push_str("--- plan\n");
    let json = serde_json::to_value(plan)
        .map(sorted)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_default();
    out.push_str(&json);
    out.push('\n');
    out
}

#[cfg(feature = "caretline")]
mod views;
#[cfg(feature = "caretline")]
pub use views::{Mapping, OwnedView, OwnedViews, check_mapping};
