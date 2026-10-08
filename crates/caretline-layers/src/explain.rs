//! The inspector: why each box landed where it did.
//!
//! [`Plan::explain`] reads a plan back in words (where each anchor resolved, the box, the
//! arrow or why there is none, the dock, avoid cells covered). [`plan_explained`] also keeps,
//! while it plans, every candidate box placement weighed: its side, cells, what it covered,
//! its distance, its arrow's cost, and why the winner won. Gathered only when asked: [`plan`]
//! keeps nothing of it.
//!
//! [`plan`]: crate::plan

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::geom::{Rect, Side};
use crate::model::{Anchor, Layer, Owner};
use crate::place::{Attach, Grid, Mode, NARROW_COLS, NARROW_ROWS, NoArrow, Plan, Planned, Size};
use crate::resolve::{Off, Resolve, Resolved};

/// Every layer of a plan, explained ([`plan_explained`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Explanation {
    pub width: u16,
    pub height: u16,
    /// In draw order, as the plan's layers, with the missing ones where they fell.
    pub layers: Vec<Explained>,
}

/// One layer's placement, explained.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Explained {
    pub id: String,
    pub owner: Owner,
    /// Each fallback anchor in order, and what the resolver answered for it (up to the one
    /// used).
    pub anchors: Vec<Tried>,
    /// The fallback used (its index in `anchors`), `None` when none resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<usize>,
    /// The area was under [`NARROW_COLS`] columns or [`NARROW_ROWS`] rows: a strip.
    #[serde(default, skip_serializing_if = "is_false")]
    pub narrow: bool,
    /// The size the renderer measured for each side's room (a side with no room, or a zero
    /// size, is left out).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measured: Vec<(Side, Size)>,
    /// Every box placement weighed, best first by the box's score and a first guess at its
    /// arrow (an off-screen anchor's are beside its edge chip).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Candidate>,
    /// The winner's index in `candidates`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner: Option<usize>,
    /// Where the winner was before the sliver rule stretched or moved it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sliver_from: Option<Rect>,
    /// Why it won, or why there is no box.
    #[serde(default)]
    pub why: String,
}

/// What the resolver answered for one fallback anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tried {
    pub anchor: Anchor,
    /// `None`: not found (no resolver knew it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolved>,
}

/// A candidate box. Scores are in tenths of a text cell, as placement keeps them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub side: Side,
    pub rect: Rect,
    /// Past the nearest four places on its side (tried only while none nearer was clear).
    #[serde(default, skip_serializing_if = "is_false")]
    pub far: bool,
    /// Text cells it covers (10 each).
    pub text: u32,
    /// Avoid weight it covers (10 per unit). Any covered ranks it after every box that covers
    /// none.
    pub avoid: u32,
    /// Dimmed cells it covers (3 each).
    pub dimmed: u32,
    /// It covers the caret (500).
    #[serde(default, skip_serializing_if = "is_false")]
    pub caret: bool,
    /// Cells from the anchor, and steps past the nearest place (5 each).
    pub distance: u32,
    /// The box alone.
    pub score: u32,
    /// What its arrow came to, when the layer asked for one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow: Option<Arrow>,
}

/// A candidate's arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arrow {
    /// Routed: its `cost` added to the box's score (the search's cost × 5, 300 per word cell
    /// crossed, 10 per unit of avoid weight crossed), over `cells` cells.
    Routed { cost: u32, cells: u16 },
    /// No way to the anchor: ranks after every box whose arrow routes.
    NoWay,
    /// Its search stopped once every way left would lose to the best so far.
    Over,
    /// Not routed: even the cheapest arrow couldn't beat the best so far.
    Pruned,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Candidate {
    /// What it ranks by: covers avoid cells, its arrow doesn't route, then its total.
    pub(crate) fn total(&self) -> Option<u32> {
        match self.arrow {
            None => Some(self.score),
            Some(Arrow::Routed { cost, .. }) => Some(self.score + cost),
            Some(Arrow::NoWay) => Some(self.score),
            Some(Arrow::Over | Arrow::Pruned) => None,
        }
    }
}

