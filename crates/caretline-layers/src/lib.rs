//! caretline-layers: placement, tracking and lifecycle for layers drawn over a caretline
//! host's screen (hints, callouts, arrows, rings, spotlights). The host draws them.
//!
//! - **Model** ([`Layers`], [`Layer`], [`LayerOp`]): serializable data, changed only by
//!   [`apply`] (an op, the actor, `now_ms`, the [`Limits`] agents are held to) and
//!   [`expire`] / [`observe`] (time, edits). No clock, randomness or I/O: a recorded op
//!   sequence replays to the same layers.
//! - **Content** ([`Content`]): opaque `{kind, data}`, as mark payloads are to the engine.
//!   The host registers a [`Renderer`] per kind, which measures it; drawing is the host's.
//!   One conventional kind, [`HINT`] (`{"title"?, "text"}`), is the one every host should
//!   render.
//! - **Anchors** ([`Anchor`]): stable keys (chars, mark ids, host keys; never screen
//!   positions), resolved to cells every frame by a [`Resolve`]: the host's [`AnchorMap`], a
//!   caretline [`FrameResolver`] (feature `caretline`), or both with [`Chain`]. Text anchors
//!   follow edits through a `ChangeSet` ([`map_anchors`]).
//! - **Placement** ([`plan`]): a pure function of the layers, the resolved anchors, the
//!   [`Grid`] and the measured sizes. It returns a serializable [`Plan`]: each layer's box or
//!   strip, the edge chip of an off-screen anchor, the arrow's route as cells, ring cells,
//!   spotlight holes, and click [`Region`]s with [`Plan::hit`].
//! - **Ops** ([`ops`]): protocol-neutral `hint.*` and `layer.*` requests and replies, for a
//!   host's own JSON protocol or caretline's.
//!
//! A host that draws its own screen, every frame:
//!
//! ```ignore
//! let mut grid = Grid::new(w, h).with_area(text_area).with_caret(caret);
//! grid.mark_text(x, y, row_text);                  // what the host drew
//! let plan = plan(&layers, &Chain(vec![&anchors, &editor]), &grid, &renderers);
//! for l in &plan.layers { /* draw l.rect, l.route, l.ring, l.chip; dim plan.spots */ }
//! ```

#[cfg(feature = "caretline")]
mod frame;
mod geom;
mod model;
pub mod ops;
mod place;
mod resolve;
mod route;

#[cfg(feature = "caretline")]
pub use frame::{FrameResolver, changes_between, map_anchors, observe};
pub use geom::{Rect, Side};
pub use model::{
    AgentDim, Anchor, Applied, BlockId, Content, HINT, Hint, Layer, LayerOp, Layers, Limits, Owner,
    Part, Pulse, Reason, Refusal, Ring, ScreenPos, Selector, Spotlight, apply, expire,
};
pub use place::{
    CellKind, Grid, MAX_WIDTH, Mode, NARROW_COLS, NARROW_ROWS, Plan, Planned, Region, Renderer,
    Renderers, Route, Size, Spot, Step, hit_regions, plan, width,
};
pub use resolve::{AnchorKey, AnchorMap, Chain, Off, Resolve, Resolved};
pub use route::Dir;
