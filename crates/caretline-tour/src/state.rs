//! The reducer: a walkthrough's progress as data, changed only by [`apply`] (ops) and
//! [`observe`] (what happened, and the time).

use std::collections::BTreeMap;

use caretline_layers::Layer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::check::{Level, check};
use crate::layers::step_layers;
use crate::model::{END, Narration, Tour};
use crate::pred::{Ctx, Pred, TourHost, holds, tally};

fn is_false(b: &bool) -> bool {
    !*b
}

/// How a walkthrough ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    /// It reached its end.
    Finished,
    /// The person stopped it.
    Stopped,
}

/// What a host remembers of a walkthrough once it ends: the version seen and how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seen {
    pub version: u32,
    pub end: End,
}

/// A walkthrough's progress. Serializable, so a host keeps it in its own single state (or a
/// caretline view's `ext`), and a trace of it replays.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TourState {
    /// The whole walkthrough, as started (so a trace needs no file). Kept after it ends, for
    /// `restart`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tour: Option<Tour>,
    /// The current step's id; `None` when no walkthrough is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// When the current step began (`now_ms`).
    #[serde(default)]
    pub since_ms: u64,
    /// Counted predicates since the step began ([`Pred::count_key`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub counts: BTreeMap<String, u32>,
    /// The steps left going forward, most recent last: `back` returns to the last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<String>,
    /// The current step's nudge has been given.
    #[serde(default, skip_serializing_if = "is_false")]
    pub nudged: bool,
    /// What the host loaded of walkthroughs seen before, by tour id, and what ended since.
    /// The host persists [`TourEffect::Ended`]; the crate never touches storage.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub seen: BTreeMap<String, Seen>,
}

impl TourState {
    /// Whether a walkthrough is running.
    pub fn running(&self) -> bool {
        self.step.is_some()
    }

    /// The current step's index.
    pub fn index(&self) -> Option<usize> {
        self.tour.as_ref()?.index(self.step.as_deref()?)
    }

    /// The current step.
    pub fn current(&self) -> Option<&crate::Step> {
        self.tour.as_ref()?.step(self.step.as_deref()?)
    }

    /// The current step's narration.
    pub fn narration(&self) -> Option<&Narration> {
        self.current()?.narration.as_ref()
    }

    /// The current step's layers, as [`TourEffect::Layers`] last gave them.
    pub fn layers(&self) -> Vec<Layer> {
        match (&self.tour, self.index()) {
            (Some(t), Some(i)) => step_layers(t, i, self.nudged),
            _ => Vec::new(),
        }
    }

    /// Whether to offer `tour`: never seen, or seen at an earlier version and marked
    /// `reoffer`. A walkthrough never starts by itself; this is for a host's prompt.
    pub fn offer(&self, tour: &Tour) -> bool {
        match self.seen.get(&tour.id) {
            None => true,
            Some(s) => s.version < tour.version && tour.reoffer,
        }
    }
}

/// A change to a [`TourState`]. Traces record these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum TourOp {
    /// Starts this walkthrough at its first step (the whole tour, so a trace is
    /// self-contained; [`ops::parse`](crate::ops::parse) looks an id up in the host's
    /// library).
    Start(Box<Tour>),
    Stop,
    Next,
    Back,
    /// Jumps to the step with this id.
    To(String),
    /// Starts the current (or last) walkthrough again.
    Restart,
}

/// What a host does after [`apply`] or [`observe`], in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum TourEffect {
    /// Apply the step's `host` patch to the host's own state.
    Host { patch: Value },
    /// Replace the walkthrough's layers (owner `guide`) with these; none clears them.
    /// [`replace_guide`](crate::replace_guide) does it.
    Layers { layers: Vec<Layer> },
    /// The current step is now `step`, `at` of `of` (from 1).
    Step {
        tour: String,
        step: String,
        at: usize,
        of: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        narration: Option<Narration>,
    },
    /// The walkthrough ended: persist `seen` for `tour`.
    Ended { tour: String, seen: Seen },
}