impl Explained {
    /// A layer about to be planned: its fallbacks as the resolver answers them (up to and
    /// including the first that resolves, as placement reads them).
    pub(crate) fn begin(layer: &Layer, anchors: &dyn Resolve, grid: &Grid) -> Explained {
        let screen = Rect::new(0, 0, grid.width, grid.height);
        let mut tried = Vec::new();
        let mut used = None;
        for (i, a) in layer.anchor.iter().enumerate() {
            if let Anchor::Screen(_) = a {
                tried.push(Tried {
                    anchor: a.clone(),
                    resolved: None,
                });
                used = Some(i);
                break;
            }
            let r = anchors.resolve(a);
            let shows = r.as_ref().is_some_and(|r| {
                r.rects.iter().any(|x| !x.intersection(&screen).is_empty()) || r.off.is_some()
            });
            tried.push(Tried {
                anchor: a.clone(),
                resolved: r,
            });
            if shows {
                used = Some(i);
                break;
            }
        }
        Explained {
            id: layer.id.clone(),
            owner: layer.owner.clone(),
            anchors: tried,
            used,
            ..Explained::default()
        }
    }
}

fn rect(r: &Rect) -> String {
    format!("({},{} {}x{})", r.x, r.y, r.w, r.h)
}

fn rects(rs: &[Rect]) -> String {
    rs.iter().map(rect).collect::<Vec<_>>().join(" ")
}

fn side(s: Side) -> &'static str {
    match s {
        Side::Below => "below",
        Side::Above => "above",
        Side::Right => "right",
        Side::Left => "left",
    }
}

fn off(o: &Off) -> String {
    match o {
        Off::Above { x } => format!(
            "off above{}",
            x.map_or(String::new(), |x| format!(" at x {x}"))
        ),
        Off::Below { x } => format!(
            "off below{}",
            x.map_or(String::new(), |x| format!(" at x {x}"))
        ),
        Off::Left { y } => format!("off left at y {y}"),
        Off::Right { y } => format!("off right at y {y}"),
    }
}

fn resolved(r: &Resolved) -> String {
    let mut s = if !r.rects.is_empty() {
        rects(&r.rects)
    } else if let Some(o) = &r.off {
        off(o)
    } else {
        "no cells".into()
    };
    if let Some(v) = &r.view {
        s.push_str(&format!(" in {v}"));
    }
    s
}

fn anchor(a: &Anchor) -> String {
    serde_json::to_string(a).unwrap_or_else(|_| format!("{a:?}"))
}

/// Why a layer that asked for an arrow has none, in words.
pub(crate) fn no_arrow(n: NoArrow) -> &'static str {
    match n {
        NoArrow::Docked => "docked: the anchor is off screen and the edge chip points the way",
        NoArrow::Screen => "screen: the layer is at a screen position",
        NoArrow::NoBox => "no_box: a strip, or no box to start from",
        NoArrow::NoWay => {
            "no_way: no route round the other layers, holes, protected cells and wide graphemes"
        }
        NoArrow::HeadOnText => "head_on_text: every cell beside the anchor is text or a word gap",
        NoArrow::HeadOffAnchorRows => {
            "head_off_anchor_rows: no arrow could end on one of the anchor's own rows (head: on_anchor_rows)"
        }
    }
}

