# Layers: hints, callouts and spotlights

> **New and not released.** [`caretline-layers`](../crates/caretline-layers) is in the
> repository but not yet on crates.io, and its API may change before its first release. This
> page covers what is built (steps 1a and 1b of the [design](../docs/design/layers.md):
> placement, views, and the kitty plumbing for pixels in Ghostty). The engine hooks it will
> build on are in caretline (view values, frame passes, `view::locate`, host ops: see
> [embedding.md](embedding.md#extending-the-engine)); the next steps (in-frame mode through an
> `install(host)` adapter, protocol and MCP tools) are listed in the design's sections 9 and
> 12.5. Walkthroughs built on it are [`caretline-tour`](tour.md).

`caretline-layers` puts things **over** a host's screen: a hint box beside a word, an arrow
to a table row, a ring round a block, a spotlight that dims everything else. It decides where
they go and keeps them pointing at the right text as the view scrolls, wraps and changes. It
draws nothing.

| caretline-layers does | The host does |
|---|---|
| Keeps the layers as data, changed only by ops | Keeps that data in its own state, beside everything else, and sets any policy for agents (`Limits`) |
| Resolves anchors (chars, blocks, the caret, the host's own keys) to cells every frame | Says where it drew things: a caretline frame, or an `AnchorMap` it fills while drawing |
| Places each box, strip, edge chip, arrow, ring and spotlight hole, and the click regions | Measures its content (`Renderer::measure`, `Renderer::chip`) and draws every part in its own style, attribution included |
| Moves text anchors through edits; drops expired layers | Feeds it each message's changes (`update_with_changes`) and the time |
| Parses `hint.show`, `layer.push` and friends from JSON | Routes them from its own protocol, decides who is an agent |

Hosts differ in look, imagery and content, so nothing that decides how a layer looks is in a
caretline crate: no theme, glyphs, colours or roles.

## Add it

Until it is published, depend on it from a checkout or by git revision:

```toml
[dependencies]
caretline-layers = { git = "https://github.com/brancusi/caretline", rev = "<full sha>" }
```

The default feature `caretline` adds what needs the engine: `FrameResolver`, `Grid::from_frame`
and mapping anchors through the `ChangeSet` each editor message returns
(`caretline::update_with_changes`, which needs a caretline release newer than 0.3.0; until then
depend on caretline by git revision too). Without it (`default-features = false`) the crate
needs only serde, for a host that draws no caretline editor at all. The `kitty` feature (off
by default) adds the pixel plumbing ([Pixels in Ghostty](#pixels-in-ghostty)) and only
`miniz_oxide`.

## The model

A **layer** is serializable data:

```json
{"id": "h-1", "owner": "agent:helper", "z": 20, "since_ms": 0, "ttl_ms": 8000,
 "anchor": [{"text": {"from": 412, "to": 421}}, {"block": 7}],
 "content": {"kind": "hint", "data": {"title": "Jump by word", "text": "Alt-arrows move one word."}},
 "arrow": true, "ring": {}, "place": ["below", "above"]}
```

| Field | Meaning |
|---|---|
| `id` | Given by `apply` when pushed empty |
| `owner` | `host` (the default), `person`, `guide` or `agent:<name>`. It decides the z band, and which of the host's limits apply |
| `z` | Draw order within the owner's band: `host` 0–9, `guide` 10–19, `agent` 20–29, `person` 30–39 |
| `since_ms`, `ttl_ms` | When it was pushed (the push's `now_ms`) and how long it lasts; `null` until popped (a host's `Limits` may give agents' layers a default and bounds) |
| `anchor` | Fallbacks, in order: the first that resolves is used ([Anchors](#anchors)) |
| `content` | `{kind, data}`, opaque to the crate; none for a layer that only rings or dims |
| `arrow` | An arrow from the box to the anchor |
| `ring` | Mark the anchor's cells (`{"pulse": {"period_ms", "cycles"}}` for a host with a frame clock) |
| `spotlight` | Dim everything but the holes (`{"holes": ["anchor", "box"]}` by default) |
| `capture` | A modal step, for [walkthroughs](tour.md) (a host may refuse it to agents: `no_agent_capture`) |
| `hide_off_screen` | With the anchor off screen, show only the edge chip |
| `place` | The sides to try for the box (default below, above, right, left) |
| `max_width` | The widest the box may be (default 52, never over two-thirds of the area) |
| `avoid` | Anchors whose cells this layer's box and arrow keep off: the text its step talks about. Resolved every frame like `anchor` (every one that shows), so they follow scrolling and edits; covered only when nothing else fits ([Avoid areas](#avoid-areas)) |

`Layers` holds them (and `hidden`, for "hide all"). In Rust, `Layer::new(anchor)` with
`with_content`, `with_arrow`, `with_ring`, `with_spotlight` and `with_avoid` builds one.

Layers change only through **`apply(&mut layers, op, actor, now_ms, &limits)`**, with a
`LayerOp`: `Push`, `Update`, `Pop` (by id, owner or all), `PopNewest`, `Toggle` or `Clear`.
`actor` is `None` for the person or the host and `Some(name)` for an agent; it returns
`Applied` (the layer id, what was popped) or a `Refusal` with a stable `Reason`. Time enters
only as `now_ms`: `expire(&mut layers, now_ms)` drops expired layers, and `observe` does that
and maps anchors through an edit. A recorded op sequence replays to the same layers.

### What agents may do: the host's policy

The crate imposes no policy. With `Limits::default()` (also `Limits::none()`) an agent may do
what the person and the host can: any number of layers, any size, no rate limit or forced
expiry, spotlights and capture, and touching any layer. `apply` then refuses only malformed
input: no anchor, nothing to show, a duplicate or unknown id, a bad actor name. Whatever the
limits, an agent's layer is always owned by it (`agent:<actor>`, whatever owner it claims) and
sits in the agent z band.

A host that wants limits sets them, field by field (each is an `Option`, or a flag, that is off
by default and set alone), or starts from **`Limits::agent_defaults()`**, the values the design
started from:

| `Limits` field | `agent_defaults()` | Refused as |
|---|---|---|
| `agent_layers` | 3 per view; a push past them replaces that actor's oldest | `too_many` (the actor has none to replace) |
| `ttl_default_ms`, `ttl_min_ms`, `ttl_max_ms` | 8 s if not given, clamped to 1–60 s | (clamped, not refused) |
| `per_second` | 2 pushes (or updates) a second per actor, by `now_ms` | `rate_limited` |
| `body_chars`, `body_lines`, `title_chars` | A hint's text at most 280 characters and 6 lines, its title 40 | `too_long` |
| `content_bytes` | Other content at most 1 KB of JSON | `too_long` |
| `agent_dim` | `AgentDim::Never`: no spotlight (the default without a policy is `Always`) | `dim_not_allowed` |
| `no_agent_capture` | `true` | `capture_not_allowed` |
| `own_layers_only` | `true`: an agent pops and updates only its own layers; dismissing, hiding and clearing are the person's | `not_allowed` |

```rust
use caretline_layers::{AgentDim, Limits};

let open = Limits::default();                    // no policy
let strict = Limits::agent_defaults();           // the design's values
let mine = Limits { agent_layers: Some(5), agent_dim: AgentDim::Never, ..Limits::default() };
```

Two things hold whatever the limits: placement never puts an agent's box over the caret or
protected cells, and never scrolls the view.

**Attribution is the host's.** The crate writes nothing into an agent's content: each placed
layer carries its `owner` (`Planned.owner`, with `Owner::actor()` for the agent's name, and
`Planned.agent`). Drawing an agent's layers so they can't pass as the host's own (its name in
the border, a colour of its own) is recommended, in the host's own style; the
[walkthrough](#walkthrough-a-ratatui-host) puts `◆ helper` in the box's top border.

## Anchors

An anchor is a stable key, never a screen position, so it survives scrolling, resizing and
edits.

| Anchor | JSON | Rust |
|---|---|---|
| Text range (document chars) | `{"text": {"from": 412, "to": 421}}` | `Anchor::Text { from, to }` |
| Text within a block | `{"text": {"block": 7, "from": 4, "to": 9}}` | `Anchor::BlockText { block, from, to }` |
| A block (its mark id) | `{"block": 7}` | `Anchor::Block(7)` |
| The caret | `{"caret": true}` | `Anchor::Caret` |
| A screen position | `{"screen": "center"}` (`top`, `bottom`) | `Anchor::Screen(ScreenPos::Center)` |
| **A host kind** | `{"host": {"kind": "row", "key": "r-104"}}` | `Anchor::Host { kind, key }` |
| A text, block or caret anchor **in one view** | `{"text": {"from": 4, "to": 9}, "in": "panel:2"}` | `Anchor::scoped("panel:2", a)` ([several views](#one-document-several-views)) |

Host kinds are the host's own things: a table row by id, a list item, a diff line, a tab.
caretline never interprets the kind or the key.

**Resolving.** Every frame a `Resolve` turns anchors into cells (`Resolved`: rects, or which
way an off-screen anchor lies):

- **`AnchorMap`**: the host records where it drew each of its things, while drawing:
  `anchors.put(AnchorKey::host("row", "r-104"), Rect::new(4, 6, 29, 1))`, or `put_off` for one
  scrolled out of sight.
- **`FrameResolver`** (feature `caretline`): text, block and caret anchors from a caretline
  `Frame` drawn at (`x`, `y`): `FrameResolver::new(&frame).at(x, y).with_doc(&doc)`. It reads
  only the visible rows; with the document it also says which way an off-screen block lies.
  For a document in several views, each view's resolver gets an id and a clip
  ([below](#one-document-several-views)).
- **`Chain`**: several, all asked: `Chain(vec![&anchors, &editor])`. The first answer whose
  cells show wins (the focused view's first), else the first that says which way it lies.

**Following edits.** Block anchors need nothing: marks follow their blocks. Text anchors move
with the text. The engine hands out each message's text changes: `update_with_changes` (and
`update_doc_with_changes`, `Session::apply_with_changes`) returns them beside the effects, as one
`ChangeSet` from the text before the message to the text after, or `None` when the text didn't
change. Pass them to `observe` after every message:

```rust
use caretline::{update_with_changes, Msg, State, Viewport};
use caretline_layers::{apply, observe, Anchor, Content, Edited, Layer, LayerOp, Layers, Limits};

let mut editor = State::new("hello world", None, Viewport { width: 40, height: 6 });
let mut layers = Layers::default();
let hint = Layer::new(Anchor::Text { from: 6, to: 11 }).with_content(Content::hint(None, "this word"));
let now_ms = 0;
apply(&mut layers, LayerOp::Push(hint), None, now_ms, &Limits::default()).unwrap();

let (_effects, changes) = update_with_changes(&mut editor, Msg::InsertText { text: "big ".into() });
// One document: every text anchor is in it. Maps them and drops expired layers.
observe(&mut layers, Edited::All, changes.as_ref(), now_ms);
assert_eq!(layers.layers[0].anchor[0], Anchor::Text { from: 10, to: 15 });
```

The start of a range sticks after an insertion there, the end before one. A range whose text
was deleted is dropped and the next fallback takes over; a layer with no anchor left goes.
`map_anchors(&mut layers, edited, &changes)` does the mapping alone.

**Which anchors an edit moves.** A `ChangeSet` belongs to one document, so `observe` and
`map_anchors` take an `Edited` that says which anchors point into the document that changed:

| `Edited` | Moves | For |
|---|---|---|
| `Edited::All` | Every text anchor, scoped or not | A host with one document, in one view or several |
| `Edited::Views { views, unscoped }` | Anchors scoped to one of `views`; unscoped anchors only when `unscoped` | A host with several documents: `views` are the ids of every view showing the edited document, and `unscoped` is `true` when it is the focused view's document (unscoped anchors resolve in the focused view first, so they belong to it) |

Anchors on another document are left as they are. A host that shows several documents scopes
its anchors ([`in`](#one-document-several-views)), so each names the document it points
into.

## One document, several views

A host can show one document in several views: a main editor and a side panel, each a
caretline `View` drawn at its own offset and clipped to its own rect. Give each view a
`FrameResolver` with a stable id (the host's own names), its offset and its clip, mark the
focused one, and put them in one `Chain`:

```rust
let main = FrameResolver::new(&main_frame).with_doc(&doc).id("main").focused();
let panel = FrameResolver::new(&panel_frame)
    .with_doc(&doc)
    .at(62, 2)
    .id("panel:2")
    .clip(Rect::new(62, 2, 36, 9)); // the cells the panel shows on the host's screen
let anchors = Chain(vec![&host_anchors, &main, &panel]);
```

- **The clip.** Cells outside a view's clip aren't visible. An anchor whose cells are all
  clipped away lies off screen the way they are (above or below with its column, left or
  right with its row), and a direction the frame gives (scrolled out of the view) is pulled
  inside the clip, so the edge chip sits by that view.
- **Scoped anchors.** A text, block or caret anchor can name its view with `in`:
  `{"text": {"from": 4, "to": 9}, "in": "panel:2"}`, or `Anchor::scoped("panel:2", anchor)` in
  Rust (`Anchor::In { view, anchor }`; `view()` and `unscoped()` take it apart). It resolves
  only in the resolver with that id and never falls back to another view (a resolver without
  an id answers only unscoped anchors). A screen or host anchor can't be scoped (a host key
  names its own place), and serde and `apply` refuse an empty view, a second scope or any
  other key beside the target.
- **Unscoped anchors** resolve in this order (`Chain`): the focused view if it shows the
  anchor; else the first resolver, in order, that shows it; else which way it lies from the
  focused view; else the first direction any resolver gives. A resolver that says "off
  screen" doesn't hide a later one that shows it. A host's own resolver marks itself focused
  with `Resolve::is_focused` (default `false`).
- **Which view answered.** `Resolved.view` (wire `in`) names the resolver that answered, so
  `Planned.anchor` and the replies of `ops::reply` and `ops::resolved` say where the layer
  landed: `{"rects": […], "in": "panel:2"}`.
- **Edits through any view.** Every view of a document shows the same text, so the
  `ChangeSet` of an edit made through any of them (`update_doc_with_changes`) maps the
  anchors scoped to all of them. Say which views show the edited document:
  `Edited::Views { views: &["main", "panel:2"], unscoped: true }` (or `Edited::All` when the
  host has only this document).
- **Several documents.** When `panel:1` shows another document, an edit to the main one
  passes `Edited::Views { views: &["main"], unscoped: true }`, and a hint scoped to `panel:1`
  stays where it is; an edit made in `panel:1` passes `Edited::Views { views: &["panel:1"],
  unscoped: false }` when `main` is focused.

## Content and `Renderer::measure`

Content is `{kind, data}`, as opaque to the crate as mark payloads are to the engine. The host
registers a **`Renderer`** per kind. Placement needs only its size:

```rust
use caretline_layers::{MeasureCtx, Renderer, Renderers, Size, HINT};

struct HintBox;

impl Renderer for HintBox {
    // The box for `cx.data`, borders included, at most `cx.avail`. A zero size means no box.
    fn measure(&self, cx: &MeasureCtx) -> Size {
        let text = cx.data["text"].as_str().unwrap_or("");
        // An agent's layer gets a "from <actor>" row, so the attribution fits in the box.
        let by = cx.owner.actor().map(|a| a.chars().count() + 5);
        let w = (text.chars().count().max(by.unwrap_or(0)) as u16 + 4).min(cx.avail.w);
        Size::new(w, 3 + u16::from(by.is_some()))
    }
}

let renderers = Renderers::new().register(HINT, HintBox);
```

`MeasureCtx` carries the layer's content `data`, the room `avail`, and its `owner`
(`Owner::actor()` for an agent's name), so a host sizes whatever it draws for the owner
inside the box: a name in the border, an attribution line. Build one with
`MeasureCtx::new(data, avail, owner)` (in a renderer's own tests, say).

`measure` is called once for each side placement tries, with that side's room as `cx.avail`:
below and above, the rows between the anchor (past the arrow's gap) and the area's edge, at
most the width cap (`max_width`, never over two-thirds of the area) wide; right and left, the
columns past the gap, at most the cap. A side with no room isn't measured. Honour `avail`: a
renderer that wraps its text to `avail.w` gives a narrow, tall box where only a narrow side is
free, and a box that keeps its size whatever room it gets is placed as before.

`measure` must be pure: the same data, room and owner give the same size, or a replay places
boxes differently. A closure `Fn(&Value, Size) -> Size` (the data and the room) is a renderer
too. A layer whose kind has no
renderer is placed without a box and listed in `Plan.unrendered`.

**`chip(data, anchor, off)`** sizes the edge chip shown where an off-screen anchor lies (one row
high; 8 by 1 by default). It gets the layer's data, the anchor that lies off screen and which
way (`Off`), so a host can size a label such as `↓ 2/11 here` to fit. It must be pure too:

```rust
use caretline_layers::{Anchor, MeasureCtx, Off, Renderer, Size};
use serde_json::Value;

struct HintBox;

impl Renderer for HintBox {
    fn measure(&self, cx: &MeasureCtx) -> Size {
        Size::new(cx.avail.w.min(30), 3)
    }
    fn chip(&self, _data: &Value, _anchor: &Anchor, off: Off) -> Size {
        let arrow = match off {
            Off::Above { .. } => "↑",
            Off::Below { .. } => "↓",
            Off::Left { .. } => "←",
            Off::Right { .. } => "→",
        };
        let label = format!("{arrow} here");
        Size::new(label.chars().count() as u16 + 2, 1)
    }
}
```

### The `hint` kind

One kind is conventional: **`hint`**, with data `{"title"?: string, "text": string}`
(`Content::hint(title, text)`, `Content::as_hint`). Every host should render it, in its own
style, so an agent can point at something in any host without knowing how that host looks.
Draw the text near the placed rect, attribute an agent's hint so it can't pass as the host's
own (recommended: [attribution](#what-agents-may-do-the-hosts-policy)), and treat the box as a
click region. The arrow and the ring are optional. Other kinds are the
host's, namespaced `<host>.<name>`.

## `plan()`

```rust
let p = plan(&layers, &anchors, &grid, &renderers);
```

A pure function of the layers, the resolved anchors, the screen and the measured sizes. The
**`Grid`** is the screen as placement sees it:

- `Grid::new(w, h)`, `with_area(rect)` (where layers may go: the text rows, never the status
  row), `with_caret(Some((x, y)))`, `with_protect(rects)` (cells no box may cover: a selection,
  a prompt);
- `mark_text(x, y, s)` for each string the host drew, so boxes avoid covering text and arrows
  go round words, and wide graphemes are never split;
- `Grid::from_frame(&frame)` and `mark_frame(&frame, x, y)` for a caretline frame;
- `avoid(rect, weight)` / `with_avoid(rect, weight)` for cells to keep off when anything else
  fits (`avoid_weight(x, y)` reads a cell's back), and `with_reach(Reach { rows, cols })` for how far a box may go to keep clear
  ([Avoid areas](#avoid-areas)).

The **`Plan`** (serializable) holds, per layer in draw order (`Planned`):

| Field | What the host draws |
|---|---|
| `rect`, `mode`, `side` | The box (`mode: box`, beside the anchor on `side`) or a one-row strip (`mode: strip`: the area is under 48 columns or 12 rows, or no box fits). A strip goes on the edge nearest its anchor: the area's bottom for one that lies below (the last free row, so above the chip or a protected last row), its top for one above, and for one on screen the top, unless the anchor is on the top row |
| `anchor` | Where the anchor resolved: rects, or `off` (above, below, left, right), and `in`, the view that answered |
| `chip` | The edge chip on the edge an off-screen anchor lies beyond |
| `dock` | A box for an off-screen anchor docks against its own chip: it sits next to the chip and shares part of its edge, and `dock` (`Attach { edge, offset }`) names a cell of that shared edge on the box's border, so the host joins the two there. When no box can touch the chip, the layer is a strip |
| `route` | The arrow: `junction` on the box's border and the same place as `attach` (`Attach { edge, offset }`: which edge, and how many cells along it from the corner), then `steps`, one cell each with the direction it enters and leaves; the last is the head. Lay out the border round `attach`: a title never sits where the arrow leaves. The head is always on a clear cell beside the anchor: blank, with nothing beside it on its row but the anchor, never on a letter, a hyphen inside a word or the gap between two words |
| `no_arrow` | Why a layer that asked for an arrow has none: `docked` (the anchor is off screen; the chip points the way), `screen` (a screen position), `no_box` (a strip, or no box), `no_way` (no route round the other layers, holes, protected cells and wide graphemes), `head_on_text` (every cell beside the anchor is text or between words, as in the middle of a tight list: the arrow is dropped rather than drawn over the text; the box and ring still mark the anchor) |
| `ring` | The anchor's cells to mark |
| `covers_avoid` | How many of the box's cells are avoid cells: 0 (and left out of the JSON) unless no box in reach kept off them all, so a host's lint can flag it |
| `owner`, `agent` | Whose layer it is (`Owner::actor()` gives an agent's name), and whether it is an agent's: what the host attributes it by |

and for the whole screen: `spots` (each spotlight's area and holes; `Plan::dimmed(x, y)` says
whether a cell is dimmed), `regions` with `Plan::hit(x, y)` (a box is `<layer>`, an edge chip
`<layer>/reveal`; a host adds its own buttons as `<layer>/<name>`), and `missing` (layers none
of whose anchors resolved).

Boxes are placed in z order, each scored over the sides it may take (measured for each side's
room): text covered, distance to
the anchor and, with an arrow, the arrow's route; never over its anchor or any other layer's
(every layer's anchor is known before any box is placed), another layer's box, chip, strip or
arrow, a hole, protected cells or (for agents) the caret. No arrow runs under a box, and chips
on one edge slide along it to a free place. A box whose arrow routes beats one whose arrow
can't; when the side of the anchor facing the box can't be reached, the arrow may end beside
the anchor on another side, pointing at it, and on a left or right edge it never leaves beside
the title row. Ties go to the first side listed, which gives flip; a box shifts along its side
to stay inside the area, reaches the edge rather than leave a sliver of words beside it, and
falls back to a strip when nothing fits.

### Avoid areas

`protect` is hard: no box covers those cells, ever. Some cells a box should only keep off
when it can: the highlighted lines of a diff, the table rows a step explains. Mark those
**avoid**:

```rust
use caretline_layers::{Grid, Layer, Rect, AVOID};

let mut grid = Grid::new(100, 40).with_area(Rect::new(0, 0, 100, 39));
grid.avoid(Rect::new(0, 12, 100, 6), AVOID); // the highlighted band, full width
// Or let the layer name what its step talks about, resolved every frame:
let layer = Layer::new(anchor).with_avoid(vec![band_anchor]);
```

- **Boxes.** A box that covers no avoid cell beats every box that covers one; among those
  that must, the least weight covered wins (`weight` is in text cells: covering a text cell
  costs 1, an avoid cell `AVOID` = 20 by default; overlapping rects keep the heavier). A
  layer's own `avoid` cells weigh `AVOID`. `Planned.covers_avoid` counts the cells a box
  covered when nothing in reach kept clear.
- **Further out.** Each side tries the nearest four places first. When none of them is clear
  of text and avoid cells (or protected cells block them), the side keeps going out, up to
  the grid's reach (default 12 rows above or below, 40 columns beside), taking only clear
  places, and stops at the first row or column that has one; the arrow bridges the gap. A
  step farther costs 5 tenths of a text cell, so near and clear beats far and clear, and far
  and clear beats near and covering.
- **Arrows** pay an avoid cell's weight on top of its cost (4 per unit of weight: 80 at the
  default, against 16 for a text cell and 6 for a gap), so they go round avoid cells, and
  words, whenever a way round exists in their corridor. The head still ends on a clear cell
  beside the anchor; when the anchor sits inside an avoid band, the head is in the band too,
  as briefly as the route allows.

With an arrow, the least-scoring box of every candidate wins; boxes are routed a side at a time,
and a box that couldn't win even with the cheapest arrow is never routed. Placement is quick
enough for every frame: at 100×40 (release build) a box costs about 6 µs (a measure per side),
a box with its arrow about 25 µs, and a spotlight with an arrow about 37 µs.

## Ops for your protocol

`ops` turns requests from any JSON protocol into ops, and results back into JSON. The host
keeps its own transport and routing (`op` and `view` are its fields):

| Op | Request fields | Becomes |
|---|---|---|
| `hint.show` | `anchor` (one or a list), `text`, `title?`, `ttl_ms?`, `place?`, `arrow?` (default true), `ring?` (default true), `avoid?` (one anchor or a list), `actor?` | A push of a `hint` layer |
| `hint.hide` | `layer` or `all: true`, `actor?` | A pop |
| `layer.push`, `layer.update` | `layer` (a layer as above), `actor?` | A push or an update |
| `layer.pop` | `layer`, `owner` or `all: true`, `actor?` | A pop |
| `layer.list` | `actor?` | Nothing applied; `ops::list` answers |

```rust
let (req, actor) = ops::parse("hint.show", &request)?;     // Err: a Refusal
if let ops::Request::Apply(op) = req {
    let applied = apply(&mut layers, op, actor.as_deref(), now_ms, &limits)?;
    let reply = ops::reply(&applied, Some(&plan));          // {"layer", "resolved", …}
}
```

`actor` names the agent a request comes from; without it the request is the host's own. A
protocol client is normally an agent, so a host may set it itself. `ops::reply` with this
frame's plan says where the layer landed (`{"rects": …}`, `{"off": "below"}`, or `null` with
`reason: "not_found"`); `ops::error` turns a refusal into `{"error": {"reason", "detail"}}`.

**`ops::schema()`** returns a JSON Schema (draft 2020-12, a `serde_json::Value`) for the wire:
every request `ops::parse` accepts, one per op with `op`, `actor` and the host's `id` and
`view` (`$defs/request`), and every reply (`$defs/reply`, `$defs/resolved`, `$defs/list`,
`$defs/error`), with the parts (`anchor` and its `in`, `layer`, `content`, `hint`, `owner`,
`side`, `rect`) as `$defs` too. The document as a whole matches any request or reply. Publish
it with your protocol, or hand it to an agent's tool definitions. It describes the shape
`parse` reads; what `apply` then refuses (no anchor, nothing to show, your `Limits`) isn't in
it. The crate's tests check it against the ops tests' requests, real replies and the design's
protocol examples.

## Purity: the rules for a host

The crate has no clock, randomness, I/O, terminal or async, and the host keeps it that way:

- **One state.** Keep `Layers` inside your app's state, not in a second store. Change it only
  with `apply`, `expire` and `observe`.
- **Time from your messages.** Pass the `now_ms` your messages carry, never a wall-clock read
  inside a reducer, so expiry and rate limits replay.
- **Feed every message.** Call `observe` with the changes `update_with_changes` returned for
  each message (`None` for one that edited nothing still expires layers), and the `Edited`
  that says which views show the document it changed, so text anchors stay on their text and
  anchors in other documents stay put.
- **Resolve and plan every frame.** Rects, routes, holes and regions are derived; store none of
  them.
- **Pure renderers.** `measure` depends only on its arguments. Replaying needs the same
  renderers registered, as it needs the same host commands.

## Walkthrough: a ratatui host

[`examples/layers_ratatui.rs`](../crates/caretline-layers/examples/layers_ratatui.rs) is a
host that draws a table of documents with ratatui and lets an agent point at a row:

```console
$ cargo run -p caretline-layers --example layers_ratatui            # s: spotlight, q: quit
$ cargo run -p caretline-layers --example layers_ratatui -- --print
  Documents

    Quarterly report        ready
    Release checklist       draft
    Onboarding guide        ready
    Style sheet             stale
    Archive index ▲         ready
       ╭──────────┴─ ◆ helper ───────────────────────╮
       │ Out of date                                 │
       │ This document changed upstream since it was │
       │ last opened.                                │
       ╰─────────────────────────────────────────────╯
```

Each frame it:

1. **Draws its own screen** and records it: every row's cells go into an `AnchorMap` under its
   stable id, and every string into the `Grid`:

   ```rust
   anchors.put(AnchorKey::host("row", id), Rect::new(4, y, row.chars().count() as u16, 1));
   grid.mark_text(4, y, &row);
   ```

2. **Takes the agent's request** as it would arrive over its protocol, and applies it:

   ```rust
   let req = json!({"op": "hint.show", "actor": "helper",
       "anchor": {"host": {"kind": "row", "key": "r-104"}},
       "title": "Out of date", "text": "This document changed upstream since it was last opened.",
       "ttl_ms": 60000});
   let (Request::Apply(op), actor) = ops::parse("hint.show", &req)? else { unreachable!() };
   apply(&mut layers, op, actor.as_deref(), now_ms, &Limits::default())?;
   ```

   `Limits::default()` is no policy: this host trusts its agent. One that doesn't passes
   `Limits::agent_defaults()` or its own.

3. **Plans**, with its `hint` renderer registered (a bordered box: the wrapped text plus four
   columns and two rows, and for an agent's layer at least as wide as the `◆ <actor>` label
   it draws in the border, from `MeasureCtx::owner`):

   ```rust
   let renderers = Renderers::new().register(HINT, HintBox);
   let p = plan(&layers, &anchors, &grid, &renderers);
   ```

4. **Draws the plan** its own way: dims the cells `p.dimmed(x, y)` reports, shades the `ring`
   cells, draws the arrow from `route.steps` (a line glyph per step from its `enter` and
   `leave` directions, a head at the last), then the box in `rect` with a `┴` at the route's
   `junction`, and the hint's title and text inside. In `Mode::Strip` it draws the text on one
   row. It attributes the agent's layer itself, clear of where the arrow attaches:

   ```rust
   if let Some(actor) = l.owner.actor() {
       let label = format!(" ◆ {actor} ");
       let mut at = 2;
       if let Some(a) = l.route.as_ref().map(|rt| rt.attach)
           && a.edge == Edge::Top
           && (at..at + label.chars().count() as u16).contains(&a.offset)
       {
           at = a.offset + 2;
       }
       buf.set_string(r.x + at, r.y, label, border);
   }
   ```

The box went below the row because it fits there; at another size it flips above or becomes a
strip, and the host's drawing code doesn't change. If the table scrolled `r-104` out of sight,
the host would `put_off` its key and the plan would give an edge chip, with the box docked
against it (`dock`) and `no_arrow: docked`, instead.

A host that draws a caretline editor rather than a table does the same with a
`FrameResolver` for its anchors, and after every message passes the changes
`update_with_changes` returned to `observe` ([Following edits](#anchors)).

## Pixels in Ghostty

With the `kitty` feature, a host that draws in Ghostty can show its layers in pixels: a panel
with a soft shadow under a box's words, an anti-aliased arrow, a ring, a translucent veil for a
spotlight. The crate still draws nothing. It turns the host's images into kitty graphics bytes
and keeps them cheap: pixels are sent once, a scroll only moves them, and a frame where nothing
changed sends nothing.

```toml
caretline-layers = { git = "https://github.com/brancusi/caretline", rev = "<full sha>", features = ["kitty"] }
```

| The host supplies | `kitty` does |
|---|---|
| A `Picture` per layer part (`"panel"`, `"arrow"`, …): straight-alpha RGBA from its own raster, a shape key, the cells it covers, `Z::Below` or `Z::Above` the text, an optional clip | Transmits what the terminal lacks, re-places what moved (`a=p`, no pixels), places a changed part before deleting the old one, deletes what's gone by id, crops a picture that reaches past the screen or its clip |
| The cell size in device pixels (`CellPx`), every frame | Ids from content: the image id hashes the shape key and the cell size, the placement id the (layer, part). The same frames give the same bytes |
| The writes, inside the frame's synchronized update | `Output.bytes`, plus what it sent, placed and deleted |
| For `t=t`, a `TempFiles` writer | The file name Ghostty accepts (it contains `tty-graphics-protocol`) |

**Each frame**, after `plan`:

1. Draw the text frame. In a box drawn in pixels, write only the words (the panel image is the
   box); leave arrows, rings and dimmed text alone, since the images go over them.
2. Build the pictures. Ask `kitty.holds(key, cell)` first: if the terminal already has a key's
   pixels, pass `image: None` and skip the raster.
3. Call `kitty.frame(&plan, &pictures, cell, files)` and write `Output.bytes` **after the
   text, before the synchronized update (DEC mode 2026) ends**, so text and pixels change
   together.

```rust
use std::io::{self, Write};

use caretline_layers::kitty::{CellPx, Cells, Image, KittyState, Picture, Z, shape_key};
use caretline_layers::{Mode, Plan};

/// One flat panel image under a box: the host's own raster (here, a solid fill).
fn raster_panel(cols: u16, rows: u16, cell: CellPx) -> Option<Image> {
    let (w, h) = (cols as u32 * cell.w as u32, rows as u32 * cell.h as u32);
    Image::new(w, h, [31u8, 36, 48, 255].repeat((w * h) as usize))
}

/// The pictures for this frame: one panel per box.
fn pictures(plan: &Plan, cell: CellPx, kitty: &KittyState) -> Vec<Picture> {
    let mut out = Vec::new();
    for l in &plan.layers {
        let (Some(r), Some(Mode::Box)) = (l.rect, l.mode) else { continue };
        // Everything the pixels depend on, and a version of your drawing code.
        let key = shape_key(&[b"my-panel-v1", &cell.w.to_le_bytes(), &cell.h.to_le_bytes(),
                              &r.w.to_le_bytes(), &r.h.to_le_bytes()]);
        let image = if kitty.holds(key, cell) { None } else { raster_panel(r.w, r.h, cell) };
        out.push(Picture { layer: l.id.clone(), part: "panel".into(), key, z: Z::Below,
                           at: Cells::from(r), clip: None, image });
    }
    out
}

fn paint(out: &mut impl Write, plan: &Plan, cell: CellPx, kitty: &mut KittyState) -> io::Result<()> {
    let pics = pictures(plan, cell, kitty);
    let px = kitty.frame(plan, &pics, cell, None);
    out.write_all(b"\x1b[?2026h")?;   // begin the synchronized update
    // ... the text frame: each box's cells hold only its words ...
    out.write_all(&px.bytes)?;        // then the pixels, in the same update
    out.write_all(b"\x1b[?2026l")?;   // end it
    out.flush()
}
```

A shape key must change whenever the pixels would: hash the sizes, the route's points, the
holes, the cell size and a version of your drawing code with `shape_key` (FNV-1a, the same on
every machine). A tall image, such as a veil a few screens high, can stay put while the view
scrolls: give it `at` cells that reach past the screen and the crate re-crops it.

**`KittyState` is a renderer cache, never state.** It remembers what the terminal holds,
derived only from the frames you drew. Keep it beside your terminal writer, not in your app's
state or a trace. Its lifecycle:

- `KittyState::new(Options::default())`: `t=d`, zlib level 6, base64 in 4096-byte chunks, which
  works over SSH. `Transport::File` (`t=t`) sends about a fifteenth of the bytes for a local
  session; it needs your `TempFiles` and falls back to `t=d` for a file it couldn't write.
- `clear()` returns the bytes that delete every image placed, by id (never `d=A`: the screen may
  hold other programs' images), and forgets them. Write them on exit, when pixels are turned
  off, and while something else covers the screen (the CLI does it for its keys overlay).
- `reset()` forgets without deleting: for when the terminal no longer holds what you placed
  (it was reset, or another program deleted everything). The next frame re-sends it all.
- A new cell size (a font change, or a move to a screen with another scale) needs neither: the
  ids include the cell size, so the next frame sends new images and deletes the old ones.

**Probing.** Pixels are on only when the terminal says so. `probe::request()` asks four things
in one write: a kitty graphics query, XTVERSION, the cell size (`CSI 16 t`) and DA1, which
every terminal answers, as the fence. The replies arrive on stdin among the person's keys, so
run them through `probe::scan` in your input parser: a `Reply` with its length, `Partial`
(wait), or `No` (a key):

```rust
use caretline_layers::probe::{self, Probe, Scan};

/// Takes the probe's replies out of what the terminal sent; the rest are keys.
/// Returns how many bytes it used: keep the rest for the next read.
fn take_replies(input: &[u8], probe: &mut Probe, keys: &mut Vec<u8>) -> usize {
    let mut i = 0;
    while i < input.len() {
        match probe::scan(&input[i..]) {
            Scan::Reply(reply, n) => {
                probe.add(&reply);
                i += n;
            }
            Scan::Partial => break, // more bytes may finish it (after a pause, they are keys)
            Scan::No => {
                keys.push(input[i]);
                i += 1;
            }
        }
    }
    i
}

fn pixels_on(p: &Probe) -> Option<CellPx> {
    let trusted = matches!(p.terminal().as_deref(), Some("ghostty" | "kitty"));
    (p.fenced && p.graphics_ok() && trusted).then_some(p.cell?)
}
```

Wait for DA1 (`fenced`) or a short timeout (the CLI waits 200 ms at most); any answer missing
by then means cells. Which terminals to trust is yours: the CLI trusts Ghostty and kitty and
stays in cells inside tmux or screen, where the graphics never reach the terminal.

**The cell size is an input.** The crate never asks the terminal: you pass a `CellPx` to every
`frame`. Get it from the probe, and when the window's pixel size (`TIOCGWINSZ`) stops matching
cells × cell size, the font changed: write `probe::cell_size_request()` and take the new
`CellSize` reply. Treat it like any other input that reaches your renderer, so the same inputs
give the same bytes. caretline carries it in the state: send it as `Msg::Resize`'s `cell_px`
and read it back from `View::cell_px` (or `Frame::cell_px`), as the CLI does, and a replay
draws the same pixels.

`caretline demo layers` is a working host: its renderer is
[`src/layers.rs`](../crates/caretline-cli/src/layers.rs) (tiny-skia rasters for a panel, arrow,
ring and veil) and its loop [`src/demo/layers.rs`](../crates/caretline-cli/src/demo/layers.rs).
The status bar shows each pixel frame's bytes, rasters and time; in Ghostty 1.3.1 the first
frame is 17.2 KB with `t=d` (1.1 KB with `t=t`), a scroll step about 300 bytes and an unchanged
frame nothing.

## Not built yet

These are designed but not built ([design, sections 9 and 12.5](../docs/design/layers.md)):
in-frame mode (an `install(host)` adapter that keeps the layers in a view's `ext` values and
draws them in a frame pass, on the engine hooks caretline now has), `hint.*` and `layer.*` in
caretline's own protocol, and MCP tools for agents.
Until then a host uses the screen-level mode on this page. The editor (`caretline FILE`) and
`caretline-mcp` show no layers; `caretline demo layers` is the one place the CLI does.
Walkthroughs are built, as a reducer whose state the host keeps: see [tour.md](tour.md).