/// Why an op was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TourReason {
    /// A malformed request or tour.
    Invalid,
    /// No step (or walkthrough) with that id.
    NotFound,
    /// No walkthrough is running (or none to restart).
    NotRunning,
    /// `back` at the first step.
    NoBack,
}

/// A refused op: a stable reason and a sentence for people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TourError {
    pub reason: TourReason,
    pub detail: String,
}

impl std::fmt::Display for TourError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let r = serde_json::to_value(self.reason)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        write!(f, "{r}: {}", self.detail)
    }
}

impl std::error::Error for TourError {}

fn refuse<T>(reason: TourReason, detail: impl Into<String>) -> Result<T, TourError> {
    Err(TourError {
        reason,
        detail: detail.into(),
    })
}

/// Applies an op with no host to ask: branches and `skip_if` see only time and counts.
/// Deterministic: the same ops at the same times give the same state and effects.
pub fn apply(s: &mut TourState, op: TourOp, now_ms: u64) -> Result<Vec<TourEffect>, TourError> {
    apply_with(s, op, now_ms, &())
}

/// Applies an op; `host` answers the predicates of `next` branches and `skip_if` met going
/// forward. Jumps (`to`) and `back` enter their step exactly, skipping nothing.
pub fn apply_with(
    s: &mut TourState,
    op: TourOp,
    now_ms: u64,
    host: &dyn TourHost,
) -> Result<Vec<TourEffect>, TourError> {
    let mut fx = Vec::new();
    match op {
        TourOp::Start(t) => {
            if let Some(p) = check(&t).into_iter().find(|p| p.level == Level::Error) {
                return refuse(TourReason::Invalid, format!("{}: {}", p.code, p.detail));
            }
            s.tour = Some(*t);
            s.step = None;
            s.history.clear();
            enter_forward(s, 0, now_ms, host, &mut fx);
        }
        TourOp::Restart => {
            if s.tour.is_none() {
                return refuse(TourReason::NotRunning, "no walkthrough to restart");
            }
            s.step = None;
            s.history.clear();
            enter_forward(s, 0, now_ms, host, &mut fx);
        }
        TourOp::Stop => {
            running(s)?;
            end(s, End::Stopped, &mut fx);
        }
        TourOp::Next => {
            running(s)?;
            forward(s, now_ms, host, &mut fx);
        }
        TourOp::Back => {
            running(s)?;
            let Some(id) = s.history.pop() else {
                return refuse(TourReason::NoBack, "at the first step");
            };
            let i = tour(s).index(&id).unwrap_or(0);
            enter(s, i, now_ms, &mut fx);
        }
        TourOp::To(id) => {
            let Some(t) = &s.tour else {
                return refuse(TourReason::NotRunning, "no walkthrough");
            };
            let Some(i) = t.index(&id) else {
                return refuse(TourReason::NotFound, format!("no step {id:?}"));
            };
            if let Some(cur) = s.step.take() {
                s.history.push(cur);
            }
            enter(s, i, now_ms, &mut fx);
        }
    }
    Ok(fx)
}

/// Checks the current step after a message the host applied (or a tick): counts what
/// happened, gives the nudge when it's time, and goes forward when `advance` holds. Call it
/// once per message, with the host's answers for that message.
pub fn observe(s: &mut TourState, host: &dyn TourHost, now_ms: u64) -> Vec<TourEffect> {
    let mut fx = Vec::new();
    let (Some(t), Some(i)) = (&s.tour, s.index()) else {
        return fx;
    };
    let step = &t.steps[i];
    let mut preds: Vec<&Pred> = step.advance.iter().collect();
    preds.extend(step.next.iter().filter_map(|b| b.when.as_ref()));
    tally(&preds, host, &mut s.counts);
    if !s.nudged
        && let Some(n) = &step.nudge
        && now_ms.saturating_sub(s.since_ms) >= n.after_ms
    {
        s.nudged = true;
        fx.push(TourEffect::Layers {
            layers: step_layers(t, i, true),
        });
    }
    let advance = step.advance.clone();
    if advance.is_some_and(|p| test(s, i, &p, host, now_ms)) {
        forward(s, now_ms, host, &mut fx);
    }
    fx
}

