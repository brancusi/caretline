//! A step as `caretline_layers` layers (owner `guide`), and planning every step at several
//! sizes.

use caretline_layers::{
    Content, Grid, HINT, Layer, LayerOp, Layers, Limits, NoArrow, Owner, Plan, Refusal, Renderers,
    Resolve, Selector, apply, plan,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::check::{Level, Problem};
use crate::model::{Step, StepAnchor, Tour};

/// The z the walkthrough's layers take: the bottom of the `guide` band.
pub const GUIDE_Z: i16 = 10;

/// Step `i`'s layers, owned by `guide`, ready to push. Each layer's content is its kind (the
/// tour's if it names none) with its data; for kinds other than `hint`, `at` and `of` (the
/// step's place, from 1) are added to an object's data so a renderer can draw step dots.
/// With `nudged`, the step's nudge data is merged into each. A `find` anchor still
/// unresolved is left out of the fallbacks, and a layer with none left is left out.
pub fn step_layers(tour: &Tour, i: usize, nudged: bool) -> Vec<Layer> {
    let Some(step) = tour.steps.get(i) else {
        return Vec::new();
    };
    let nudge = step.nudge.as_ref().filter(|_| nudged).map(|n| &n.data);
    step.layers
        .iter()
        .filter_map(|l| {
            let anchor: Vec<_> = l
                .anchor
                .iter()
                .filter_map(StepAnchor::anchor)
                .cloned()
                .collect();
            if anchor.is_empty() {
                return None;
            }
            // No kind and no data: a layer with no content (a ring or a spotlight alone, no
            // box), as caretline-layers models one. A nudge doesn't give it content.
            if l.kind.is_none() && l.data.is_null() {
                return Some(layer(l, anchor, None));
            }
            let kind = l.kind.clone().unwrap_or_else(|| tour.kind.clone());
            let mut data = l.data.clone();
            if let Some(Value::Object(n)) = nudge {
                let mut m = match data {
                    Value::Object(m) => m,
                    _ => Map::new(),
                };
                m.extend(n.clone());
                data = Value::Object(m);
            }
            if kind != HINT {
                let mut m = match data {
                    Value::Object(m) => m,
                    Value::Null => Map::new(),
                    other => {
                        return Some(layer(l, anchor, Some(Content { kind, data: other })));
                    }
                };
                m.insert("at".into(), (i + 1).into());
                m.insert("of".into(), tour.steps.len().into());
                data = Value::Object(m);
            }
            Some(layer(l, anchor, Some(Content { kind, data })))
        })
        .collect()
}

fn layer(
    l: &crate::StepLayer,
    anchor: Vec<caretline_layers::Anchor>,
    content: Option<Content>,
) -> Layer {
    let mut out = Layer::new(caretline_layers::Anchor::Caret);
    out.id = l.id.clone();
    out.owner = Owner::Guide;
    out.z = GUIDE_Z;
    out.anchor = anchor;
    out.content = content;
    out.arrow = l.place.arrow;
    out.ring = l.place.ring.clone();
    out.spotlight = l.place.spotlight.clone();
    out.capture = l.capture;
    out.avoid = l
        .avoid
        .iter()
        .filter_map(StepAnchor::anchor)
        .cloned()
        .collect();
    out.hide_off_screen = l.place.hide_off_screen;
    out.place = l.place.sides.clone();
    out.max_width = l.place.max_width;
    out
}

/// Does a [`TourEffect::Layers`](crate::TourEffect::Layers): pops every `guide` layer and
/// pushes these, as the host (no agent limits apply).
pub fn replace_guide(layers: &mut Layers, new: Vec<Layer>, now_ms: u64) -> Result<(), Refusal> {
    let none = Limits::default();
    apply(
        layers,
        LayerOp::Pop(Selector::Owner(Owner::Guide)),
        None,
        now_ms,
        &none,
    )?;
    for l in new {
        apply(layers, LayerOp::Push(l), None, now_ms, &none)?;
    }
    Ok(())
}

/// One step planned at one size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepPlan {
    pub step: String,
    pub width: u16,
    pub height: u16,
    pub plan: Plan,
    /// What didn't work: a layer whose anchor resolved nowhere (`not_found`), a kind with no
    /// renderer (`unrendered`), a `find` never resolved (`unresolved_find`) or a layer the
    /// model refused (`refused`) are errors; an arrow with no way (`no_way`) and an anchor
    /// off screen (`off_screen`) are warnings.
    pub problems: Vec<Problem>,
}

