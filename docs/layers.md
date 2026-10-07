# Layers: hints, callouts and spotlights

> **New and not released.** [`caretline-layers`](../crates/caretline-layers) is in the
> repository but not yet on crates.io, and its API may change before its first release. This
> page covers what is built (step 1a of the [design](../docs/design/layers.md)); the next
> steps (engine hooks, in-frame mode, pixels, walkthroughs, protocol and MCP tools in caretline
> itself) are listed in the design's section 9.

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
needs only serde, for a host that draws no caretline editor at all.

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
| `capture` | A modal step, for walkthroughs (a host may refuse it to agents: `no_agent_capture`) |
| `hide_off_screen` | With the anchor off screen, show only the edge chip |
| `place` | The sides to try for the box (default below, above, right, left) |
| `max_width` | The widest the box may be (default 52, never over two-thirds of the area) |

`Layers` holds them (and `hidden`, for "hide all"). In Rust, `Layer::new(anchor)` with
`with_content`, `with_arrow`, `with_ring` and `with_spotlight` builds one.

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
- **`Chain`**: both, asked in order: `Chain(vec![&anchors, &editor])`.

**Following edits.** Block anchors need nothing: marks follow their blocks. Text anchors move
with the text. The engine hands out each message's text changes: `update_with_changes` (and
`update_doc_with_changes`, `Session::apply_with_changes`) returns them beside the effects, as one
`ChangeSet` from the text before the message to the text after, or `None` when the text didn't
change. Pass them to `observe` after every message:

```rust
use caretline::{update_with_changes, Msg, State, Viewport};
use caretline_layers::{apply, observe, Anchor, Content, Layer, LayerOp, Layers, Limits};

let mut editor = State::new("hello world", None, Viewport { width: 40, height: 6 });
let mut layers = Layers::default();
let hint = Layer::new(Anchor::Text { from: 6, to: 11 }).with_content(Content::hint(None, "this word"));
let now_ms = 0;
apply(&mut layers, LayerOp::Push(hint), None, now_ms, &Limits::default()).unwrap();

let (_effects, changes) = update_with_changes(&mut editor, Msg::InsertText { text: "big ".into() });
observe(&mut layers, changes.as_ref(), now_ms); // maps text anchors, drops expired layers
assert_eq!(layers.layers[0].anchor[0], Anchor::Text { from: 10, to: 15 });
```

The start of a range sticks after an insertion there, the end before one. A range whose text
was deleted is dropped and the next fallback takes over; a layer with no anchor left goes.
`map_anchors(&mut layers, &changes)` does the mapping alone.

## Content and `Renderer::measure`

Content is `{kind, data}`, as opaque to the crate as mark payloads are to the engine. The host
registers a **`Renderer`** per kind. Placement needs only its size:

```rust
use caretline_layers::{Renderer, Renderers, Size, HINT};
use serde_json::Value;

struct HintBox;

impl Renderer for HintBox {
    // The box for `data`, borders included, at most `avail`. A zero size means no box.
    fn measure(&self, data: &Value, avail: Size) -> Size {
        let text = data["text"].as_str().unwrap_or("");
        let w = (text.chars().count() as u16 + 4).min(avail.w);
        Size::new(w, 3)
    }
}

let renderers = Renderers::new().register(HINT, HintBox);
```

`measure` must be pure: the same data and room give the same size, or a replay places boxes
differently. A closure `Fn(&Value, Size) -> Size` is a renderer too. A layer whose kind has no
renderer is placed without a box and listed in `Plan.unrendered`.

**`chip(data, anchor, off)`** sizes the edge chip shown where an off-screen anchor lies (one row
high; 8 by 1 by default). It gets the layer's data, the anchor that lies off screen and which
way (`Off`), so a host can size a label such as `↓ 2/11 here` to fit. It must be pure too:

```rust
use caretline_layers::{Anchor, Off, Renderer, Size};
use serde_json::Value;

struct HintBox;

impl Renderer for HintBox {
    fn measure(&self, _data: &Value, avail: Size) -> Size {
        Size::new(avail.w.min(30), 3)
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
- `Grid::from_frame(&frame)` and `mark_frame(&frame, x, y)` for a caretline frame.

The **`Plan`** (serializable) holds, per layer in draw order (`Planned`):

| Field | What the host draws |
|---|---|
| `rect`, `mode`, `side` | The box (`mode: box`, beside the anchor on `side`) or a one-row strip (`mode: strip`: the area is under 48 columns or 12 rows, or no box fits) |
| `anchor` | Where the anchor resolved: rects, or `off` (above, below, left, right) |
| `chip` | The edge chip on the edge an off-screen anchor lies beyond |
| `dock` | A box for an off-screen anchor docks flush against its chip: where on the box's border (`Attach { edge, offset }`) the chip touches it, so the host joins the two there |
| `route` | The arrow: `junction` on the box's border and the same place as `attach` (`Attach { edge, offset }`: which edge, and how many cells along it from the corner), then `steps`, one cell each with the direction it enters and leaves; the last is the head. Lay out the border round `attach`: a title never sits where the arrow leaves |
| `no_arrow` | Why a layer that asked for an arrow has none: `docked` (the anchor is off screen; the chip points the way), `screen` (a screen position), `no_box` (a strip, or no box), `no_way` (no route round the other layers, holes, protected cells and wide graphemes) |
| `ring` | The anchor's cells to mark |
| `owner`, `agent` | Whose layer it is (`Owner::actor()` gives an agent's name), and whether it is an agent's: what the host attributes it by |

and for the whole screen: `spots` (each spotlight's area and holes; `Plan::dimmed(x, y)` says
whether a cell is dimmed), `regions` with `Plan::hit(x, y)` (a box is `<layer>`, an edge chip
`<layer>/reveal`; a host adds its own buttons as `<layer>/<name>`), and `missing` (layers none
of whose anchors resolved).

Boxes are placed in z order, each scored over the sides it may take: text covered, distance to
the anchor and, with an arrow, the arrow's route; never over its anchor or any other layer's
(every layer's anchor is known before any box is placed), another layer's box, chip, strip or
arrow, a hole, protected cells or (for agents) the caret. No arrow runs under a box, and chips
on one edge slide along it to a free place. A box whose arrow routes beats one whose arrow
can't; when the side of the anchor facing the box can't be reached, the arrow may end beside
the anchor on another side, pointing at it, and on a left or right edge it never leaves beside
the title row. Ties go to the first side listed, which gives flip; a box shifts along its side
to stay inside the area, reaches the edge rather than leave a sliver of words beside it, and
falls back to a strip when nothing fits.

Placement is quick enough for every frame: at 100×40 (release build) a box costs about 4 µs, a
box with its arrow about 27 µs, and a spotlight with an arrow about 44 µs.

## Ops for your protocol

`ops` turns requests from any JSON protocol into ops, and results back into JSON. The host
keeps its own transport and routing (`op` and `view` are its fields):

| Op | Request fields | Becomes |
|---|---|---|
| `hint.show` | `anchor` (one or a list), `text`, `title?`, `ttl_ms?`, `place?`, `arrow?` (default true), `ring?` (default true), `actor?` | A push of a `hint` layer |
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

## Purity: the rules for a host

The crate has no clock, randomness, I/O, terminal or async, and the host keeps it that way:

- **One state.** Keep `Layers` inside your app's state, not in a second store. Change it only
  with `apply`, `expire` and `observe`.
- **Time from your messages.** Pass the `now_ms` your messages carry, never a wall-clock read
  inside a reducer, so expiry and rate limits replay.
- **Feed every message.** Call `observe` with the changes `update_with_changes` returned for
  each message (`None` for one that edited nothing still expires layers), so text anchors stay
  on their text.
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
   columns and two rows):

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

## Not built yet

These are designed but not built ([design, section 9](../docs/design/layers.md)): the engine
hooks for drawing layers inside a caretline frame (in-frame mode), the kitty graphics plumbing
for pixel renderers, `caretline-tour` walkthroughs, `hint.*` and `layer.*` in caretline's own
protocol, and MCP tools for agents. Until then a host uses the screen-level mode on this page,
and nothing in `caretline`, `caretline-cli` or `caretline-mcp` shows layers.
