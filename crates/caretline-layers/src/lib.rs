//! caretline-layers: placement, tracking and lifecycle for layers drawn over a caretline
//! host's screen (hints, callouts, arrows, rings, spotlights). The host draws them.
//!
//! - **Model** ([`Layers`], [`Layer`], [`LayerOp`]): serializable data, changed only by
//!   [`apply`] (an op, the actor, `now_ms`, and the host's policy for agents, [`Limits`]:
//!   none by default) and [`expire`] / [`observe`] (time, edits). No clock, randomness or
//!   I/O: a recorded op sequence replays to the same layers.
//! - **Policy is the host's.** The crate restricts nothing: with [`Limits::default`] agents
//!   may do what the person and the host can, and only malformed input is refused. A host
//!   that wants limits passes its own, or [`Limits::agent_defaults`]. Attribution is the
//!   host's too: each [`Planned`] carries its [`Owner`] ([`Owner::actor`]), and drawing an
//!   agent's layers so they can't pass as the host's own is recommended.
//! - **Content** ([`Content`]): opaque `{kind, data}`, as mark payloads are to the engine.
//!   The host registers a [`Renderer`] per kind, which measures it; drawing is the host's.
//!   One conventional kind, [`HINT`] (`{"title"?, "text"}`), is the one every host should
//!   render.
//! - **Anchors** ([`Anchor`]): stable keys (chars, mark ids, host keys; never screen
//!   positions), resolved to cells every frame by a [`Resolve`]: the host's [`AnchorMap`], a
//!   caretline [`FrameResolver`] (feature `caretline`), or several with [`Chain`]. Text anchors
//!   follow edits through the `ChangeSet` each editor message returns
//!   (`caretline::update_with_changes`; [`observe`], [`map_anchors`]). A `ChangeSet` belongs
//!   to one document, so the host says which views show it ([`Edited`]): only anchors scoped
//!   to those views (and unscoped ones, when it is the focused view's document) move.
//! - **Views:** one document shown in several views gets a [`FrameResolver`] per view, each
//!   with an id, offset and clip, the focused one marked. An anchor scoped to a view
//!   ([`Anchor::scoped`], wire `"in"`) resolves only there; an unscoped one in the focused
//!   view, else the first that shows it, else which way it lies from the focused view.
//!   [`Resolved::view`] says which.
//! - **Placement** ([`plan`]): a pure function of the layers, the resolved anchors, the
//!   [`Grid`] and the sizes the host's renderer measures for each side's room. It returns a
//!   serializable [`Plan`]: each layer's box or
//!   strip, the edge chip of an off-screen anchor, the arrow's route as cells, ring cells,
//!   spotlight holes, and click [`Region`]s with [`Plan::hit`].
//! - **Ops** ([`ops`]): protocol-neutral `hint.*` and `layer.*` requests and replies, for a
//!   host's own JSON protocol or caretline's.
//! - **Pixels** (feature `kitty`, off by default): `kitty::KittyState` turns a plan and the
//!   host's own images into kitty graphics bytes (transmit, place, move, delete), and
//!   `probe` parses a terminal's answers. Still pure: the host rasterises and does the I/O.
//!
//! A host that draws its own screen, every frame:
//!
//! ```ignore
//! let mut grid = Grid::new(w, h).with_area(text_area).with_caret(caret);
//! grid.mark_text(x, y, row_text);                  // what the host drew
//! let plan = plan(&layers, &Chain(vec![&anchors, &editor]), &grid, &renderers);
//! for l in &plan.layers { /* draw l.rect, l.route, l.ring, l.chip; dim plan.spots */ }
//! ```

// `ops::schema` is one `json!` literal, deeper than the default limit.
#![recursion_limit = "256"]

#[cfg(feature = "caretline")]
mod frame;
mod geom;
#[cfg(feature = "kitty")]
pub mod kitty;
mod model;
pub mod ops;
mod place;
#[cfg(feature = "kitty")]
pub mod probe;
mod resolve;
mod route;

#[cfg(feature = "caretline")]
pub use frame::{Edited, FrameResolver, map_anchors, observe};
pub use geom::{Rect, Side};
pub use model::{
    AgentDim, Anchor, Applied, BlockId, Content, HINT, Hint, Layer, LayerOp, Layers, Limits, Owner,
    Part, Pulse, Reason, Refusal, Ring, ScreenPos, Selector, Spotlight, apply, expire,
};
pub use place::{
    AVOID, Attach, CellKind, Edge, Grid, MAX_WIDTH, MeasureCtx, Mode, NARROW_COLS, NARROW_ROWS,
    NoArrow, Plan, Planned, Reach, Region, Renderer, Renderers, Route, Size, Spot, Step,
    hit_regions, plan, width,
};
pub use resolve::{AnchorKey, AnchorMap, Chain, Off, Resolve, Resolved};
pub use route::Dir;