/// The lines a plan says about one placed layer.
fn planned(p: &Planned, out: &mut Vec<String>) {
    match &p.anchor {
        Some(a) => out.push(format!("  anchor: {}", resolved(a))),
        None => out.push("  anchor: a screen position".into()),
    }
    match (p.rect, p.mode) {
        (Some(r), Some(Mode::Box)) => out.push(format!(
            "  box {} {}",
            p.side.map_or("at the screen position", side),
            rect(&r)
        )),
        (Some(r), Some(Mode::Strip)) => out.push(format!("  strip {}", rect(&r))),
        _ => out.push("  no box".into()),
    }
    if let Some(c) = p.chip {
        out.push(format!("  edge chip {}", rect(&c)));
    }
    if let Some(Attach { edge, offset }) = p.dock {
        out.push(format!(
            "  docked: the chip meets the box's {} edge at {offset}",
            format!("{edge:?}").to_lowercase()
        ));
    }
    if let Some(rt) = &p.route {
        let head = rt
            .steps
            .last()
            .map_or(String::new(), |s| format!(" to ({},{})", s.x, s.y));
        out.push(format!(
            "  arrow from ({},{}) ({} edge, {}){head}, {} cells",
            rt.junction.0,
            rt.junction.1,
            format!("{:?}", rt.attach.edge).to_lowercase(),
            rt.attach.offset,
            rt.steps.len()
        ));
    }
    if let Some(n) = p.no_arrow {
        out.push(format!("  no arrow: {}", no_arrow(n)));
    }
    if p.covers_avoid > 0 {
        out.push(format!(
            "  covers {} avoid cells: no box in reach kept off them all",
            p.covers_avoid
        ));
    }
    if !p.ring.is_empty() {
        out.push(format!("  ring {}", rects(&p.ring)));
    }
}

impl Plan {
    /// The plan in words, per layer: where its anchor resolved (and in which view), the box
    /// or strip and its side, the edge chip and dock, the arrow or why there is none, avoid
    /// cells covered; then the missing and unrendered layers. Read from the plan alone; for
    /// the candidates weighed and why the winner won, plan with
    /// [`plan_explained`](crate::plan_explained).
    pub fn explain(&self) -> String {
        let mut out = vec![format!(
            "plan {}x{}: {} layers",
            self.width,
            self.height,
            self.layers.len()
        )];
        for p in &self.layers {
            out.push(format!(
                "{} ({}, z {}){}",
                p.id,
                serde_json::to_value(&p.owner)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default(),
                p.z,
                if self.unrendered.contains(&p.id) {
                    ": no renderer for its kind"
                } else {
                    ""
                }
            ));
            planned(p, &mut out);
        }
        for id in &self.missing {
            out.push(format!("{id}: missing, none of its anchors resolved"));
        }
        let mut s = out.join("\n");
        s.push('\n');
        s
    }
}

impl Explanation {
    /// The layer explained, by id.
    pub fn get(&self, id: &str) -> Option<&Explained> {
        self.layers.iter().find(|l| l.id == id)
    }
}

/// Why `w` ranks before `r`, in words.
fn beats(w: &Candidate, r: &Candidate) -> String {
    let wa = w.avoid > 0;
    let ra = r.avoid > 0;
    if wa != ra {
        return format!(
            "it covers no avoid cells; the runner-up covers weight {}",
            r.avoid
        );
    }
    let wm = matches!(w.arrow, Some(Arrow::NoWay));
    let rm = matches!(
        r.arrow,
        Some(Arrow::NoWay) | Some(Arrow::Over) | Some(Arrow::Pruned)
    );
    if wm != rm && !wm {
        return match r.arrow {
            Some(Arrow::NoWay) => "its arrow routes; the runner-up's has no way".into(),
            _ => format!(
                "total {} beats the runner-up, whose arrow couldn't come in under it",
                w.total().unwrap_or(0)
            ),
        };
    }
    match (w.total(), r.total()) {
        (Some(a), Some(b)) if a < b => format!(
            "total {a} < {b} (text {} vs {}, avoid {} vs {}, distance {} vs {}{})",
            w.text,
            r.text,
            w.avoid,
            r.avoid,
            w.distance,
            r.distance,
            match (w.arrow, r.arrow) {
                (Some(Arrow::Routed { cost: a, .. }), Some(Arrow::Routed { cost: b, .. })) =>
                    format!(", arrow {a} vs {b}"),
                _ => String::new(),
            }
        ),
        (Some(a), Some(b)) if a == b && w.side != r.side => format!(
            "a tie at {a}: {} comes before {} in the layer's sides",
            side(w.side),
            side(r.side)
        ),
        (Some(a), Some(b)) if a == b => format!("a tie at {a}: tried first"),
        _ => "ranked first".into(),
    }
}