/// Sets a step's scene up at a size and calls back with what it drew: the grid and the
/// anchor resolver ([`plan_steps`]).
pub type Scene<'a> = dyn FnMut(&Step, u16, u16, &mut dyn FnMut(&Grid, &dyn Resolve)) + 'a;

/// Plans every step of `tour` at each size, for a host that checks its walkthroughs at
/// several terminal sizes. For each step and size, `scene` sets the step's scene up (its
/// `host` patch applied, drawn at that size) and calls back with the grid and the anchor
/// resolver it drew; the step's layers are planned with them and the host's `renderers`.
/// A scene that doesn't call back plans nothing for that step and size.
///
/// ```ignore
/// let plans = plan_steps(&tour, &[(80, 24), (44, 16)], &renderers, &mut |step, w, h, go| {
///     let (grid, anchors) = my_host.draw(step.host.as_ref(), w, h);
///     go(&grid, &anchors)
/// });
/// ```
pub fn plan_steps(
    tour: &Tour,
    sizes: &[(u16, u16)],
    renderers: &Renderers,
    scene: &mut Scene,
) -> Vec<StepPlan> {
    let mut out = Vec::new();
    for (i, step) in tour.steps.iter().enumerate() {
        let mut layers = Layers::default();
        let mut base = Vec::new();
        for l in &step.layers {
            for a in &l.anchor {
                if let StepAnchor::Find { text, .. } = a {
                    base.push(Problem::layer(
                        Level::Error,
                        "unresolved_find",
                        &step.id,
                        &l.id,
                        format!("{text:?} was never resolved (Tour::resolve_finds)"),
                    ));
                }
            }
        }
        for l in step_layers(tour, i, false) {
            let id = l.id.clone();
            if let Err(r) = apply(&mut layers, LayerOp::Push(l), None, 0, &Limits::default()) {
                base.push(Problem::layer(
                    Level::Error,
                    "refused",
                    &step.id,
                    &id,
                    r.to_string(),
                ));
            }
        }
        for &(w, h) in sizes {
            let mut problems = base.clone();
            let mut planned = None;
            scene(step, w, h, &mut |grid, anchors| {
                planned = Some(plan(&layers, anchors, grid, renderers));
            });
            let Some(p) = planned else { continue };
            for id in &p.missing {
                problems.push(Problem::layer(
                    Level::Error,
                    "not_found",
                    &step.id,
                    id,
                    format!("no anchor resolved at {w}×{h}"),
                ));
            }
            for id in &p.unrendered {
                problems.push(Problem::layer(
                    Level::Error,
                    "unrendered",
                    &step.id,
                    id,
                    "no renderer for its kind".to_string(),
                ));
            }
            for l in &p.layers {
                if l.no_arrow == Some(NoArrow::NoWay) {
                    problems.push(Problem::layer(
                        Level::Warning,
                        "no_way",
                        &step.id,
                        &l.id,
                        format!("no arrow fits at {w}×{h}"),
                    ));
                }
                if l.anchor.as_ref().is_some_and(|a| a.off.is_some()) {
                    problems.push(Problem::layer(
                        Level::Warning,
                        "off_screen",
                        &step.id,
                        &l.id,
                        format!("its anchor is off screen at {w}×{h}"),
                    ));
                }
            }
            out.push(StepPlan {
                step: step.id.clone(),
                width: w,
                height: h,
                plan: p,
                problems,
            });
        }
    }
    out
}
