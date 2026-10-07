//! caretline-overlay: hints, arrows, callouts, rings and spotlights drawn over a caretline
//! host's screen, in text cells.
//!
//! - **Model** ([`Layers`], [`Layer`], [`LayerOp`]): serializable data. Every change goes
//!   through [`apply`] with the actor and `now_ms`, which enforces the agent policy
//!   ([`Limits`]); [`expire`] drops layers whose time is up. No clock is read anywhere, so a
//!   trace of ops replays exactly.
//! - **Anchors** ([`Anchor`]): stable keys (a text range, a block, the caret, a host kind),
//!   resolved to cells every frame by a [`Resolve`]: the host's [`AnchorMap`], a caretline
//!   [`FrameResolver`] (feature `caretline`), or both with [`Chain`].
//! - **Layout** ([`layout`]): layers in, a [`Scene`] out. Pure and serializable, all geometry
//!   in cell coordinates: placement with flip, shift and the sliver rule, strip mode at narrow
//!   sizes, edge chips for off-screen anchors, arrows routed round words, click [`Region`]s.
//! - **Rendering** ([`compose`]): a scene onto any [`CellGrid`], never splitting a wide
//!   grapheme. Dim and ring are [`Flags`] on the host's own cells; overlay cells get
//!   `overlay.*` roles. [`TestGrid`] for tests; `ratatui::buffer::Buffer` with the `ratatui`
//!   feature.
//!
//! A host that draws its own screen calls, every frame:
//!
//! ```ignore
//! let grid = Grid::scan(&buf).with_area(text_area).with_caret(caret);
//! let scene = layout(&layers, &Chain(vec![&anchors, &editor]), &grid, &Opts::new(Glyphs::Rounded));
//! compose(&scene, &mut buf);
//! ```

mod compose;
#[cfg(feature = "caretline")]
mod frame;
mod geom;
mod layout;
mod model;
mod place;
mod resolve;
mod route;
mod text;
#[cfg(feature = "ratatui")]
mod tui;

pub use compose::{CellGrid, Flags, TestCell, TestGrid, compose, put, put_str};
#[cfg(feature = "caretline")]
pub use frame::{FrameResolver, changes_between, map_anchors, observe};
pub use geom::{Rect, Side};
pub use layout::{
    Arrowed, Glyphs, Opts, Panel, PanelKind, Placed, RingMark, RouteCell, Run, Scene, Span,
    TextMeasure, hit, layout, role,
};
pub use model::{
    AgentDim, Anchor, Applied, Arrow, BlockId, Callout, Chip, Item, Layer, LayerOp, Layers, Limits,
    Owner, Part, Pulse, Reason, Refusal, Ring, ScreenPos, Selector, Spotlight, apply, expire,
};
pub use place::{
    CellKind, Grid, MAX_WIDTH, Measure, Mode, NARROW_COLS, NARROW_ROWS, Plan, Planned, Region,
    Route, Spot, Step, hit_regions, plan,
};
pub use resolve::{AnchorKey, AnchorMap, Chain, Off, Resolve, Resolved};
pub use route::Dir;
pub use text::{KeyLabels, NoKeys, str_width, width};
#[cfg(feature = "ratatui")]
pub use tui::{Theme, Themed};