impl fmt::Display for Explanation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "plan {}x{}: {} layers",
            self.width,
            self.height,
            self.layers.len()
        )?;
        for l in &self.layers {
            write!(f, "{l}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Explained {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let owner = serde_json::to_value(&self.owner)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        writeln!(f, "{} ({owner})", self.id)?;
        for (i, t) in self.anchors.iter().enumerate() {
            let what = match (&t.anchor, &t.resolved) {
                (Anchor::Screen(_), _) => "a screen position".to_string(),
                (_, None) => "not found".to_string(),
                (_, Some(r)) => resolved(r),
            };
            let mark = if self.used == Some(i) { "*" } else { " " };
            writeln!(
                f,
                "  {mark}anchor {} of {}: {} -> {what}",
                i + 1,
                self.anchors.len(),
                anchor(&t.anchor)
            )?;
        }
        if self.narrow {
            writeln!(
                f,
                "  narrow area (under {NARROW_COLS} columns or {NARROW_ROWS} rows): a strip"
            )?;
        }
        if !self.measured.is_empty() {
            let m: Vec<String> = self
                .measured
                .iter()
                .map(|(s, z)| format!("{} {}x{}", side(*s), z.w, z.h))
                .collect();
            writeln!(f, "  measured: {}", m.join(", "))?;
        }
        if !self.candidates.is_empty() {
            writeln!(
                f,
                "  candidates ({}): side rect | text avoid dim caret dist | box arrow = total",
                self.candidates.len()
            )?;
            for (i, c) in self.candidates.iter().enumerate() {
                let mark = if self.winner == Some(i) { "*" } else { " " };
                let arrow = match c.arrow {
                    None => "-".to_string(),
                    Some(Arrow::Routed { cost, cells }) => format!("{cost} ({cells} cells)"),
                    Some(Arrow::NoWay) => "no way".into(),
                    Some(Arrow::Over) => "over".into(),
                    Some(Arrow::Pruned) => "pruned".into(),
                };
                writeln!(
                    f,
                    "   {mark}{:<5} {:<16}| {:>3} {:>3} {:>3} {:<3} {:>3}{} | {:>4} {} = {}",
                    side(c.side),
                    rect(&c.rect),
                    c.text,
                    c.avoid,
                    c.dimmed,
                    if c.caret { "yes" } else { "-" },
                    c.distance,
                    if c.far { " far" } else { "" },
                    c.score,
                    arrow,
                    c.total().map_or("-".into(), |t| t.to_string())
                )?;
            }
        }
        if let Some(r) = self.sliver_from {
            writeln!(f, "  the sliver rule moved it from {}", rect(&r))?;
        }
        if !self.why.is_empty() {
            writeln!(f, "  {}", self.why)?;
        }
        Ok(())
    }
}

/// The sentence for a winner among `cands`.
pub(crate) fn why_won(cands: &[Candidate], winner: usize) -> String {
    let w = &cands[winner];
    // The runner-up: the best other by rank (avoid, no route, total), routed or not.
    let rank = |c: &Candidate| {
        (
            c.avoid > 0,
            !matches!(c.arrow, None | Some(Arrow::Routed { .. })),
            c.total().unwrap_or(u32::MAX),
        )
    };
    let runner = cands
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != winner)
        .min_by_key(|(_, c)| rank(c));
    match runner {
        None => format!("won: the only place for a box {}", side(w.side)),
        Some((_, r)) => format!("won: {}", beats(w, r)),
    }
}
