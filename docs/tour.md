# Walkthroughs

> **New and not released.** [`caretline-tour`](../crates/caretline-tour) is in the repository
> but not yet on crates.io, and its API may change before its first release. It is built on
> [`caretline-layers`](layers.md), also unreleased. caretline now has the engine hooks that
> would keep a walkthrough inside its state and traces (view values and `Msg::Ext`), but this
> crate doesn't use them yet ([Not built yet](#not-built-yet)): the host keeps the
> walkthrough's state itself.

A **walkthrough** is a list of steps that shows someone round a host's screen. Each step does
three things:

- **sets the scene**: `host`, a patch to the host's own state (which panel is open, what has
  focus), opaque to the crate;
- **points at things in it**: `layers`, hint boxes, arrows, rings and spotlights placed by
  [`caretline-layers`](layers.md);
- **says what it is about**: `narration`, the step's own words, which the host shows where it
  likes.

`caretline-tour` reads walkthroughs from TOML or JSON, keeps the progress as data, and tells
the host what to do. It draws nothing and stores nothing.

| caretline-tour does | The host does |
|---|---|
| Reads and checks the format, strictly | Keeps the files, or builds tours in code |
| Keeps the progress (`TourState`), changed only by ops and `observe` | Keeps that state in its own state, beside everything else |
| Returns effects in order: the step's `host` patch, its layers, the step change, the end | Applies them in its own update: patches its state, replaces the guide layers, shows the narration, saves what was seen |
| Asks predicates (`advance`, `skip_if`, branches) | Answers them through `TourHost`: did this action run, did this event happen, what is the state at this key |
| Plans every step at several sizes (`plan_steps`) | Draws each step's scene at a size and says where things went |
| Parses `tour.*` requests and gives a JSON Schema | Routes them from its own protocol |

Pure: no clock (time is a `now_ms` the host passes), no randomness, I/O, terminal or async.
The same ops at the same times give the same state and effects.

## Add it

Until it is published, depend on it from a checkout or by git revision:

```toml
[dependencies]
caretline-tour = { git = "https://github.com/brancusi/caretline", rev = "<full sha>" }
caretline-layers = { git = "https://github.com/brancusi/caretline", rev = "<full sha>" }
```

The default feature `caretline` adds what needs the engine: `Editor`, which answers the
editor's own predicates from caretline types, and `find_in`, which finds a text in a
caretline document. Without it (`default-features = false`) the crate needs serde,
serde_json, toml and `caretline-layers` without its engine feature, for a host that draws no
caretline editor.

## The format

[`examples/tour.toml`](../crates/caretline-tour/examples/tour.toml) is a four-step walkthrough
over a host with a table of rows on the left and an editor on the right:

```toml
id = "example.basics"
version = 1
title = "A table and an editor"
kind = "hint"                                   # the content kind of a layer that names none

# A page of narration with nothing pointed at: it moves on by `tour.step {to = "next"}`.
[[step]]
id = "welcome"
host = { panels = { left = "table", right = "editor" }, focus = "table" }
narration = { title = "Welcome", text = "On the left, a table of rows; on the right, the row you open, as text." }

# The single-layer shorthand: `anchor`, `data` and `place` on the step are its layers[0]
# (id "rows/0").
[[step]]
id = "rows"
host = { focus = "table", select = "first" }
narration = { title = "Rows", text = "Each row is one record. Move with the arrow keys; Enter opens it." }
anchor = { host = { kind = "row", key = "first" } }
data = { title = "A row", text = "Press Enter to open it." }
place = { arrow = true, ring = true, sides = ["right", "below"] }
advance = { any = [{ command = "row.open" }, { event = "row.opened" }] }

# Two layers, one in each view of the document.
[[step]]
id = "edit"
host = { panels = { right = "editor" }, focus = "editor" }
narration = { title = "Two views, one text", text = "The outline follows the text you edit." }
advance = { msg = "insert_text", count = 5 }    # before [[step.layers]], or TOML puts it in the last one
nudge = { after_ms = 8000, data = { text = "Type a few letters: the outline keeps up." } }

[[step.layers]]                                 # id "edit/0"
anchor = { find = "## Summary", in = "main" }
data = { text = "Type here…" }
place = { arrow = true }

[[step.layers]]
id = "outline-entry"
anchor = [{ find = "Summary", in = "outline" }, { screen = "top" }]
data = { text = "…and the outline shows the same heading." }

[[step]]
id = "finish"
host = { focus = "table" }
anchor = { screen = "center" }
data = { title = "That's all", text = "Open the walkthrough again from the help menu." }
capture = true
# Branching: back to the rows if the host says so (its state at "again"), else the end.
next = [{ if = { state = { key = "again", present = true } }, goto = "rows" }, { goto = "end" }]
```