fn tour(s: &TourState) -> &Tour {
    s.tour.as_ref().expect("a running walkthrough has its tour")
}

fn running(s: &TourState) -> Result<(), TourError> {
    if s.running() && s.index().is_some() {
        Ok(())
    } else {
        refuse(TourReason::NotRunning, "no walkthrough is running")
    }
}

fn test(s: &TourState, i: usize, p: &Pred, host: &dyn TourHost, now_ms: u64) -> bool {
    let step = &tour(s).steps[i];
    let anchors = |name: &str| step.anchors(name);
    holds(
        p,
        &Ctx {
            host,
            counts: &s.counts,
            lasted_ms: now_ms.saturating_sub(s.since_ms),
            anchors: &anchors,
        },
    )
}

/// Where going forward from step `i` leads: a step index, or `None` for the end.
fn target(s: &TourState, i: usize, host: &dyn TourHost, now_ms: u64) -> Option<usize> {
    let t = tour(s);
    for b in &t.steps[i].next {
        if b.when.as_ref().is_none_or(|p| test(s, i, p, host, now_ms)) {
            return if b.goto == END {
                None
            } else {
                // `check` refuses an unknown goto at start.
                t.index(&b.goto)
            };
        }
    }
    (i + 1 < t.steps.len()).then_some(i + 1)
}

fn forward(s: &mut TourState, now_ms: u64, host: &dyn TourHost, fx: &mut Vec<TourEffect>) {
    let Some(i) = s.index() else { return };
    match target(s, i, host, now_ms) {
        None => end(s, End::Finished, fx),
        Some(j) => {
            if let Some(cur) = s.step.take() {
                s.history.push(cur);
            }
            enter_forward(s, j, now_ms, host, fx);
        }
    }
}

/// Enters step `i` going forward: passes over steps whose `skip_if` holds on arrival.
fn enter_forward(
    s: &mut TourState,
    mut i: usize,
    now_ms: u64,
    host: &dyn TourHost,
    fx: &mut Vec<TourEffect>,
) {
    // Each step is passed over at most once, so a cycle of skips ends.
    for _ in 0..=tour(s).steps.len() {
        let Some(p) = tour(s).steps[i].skip_if.clone() else {
            break;
        };
        // Arriving: a fresh count and clock.
        s.counts.clear();
        s.since_ms = now_ms;
        if !test(s, i, &p, host, now_ms) {
            break;
        }
        match target(s, i, host, now_ms) {
            None => {
                s.step = Some(tour(s).steps[i].id.clone());
                return end(s, End::Finished, fx);
            }
            Some(j) => i = j,
        }
    }
    enter(s, i, now_ms, fx);
}

/// Enters step `i` exactly: its host patch, its layers, and the step change.
fn enter(s: &mut TourState, i: usize, now_ms: u64, fx: &mut Vec<TourEffect>) {
    let t = tour(s);
    let step = &t.steps[i];
    if let Some(h) = &step.host {
        fx.push(TourEffect::Host { patch: h.clone() });
    }
    fx.push(TourEffect::Layers {
        layers: step_layers(t, i, false),
    });
    fx.push(TourEffect::Step {
        tour: t.id.clone(),
        step: step.id.clone(),
        at: i + 1,
        of: t.steps.len(),
        narration: step.narration.clone(),
    });
    s.step = Some(step.id.clone());
    s.since_ms = now_ms;
    s.counts.clear();
    s.nudged = false;
}

fn end(s: &mut TourState, how: End, fx: &mut Vec<TourEffect>) {
    let t = tour(s);
    let id = t.id.clone();
    let seen = Seen {
        version: t.version,
        end: how,
    };
    fx.push(TourEffect::Layers { layers: Vec::new() });
    fx.push(TourEffect::Ended {
        tour: id.clone(),
        seen,
    });
    s.seen.insert(id, seen);
    s.step = None;
    s.counts.clear();
    s.history.clear();
    s.nudged = false;
}
