//! caretline-tour: walkthroughs over a caretline host's screen. A walkthrough is a list of
//! steps; each step sets the host's scene (`host`, an opaque patch), points at things in it
//! (`layers`, placed by `caretline-layers`) and says what it is about (`narration`). The host
//! keeps the state and draws.
//!
//! - **Format** ([`Tour`], [`Step`], [`StepLayer`], [`Narration`], [`Pred`], [`Branch`]):
//!   `{id, version, title, kind, step[]}`, read strictly from TOML ([`parse_toml`]) or JSON
//!   ([`parse_json`]). A step has `layers`, or the single-layer shorthand (`anchor`, `kind`,
//!   `data`, `place`, `capture`) that reads as `layers[0]`. [`check`] lints what parses but
//!   can't work.
//! - **Reducer** ([`TourState`], [`TourOp`], [`apply`], [`TourEffect`]): progress as
//!   serializable data, changed only by ops and [`observe`]. Entering a step yields its `host`
//!   patch, its layers and the step change, whichever way it was reached, so a jump looks the
//!   same as arriving in order. No clock: time is `now_ms`.
//! - **Predicates** ([`Pred`], [`TourHost`]): `advance`, `skip_if` and branches ask the host
//!   what happened in a message (an action ran, a host event, a state subset) and check the
//!   time. With the `caretline` feature, [`Editor`] answers the editor's own questions
//!   (message kinds, commands, `caret_in`, `changed`, `folded`, `selection`). Steps without
//!   predicates move only by ops: that is how pages are turned.
//! - **Layers** ([`step_layers`], [`replace_guide`], [`plan_steps`]): a step as
//!   `caretline_layers::Layer`s owned by `guide`, and every step planned at several sizes.
//! - **Ops** ([`ops`]): `tour.start`, `tour.step`, `tour.restart`, `tour.stop`, `tour.list`,
//!   for a host's own JSON protocol, with a JSON Schema.
//!
//! A host, per message:
//!
//! ```ignore
//! for fx in caretline_tour::observe(&mut tour_state, &my_host_answers, now_ms) {
//!     match fx {
//!         TourEffect::Host { patch } => my_state.apply_patch(patch),
//!         TourEffect::Layers { layers } => replace_guide(&mut my_layers, layers, now_ms)?,
//!         TourEffect::Step { narration, .. } => my_panel.show(narration),
//!         TourEffect::Ended { tour, seen } => my_store.remember(tour, seen),
//!     }
//! }
//! ```

// `ops::schema` is one `json!` literal, deeper than the default limit.
#![recursion_limit = "256"]

mod check;
#[cfg(feature = "caretline")]
mod editor;
mod layers;
mod model;
pub mod ops;
mod pred;
mod state;

pub use check::{Level, Problem, check};
#[cfg(feature = "caretline")]
pub use editor::{Editor, find_in};
pub use layers::{GUIDE_Z, Scene, StepPlan, plan_steps, replace_guide, step_layers};
pub use model::{
    Branch, DEFAULT_KIND, END, Narration, Nudge, Place, RESERVED, Step, StepAnchor, StepLayer,
    Tour, parse_json, parse_toml,
};
pub use pred::{Pred, StateTest, TourHost, subset};
pub use state::{
    End, Seen, TourEffect, TourError, TourOp, TourReason, TourState, apply, apply_with, observe,
};