`parse_toml` and `parse_json` read it; both are **strict**: an unknown field anywhere is an
error, except inside `data` and `host`, which are opaque. The same walkthrough in JSON has
`"step": [...]` (`"steps"` is read too).

**The walkthrough:**

| Field | Meaning |
|---|---|
| `id` | Its id, for `tour.start` by id and for seen-state |
| `version` | Default 1. Bump it when it changes enough to offer it again |
| `title` | For a host's menu (`tour.list` gives it) |
| `kind` | The content kind of a layer that names none. Default `hint`, the kind every host renders ([layers.md](layers.md#the-hint-kind)) |
| `reoffer` | Offer this version to someone who stopped or finished an earlier one ([Seen-state](#seen-state-is-the-hosts)) |
| `step` | The steps, in order (TOML's `[[step]]`) |
| `meta` | The host's own metadata about the walkthrough (author, dates, audience…). Opaque, like a step's `host`: carried, never read, left out when absent. Every other unknown field is refused |

**A step:**

| Field | Meaning |
|---|---|
| `id` | Unique in the walkthrough. `next`, `back` and `end` are reserved |
| `host` | Optional. The host's patch for this step, any JSON. The crate hands it over on entering the step and never reads it |
| `narration` | Optional. `{title?, text}`: the step's own words. Not a layer: nothing is placed, no box. The host shows it in a panel, a strip, reads it aloud, or not at all |
| `layers` | The step's layers: `[{id?, anchor, kind?, data, place?, capture?}]` |
| `anchor`, `kind`, `data`, `place`, `capture` | The single-layer shorthand: a step with these is a step with one layer. Both `layers` and any of these is an error |
| `advance` | Optional [predicate](#predicates): move on when it holds. A step without one moves only by ops; that is how pages are turned |
| `skip_if` | Optional predicate: passed over, going forward, when it holds on arrival |
| `nudge` | Optional `{after_ms, data}`: once the step has lasted `after_ms`, `data` is merged into each layer's data, once per visit |
| `next` | Optional branches `[{if?, goto}]`: going forward takes the first whose `if` holds (or that has none); `goto` is a step id or `end`. With none taken, the next step in order; after the last, the end |

**A layer** is a `caretline-layers` layer the walkthrough owns (owner `guide`, z 10):

| Field | Meaning |
|---|---|
| `id` | Defaults to `<step id>/<index>`: `rows/0`, `edit/0` |
| `anchor` | One anchor, or a list of fallbacks in order. Any [`caretline-layers` anchor](layers.md#anchors) (`{block}`, `{text}`, `{caret}`, `{host = {kind, key}}`, `{screen}`, with `in` for one view), or `{find = "text", in? = "view"}` |
| `kind` | Defaults to the walkthrough's `kind` |
| `data` | What the layer's box shows, for the kind's renderer. A `hint` takes `{title?, text}`. With neither `kind` nor `data`, the layer has no content: a ring or a spotlight alone, no box |
| `place` | `{sides, max_width, arrow, ring, spotlight, hide_off_screen}` (`ring` and `spotlight` take `true` or their object; `max_w` and `connector` are read as `max_width` and `arrow`), or just a list of sides |
| `capture` | A modal step: the host routes keys to the walkthrough |
| `avoid` | What the box and arrow should keep off, softly ([avoid areas](layers.md#avoid-areas)): one anchor or a list, `find` included (resolved with the anchors). It becomes the layer's `avoid`, so it follows scroll and edits. Also a single-layer field on the step |

For kinds other than `hint`, the layer's data gets `at` and `of` (the step's place, from 1), so
a renderer can draw step dots. A `hint`'s data stays `{title?, text}`.

### `find` is resolved by the host, before the start

`{find = "## Summary", in = "main"}` names a text, not a position. The host looks it up once,
in its own documents, before it starts the walkthrough, with `Tour::resolve_finds`; with the
`caretline` feature, `find_in` finds the text in a caretline document (as text within the
block it starts in, so it follows that block through edits). The started walkthrough then
holds stable anchors, and so does every trace of it. A `find` with no answer stays in the
tour and is left out of its layer's fallbacks: `outline-entry` above falls back to
`{screen = "top"}`, and a layer with no anchor left isn't shown.

### Check it

`check(&tour)` lists what parses but can't work, each a `Problem { level, code, step?, layer?,
detail }`. Errors (`apply` refuses to start a walkthrough with any): `no_steps`, `no_id`,
`duplicate_step`, `duplicate_layer`, `reserved_id`, `unknown_goto`, `no_anchor`,
`unknown_anchor` (a `caret_in`, `changed` or `folded` naming no layer of the step) and
`hint_data`. Warnings: `empty_step` (no layers, narration or host patch), `unscoped_find` (a
`find` with no `in`: which view's text?), `nudge_data`, `zero_count` and
`unreachable_branch`.

```rust
use caretline::State;
use caretline_tour::{Level, TourEffect, TourOp, TourState, apply, check, find_in, parse_toml};

fn start(
    ed: &State,
    tours: &mut TourState,
    src: &str,
    now_ms: u64,
) -> Result<Vec<TourEffect>, Box<dyn std::error::Error>> {
    let mut tour = parse_toml(src)?;
    for p in check(&tour) {
        // Errors also make `apply` refuse to start it (`invalid`).
        eprintln!("{:?} {}: {}", p.level, p.code, p.detail);
        if p.level == Level::Error {
            return Err(p.detail.into());
        }
    }
    // Once, in the host's own documents: `view` is the `in` of the anchor, if any.
    let missing = tour.resolve_finds(|text, _view| find_in(&ed.doc, text));
    for (step, layer, text) in missing {
        eprintln!("{step} {layer}: {text:?} isn't in the document");
    }
    Ok(apply(tours, TourOp::Start(Box::new(tour)), now_ms)?)
}
```

A host with several documents picks the one to search by `view`.

## The reducer

`TourState` is the progress: the whole walkthrough as started (so a trace of it needs no
file), the current step, when it began, the counts its predicates keep, the history `back`
uses, and the seen-state. It serializes, so the host keeps it in its own single state.

Two functions change it, and both return effects for the host to apply, in order:

- `apply(&mut state, op, now_ms) -> Result<Vec<TourEffect>, TourError>`, with a `TourOp`:
  `Start(tour)`, `Stop`, `Next`, `Back`, `To(step id)` or `Restart`. `apply_with` takes a
  `TourHost` too, to answer the branches and `skip_if` met going forward; `apply` answers them
  from time and counts alone. A refusal has a `reason` (`invalid`, `not_found`,
  `not_running`, `no_back`) and a `detail`.
- `observe(&mut state, &host, now_ms) -> Vec<TourEffect>`, once per message the host applied
  (or per tick): counts what happened, gives the nudge when it's time, and moves on when
  `advance` holds.

| Effect | The host |
|---|---|
| `Host { patch }` | Applies the step's `host` value to its own state |
| `Layers { layers }` | Replaces the walkthrough's layers (owner `guide`) with these; none clears them. `replace_guide` does it |
| `Step { tour, step, at, of, narration }` | The current step changed: show its narration and "at of of" |
| `Ended { tour, seen }` | The walkthrough finished or was stopped: save `seen` for `tour` |

Entering the first step of the example gives (as JSON):

```json
[{"effect": "host", "patch": {"focus": "table", "panels": {"left": "table", "right": "editor"}}},
 {"effect": "layers", "layers": []},
 {"effect": "step", "tour": "example.basics", "step": "welcome", "at": 1, "of": 4,
  "narration": {"title": "Welcome", "text": "On the left, a table of rows; on the right, the row you open, as text."}}]
```

In the host's update, the effects are applied like any other:

```rust
use caretline_layers::{Layers, Refusal};
use caretline_tour::{Narration, Seen, replace_guide};
use serde_json::Value;
use std::collections::BTreeMap;

struct App {
    scene: Value, // the host's own state, which a step's `host` patches
    tour: TourState,
    layers: Layers,
    narration: Option<(usize, usize, Narration)>,
    seen: BTreeMap<String, Seen>, // saved by the host, loaded into `tour.seen` at start-up
}

impl App {
    fn run(&mut self, fx: Vec<TourEffect>, now_ms: u64) -> Result<(), Refusal> {
        for e in fx {
            match e {
                TourEffect::Host { patch } => merge(&mut self.scene, patch),
                TourEffect::Layers { layers } => replace_guide(&mut self.layers, layers, now_ms)?,
                TourEffect::Step { at, of, narration, .. } => {
                    self.narration = narration.map(|n| (at, of, n))
                }
                TourEffect::Ended { tour, seen } => {
                    self.narration = None;
                    self.seen.insert(tour, seen);
                }
            }
        }
        Ok(())
    }
}

/// A JSON merge patch: objects merge key by key, `null` removes, anything else replaces.
fn merge(into: &mut Value, patch: Value) {
    match (into, patch) {
        (Value::Object(m), Value::Object(p)) => {
            for (k, v) in p {
                if v.is_null() {
                    m.remove(&k);
                } else {
                    merge(m.entry(k).or_insert(Value::Null), v);
                }
            }
        }
        (into, patch) => *into = patch,
    }
}
```

How a patch applies is the host's: a merge patch is one choice. The guide's layers then go
through `caretline-layers` like any other: planned each frame, drawn by the host's renderers,
moved through edits by its `observe` ([layers.md](layers.md#walkthrough-a-ratatui-host)).

### Moving, and jumps that are exact

- **Going forward** (start, restart, `next`, `advance`) follows the step's `next` branches,
  and passes over steps whose `skip_if` holds on arrival. After the last step, or at a
  `goto = "end"`, the walkthrough finishes.
- **Jumps are exact.** `To(id)` and `Back` enter their step and nothing else: no `skip_if`,
  no branches. Entering a step yields its `host` patch, its layers and the step change, the
  same effects however it was reached, and nothing of the step it left stays. So a jump to a
  step looks the same as arriving there in order, provided each step's `host` sets the scene
  it relies on: only the target's patch is applied, not those of the steps in between.
- **`Back`** returns to the step left last; steps passed over by `skip_if` aren't in the
  history. At the first step it is refused (`no_back`).
- **`To`** also works after a walkthrough ended: it enters that step of the last one.
  `Restart` starts the last one again from its first step.
- **`Stop`** ends it: the guide's layers are cleared and `Ended` says it was stopped.

A step's counts and clock start afresh on every entry, and its nudge is given once per visit.

## Predicates

`advance`, `skip_if` and a branch's `if` are predicates, one key each (and `count`):

| Predicate | Holds when |
|---|---|
| `{msg = "insert_text", count? = 5}` | A message of that kind was applied, `count` times (default 1) since the step began |
| `{command = "row.open", count?}` | That action or command ran, `count` times |
| `{event = "row.opened", count?}` (also `effect`, `host_effect`) | That host event happened, `count` times |
| `{state = {key, present?, match?}}` (also `ext`) | The host's state at `key` is present (the default), absent (`present = false`), or contains `match` (a subset: every key of a pattern object, recursively) |
| `{caret_in = "anchor"}` | The caret is inside the step's first layer's anchor, or the anchor of the layer with that id |
| `{selection = "nonempty"}` | Some selection is non-empty |
| `{changed = "anchor"}` | The message changed text at that anchor |
| `{folded = "anchor"}` | The anchor's block is folded |
| `{after_ms = N}` | The step has lasted N ms (by the `now_ms` the host passes, so ticks keep replay exact) |
| `{any = […]}`, `{all = […]}`, `{not = {…}}` | Composition |

Counts are kept since the step began, so `all = [{command = "a"}, {command = "b"}]` holds
once both have run, in separate messages or not.

### `TourHost`: the host's answers

A predicate asks; the host answers through `TourHost`, about the message it just applied and
its state now. Every method answers no by default, so a host implements only what its
walkthroughs use. `()` is the host that answers nothing (ops and time alone).

```rust
use caretline_tour::{TourHost, observe};

/// What happened in one message of the host's own update.
struct Happened<'a> {
    scene: &'a Value,
    ran: Option<&'a str>,
    events: &'a [String],
}

impl TourHost for Happened<'_> {
    fn ran(&self, command: &str) -> bool {
        self.ran == Some(command)
    }
    fn event(&self, name: &str) -> bool {
        self.events.iter().any(|e| e == name)
    }
    fn state(&self, key: &str) -> Option<Value> {
        self.scene.get(key).cloned()
    }
}

impl App {
    /// After the host's own update has applied a message.
    fn after(&mut self, ran: Option<&str>, events: &[String], now_ms: u64) -> Result<(), Refusal> {
        let host = Happened { scene: &self.scene, ran, events };
        let fx = observe(&mut self.tour, &host, now_ms);
        self.run(fx, now_ms)
    }
}
```

The other methods are `msg(kind)`, `caret_in(anchors)`, `selection()`, `changed(anchors)` and
`folded(anchors)`. The anchor ones get the named layer's resolved anchors.

### The editor's answers

With the `caretline` feature, `Editor` answers the editor's own questions for one message a
caretline editor applied: `msg` (the message's kind), `command` (the message equals that
catalog command's), `event` (an effect of that name, or a host effect), `caret_in`,
`selection`, `changed` and `folded`. What it can't answer (a host command, a host event,
`state`) goes to the host given with `with_host`:

```rust
use caretline::{Msg, update_with_changes};
use caretline_tour::Editor;

fn on_editor_msg(ed: &mut State, app: &mut App, msg: Msg, now_ms: u64) -> Result<(), Refusal> {
    let (effects, changes) = update_with_changes(ed, msg.clone());
    let host = Happened { scene: &app.scene, ran: None, events: &[] };
    let on = Editor::new(&ed.doc, &ed.view)
        .message(&msg, &effects, changes.as_ref())
        .with_host(&host);
    let fx = observe(&mut app.tour, &on, now_ms);
    app.run(fx, now_ms)
}
```

`Editor::new(doc, view)` alone, with no message, is a tick. Its approximations: a block runs
from its mark to the next, and `command` matches the engine's catalog, not a host's.

## Checking at several sizes

A step that reads well at 120 columns may point at nothing at 44. `plan_steps(&tour, sizes,
&renderers, scene)` plans every step at every size: for each, the host's `scene` callback sets
the step's scene up at that size and calls back with the grid it drew and its anchor
resolver, and the step's layers are planned with them and the host's renderers. Each
`StepPlan { step, width, height, plan, problems }` reports errors (`not_found`: an anchor
resolved nowhere; `unrendered`: a kind with no renderer; `unresolved_find`; `refused`: a layer
the model refused) and warnings (`no_way`: no arrow fits; `off_screen`: the anchor is off
screen). Run it in a test or a lint command:

```rust
use caretline_layers::{AnchorKey, AnchorMap, Grid, HINT, Rect, Renderers, Size};
use caretline_tour::{Tour, plan_steps};

/// The host's drawing, at a size: its grid of text and where its things went.
fn draw(scene: &Value, w: u16, h: u16) -> (Grid, AnchorMap) {
    let mut grid = Grid::new(w, h);
    let mut anchors = AnchorMap::new();
    if scene["panels"]["left"] == "table" {
        grid.mark_text(2, 2, "the first row");
        anchors.put(AnchorKey::host("row", "first"), Rect::new(2, 2, w / 2 - 4, 1));
    }
    (grid, anchors)
}

fn lint(tour: &Tour) -> bool {
    let renderers = Renderers::new().register(HINT, |_: &Value, avail: Size| {
        Size::new(avail.w.min(40), avail.h.min(4))
    });
    let mut scene = serde_json::json!({});
    let plans = plan_steps(tour, &[(120, 40), (80, 24), (44, 16)], &renderers, &mut |step, w, h, go| {
        // The scene as a person arriving in order sees it: every patch so far.
        if let Some(patch) = &step.host {
            merge(&mut scene, patch.clone());
        }
        let (grid, anchors) = draw(&scene, w, h);
        go(&grid, &anchors)
    });
    let mut ok = true;
    for p in &plans {
        for problem in &p.problems {
            eprintln!("{}×{} {}: {:?} {} ({})", p.width, p.height, p.step, problem.level, problem.code, problem.detail);
            ok &= problem.level != Level::Error;
        }
    }
    ok
}
```

A scene that doesn't call back plans nothing for that step and size. Resolve the `find`s
first, as for a start, or each one is an `unresolved_find`.

## Ops in your own protocol

`caretline_tour::ops` parses and answers walkthrough requests in a host's own JSON protocol,
as `caretline_layers::ops` does for layers:

| Op | Fields | Becomes |
|---|---|---|
| `tour.start` | `tour`: a whole walkthrough (an object), or the id of one in the host's library | `TourOp::Start` |
| `tour.step` | `to`: a step id, `"next"` or `"back"` | `TourOp::To`, `Next` or `Back` |
| `tour.restart` | | `TourOp::Restart` |
| `tour.stop` | | `TourOp::Stop` |
| `tour.list` | | nothing applied: `ops::list` answers |

Every request may carry the host's request `id` and its routing `view`; anything else is
refused. `ops::reply(&state)` says where the walkthrough is, `ops::list(library, &state)` lists
the library with what was seen, and `ops::error(&e)` turns a refusal into an error result:

```json
{"op": "tour.step", "to": "edit"}
{"tour": "example.basics", "step": "edit", "at": 3, "of": 4,
 "narration": {"title": "Two views, one text", "text": "The outline follows the text you edit."}}

{"op": "tour.step", "to": "nope"}
{"error": {"reason": "not_found", "detail": "no step \"nope\""}}

{"op": "tour.stop"}
{"tour": "example.basics", "step": null}
```

```rust
use caretline_tour::apply_with;
use caretline_tour::ops::{self, Request};

impl App {
    fn tour_request(&mut self, ed: &State, library: &[Tour], op: &str, req: &Value, now_ms: u64) -> Value {
        let mut o = match ops::parse(op, req, library) {
            Ok(Request::List) => return ops::list(library, &self.tour),
            Ok(Request::Apply(o)) => o,
            Err(e) => return ops::error(&e),
        };
        if let TourOp::Start(t) = &mut o {
            t.resolve_finds(|text, _view| find_in(&ed.doc, text));
        }
        let host = Happened { scene: &self.scene, ran: None, events: &[] };
        match apply_with(&mut self.tour, o, now_ms, &host) {
            Ok(fx) => match self.run(fx, now_ms) {
                Ok(()) => ops::reply(&self.tour),
                Err(r) => serde_json::json!({"error": {"reason": "refused", "detail": r.to_string()}}),
            },
            Err(e) => ops::error(&e),
        }
    }
}
```

`ops::schema()` is a JSON Schema (draft 2020-12) for every request `parse` accepts, the
replies, and the walkthrough format itself (`$defs/tour`, `step`, `layer`, `pred`, …), for a
host that publishes its protocol or validates walkthrough files in an editor. What a schema
can't say (`layers` and the shorthand together, one key per predicate) the parser and `check`
catch.

## Seen-state is the host's

The crate never touches storage. When a walkthrough ends, `Ended { tour, seen }` carries
`Seen { version, end }` (`end` is `finished` or `stopped`) for the host to save, and the state
keeps it in `TourState.seen`. At start-up the host loads what it saved into `seen`.

A walkthrough never starts by itself. `state.offer(&tour)` says whether to offer one in a
prompt: never seen, or seen at an earlier version of a walkthrough marked `reoffer = true`.
An explicit start (`tour.start`, a menu item) is never refused for having been seen.

## Not built yet

These are designed but not built ([design, sections 6, 12.4 and 12.5](../docs/design/layers.md)):
the walkthrough's state in a caretline view (`ext["tour"]`) with an ext reducer for its ops
(the engine's `View::ext` and `Msg::Ext`, which exist now), so an editor trace records and
replays it with the engine's own; capture routing; the host's
catalog for `command`; and `caretline demo guide`. Until then the host keeps `TourState`
beside its own state, as on this page.
