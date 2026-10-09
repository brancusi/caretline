# Rust API

The `caretline` crate, by job. Every snippet here compiles against the crate on `main`.
The complete program at the end is also in the repo as
[`examples/basic.rs`](../crates/caretline/examples/basic.rs):

```sh
cargo run -p caretline --example basic
```

## Add the dependency

caretline is on [crates.io](https://crates.io/crates/caretline):

```sh
cargo add caretline
```

```toml
[dependencies]
caretline = "0.5"
```

Its library is `caretline::`; this repository's crate is the same code. A few things on `main` are newer than the latest release;
for those, use a git dependency (`caretline = { git = "https://github.com/brancusi/caretline" }`), ideally pinned with `rev = "…"`.

Its dependencies are ropey, smallvec, smartstring, the unicode crates, serde, serde_json and
log. There's no terminal crate and no ratatui.

## The public surface

| Item | Where | Use it to |
|---|---|---|
| `State`, `Config`, `Viewport`, `Scroll` | `caretline` | Hold and configure the editor: one document and one view |
| `Document`, `View`, `ViewConfig`, `Follow`, `ScrollPastEnd`, `ExternalUndo` | `caretline` | A document and its views, separately: see [Several views](#several-views-of-one-document) |
| `CellPx` | `caretline` | One cell's size in device pixels, carried by `Msg::Resize` into `View::cell_px` and `Frame::cell_px`: see [Render](#render) |
| `update_doc` | `caretline` | Apply a message through one of several views |
| `update_with_changes`, `update_doc_with_changes`, `ChangeSet`, `Assoc` | `caretline` | Apply a message and get its text changes, to map positions of your own: see [Map your own positions](#map-your-own-positions-through-each-message) |
| `ExtChange` | `caretline` | A change from elsewhere, for `Msg::External`: see [messages.md](messages.md#changes-from-elsewhere) |
| `Msg`, `Dir`, `By`, `Effect` | `caretline` | Say what happened; get work back. Both are `#[non_exhaustive]`: a `match` ends with a `_` arm |
| `update`, `replay` | `caretline` | Apply one message; fold many |
| `update::selection_text` | `caretline::update` | Get the selected text, as a copy would |
| `view`, `Frame`, `Cell`, `CellFlags`, `Region`, `Role` | `caretline` | Render to cells; draw over a frame with its writers (`set`, `restyle`, `flag`, `role`, `region_at`) |
| `view::{render, render_plain, render_skipping, hit, locate, Locate, RowInfo, Hit, display_width}` | `caretline::view` | Render any view, with or without the host's frame passes; read cells and rows; style them by meaning; hit-test a cell; find a char position's cell |
| `OutlineLayout`, `views::hidden_lines` | `caretline` | The [outline layout](structure.md#the-outline-layout) and folds |
| `keymap`, `Key`, `KeyCode`, `Mods` | `caretline` | Map keys to messages |
| `parse_keys`, `script_to_msgs`, `keymap::ScriptItem` | `caretline` | Use the `--keys` notation |
| `trace::{TraceLine, parse_msgs, replay_trace, replay_trace_views}` | `caretline::trace` | Record and replay sessions (with their views) |
| `layout::{Layout, LineFormat, RowPos, text_format, ensure_caret_visible}` | `caretline::layout` | Lower-level layout queries (`Layout::of(doc, view)`; `hangs(line)`: [a caret after hanging space](#a-caret-after-hanging-space)) |
| `helix::*` | `caretline::helix` | Helix's `Selection`, `Range`, `Transaction`, `History`, `Rope`, … |
| `Session`, `protocol::*` | `caretline` | A state with a rev and a trace, and the [protocol](protocol.md) in process: see [Session](#session) |
| `Marks`, `Mark`, `MarkId`, `MarkAttrs` | `caretline` | Block identity that survives edits, with the host's payload: see [Block marks](#block-marks) |
| `marks::{Clipboard, ClipMark, MarkDelta, Fixup, is_line_start}`, `update::mark_only_edit` | `caretline::marks`, `::update` | The register with carried marks, the per-revision deltas, a host's undoable mark edit |
| `OutlineConfig`, `Outline`, `BlockInfo`, `Kind`, `NewBlock`, `outline::{markdown, derive, content, Hang}` | `caretline`, `::outline` | Block documents: [structure](structure.md) and the [Markdown grammar](markdown.md) over the same buffer; `Document::take_touched` says [which blocks changed](#what-changed-since-you-last-looked) |
| `outline_keymap`, `keymap_for`, `script_to_msgs_for` | `caretline` | A block document's keys |
| `Host`, `Ctx`, `Edit`, `MarkOp`, `Decoration`, `Deco` | `caretline` | Your app's [extensions](embedding.md#extending-the-engine): commands, input rules (`Edit::then_default`: [adjust the engine's own action](embedding.md#adjusting-what-enter-does)), decorations |
| `ExtFns`, `ExtOut`, `Observed` | `caretline` | [View values and their reducers](embedding.md#view-values-and-ext-reducers): `Host::ext`, `Msg::Ext`, `View::ext` |
| `HostCommandInfo`, `OpFns` | `caretline` | [Catalog entries and protocol ops](embedding.md#catalog-entries-and-protocol-ops): `Host::catalog`, `Host::op` |
| `Host::frame_pass` | `caretline` | [Frame passes](embedding.md#frame-passes): draw over every rendered frame |
| `commands::{commands, command, command_msg, default_keymap, command_for, Binding, CommandInfo, Category, Platform}` | `caretline::commands` | The [command catalog and the default keymap](keys.md) |
| `trace::replay_trace_with` | `caretline::trace` | Replay a trace that runs host commands or ext reducers |

## Create a state

From text, with an optional path that `save` writes to, and a viewport in cells (the last row
is the status bar):

```rust
use caretline::{State, Viewport};

let state = State::new("# Draft\n", Some("draft.md".into()), Viewport { width: 80, height: 24 });
assert_eq!(state.caret(), 0);
assert!(!state.doc.dirty);
```

From a file. A file that doesn't exist yet is an empty, clean document. The line ending
(LF or CRLF) is detected from the text:

```rust
use caretline::{State, Viewport};

fn open(path: &str) -> std::io::Result<State> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    Ok(State::new(&text, Some(path.to_string()), Viewport { width: 80, height: 24 }))
}
```

Change the config directly. It's plain data:

```rust
use caretline::{State, Viewport};

let mut state = State::new("a\tb", None, Viewport { width: 80, height: 24 });
state.doc.config.tab_width = 2;
state.doc.config.soft_wrap = false;
state.view.config.status_bar = false; // every row shows text; no status bar
```

The config types (`Config`, `ViewConfig`, `OutlineConfig`, `OutlineLayout`, and `ConfigInput`
when you build a state by hand) are `#[non_exhaustive]`, so new settings never break your build.
To make one, start from `default()` and chain the `with_` setters:
`OutlineLayout::default().with_hang_glyphs(true)`.

`OutlineConfig` holds a block document's grammar and rules: `indent`, `tags`, `new_tag`,
`atomic_images`, `numbered` (see [markdown.md](markdown.md#outlineconfig)) and `nest_joins`,
off by default: on, Tab closes the blank row above a block it nests directly under the block
above, and Shift-Tab adds none back (`OutlineConfig::default().with_nest_joins(true)`; see
[structure.md](structure.md#blank-rows-and-nest_joins)).

Three settings in `view.config` shape scrolling: `scrolloff` (rows of margin kept around the
caret), `page_overlap` (rows of the previous screen a page motion keeps, default 0) and
`scroll_past_end`, which lets the view scroll past the document's last row, as an editor's
"scroll beyond last line" does, so writing at the end of a long page keeps the caret's
`scrolloff` margin below it instead of scrolling the page under a caret on the last row.
`ScrollPastEnd::Off` (the default) keeps the last row at the bottom; `Margin` allows as many
empty rows as `scrolloff`; `Rows(n)` up to `n`; `Half` up to half the view. In JSON it is
`"scroll_past_end": "margin"` (or `"half"`, or `{"rows": 3}`), left out while off.
`ScrollPastEnd::rows(h, scrolloff)` says how many empty rows that is for a view of `h` text rows.

```rust
use caretline::{ScrollPastEnd, ViewConfig};

let config = ViewConfig::default()
    .with_scrolloff(3)
    .with_page_overlap(2)
    .with_scroll_past_end(ScrollPastEnd::Margin);
```

`config.single_line` makes the document a one-line text field (a filter box, a prompt): the
text never holds a line break. Enter changes nothing, line breaks typed, pasted, edited in or
changed from elsewhere become spaces where they land (never joining two words or doubling a
space), lines never wrap and scroll sideways instead, and Up, Down and the page keys go to the start or the
end. It is ignored in a block document (`sanitize` turns it off there). After setting it
directly, call `state.sanitize()`: line breaks already in the text are flattened the same way,
and a history whose undo or redo could bring one back starts again. In JSON it is
`"config": {"single_line": true}`, left out while false. See
[embedding.md](embedding.md#a-one-line-field).

## Apply messages

`update` applies one message and returns the effects:

```rust
use caretline::{update, By, Dir, Msg, State, Viewport};

let mut state = State::new("hello world", None, Viewport { width: 40, height: 5 });
update(&mut state, Msg::Move { dir: Dir::Forward, by: By::Word, extend: false });
update(&mut state, Msg::InsertText { text: ",".into() });
assert_eq!(state.doc.text.to_string(), "hello, world");
```

`replay` folds a list and drops the effects:

```rust
use caretline::{replay, Msg, State, Viewport};

let mut state = State::new("", None, Viewport { width: 40, height: 5 });
replay(&mut state, [Msg::InsertText { text: "ab".into() }, Msg::DeleteBackward]);
assert_eq!(state.doc.text.to_string(), "a");
```

Keys go through the pure keymap. `script_to_msgs` takes the `--keys` notation
([messages.md](messages.md#key-scripts)) and counts `<wait:MS>` from the time you give it:

```rust
use caretline::{keymap, replay, script_to_msgs, Key, KeyCode, Mods, State, Viewport};

let mut state = State::new("hello", None, Viewport { width: 40, height: 5 });
// One key:
let ctrl_e = Key { code: KeyCode::Char('e'), mods: Mods { ctrl: true, ..Mods::default() } };
let msg = keymap(&ctrl_e).expect("Ctrl-E is bound");
// A script:
let mut msgs = vec![msg];
msgs.extend(script_to_msgs(" there<s-a-left>", state.doc.now_ms).unwrap());
replay(&mut state, msgs);
assert_eq!(state.doc.text.to_string(), "hello there");
```

Send `Msg::Tick { now_ms }` with real time before user input if you want typing grouped into
undo steps the way a person expects ([architecture.md](architecture.md#undo-grouping-worked-through)).

A tick never moves the view. The engine sorts every message by how it may move the view
(an internal `Msg::view_motion()`, an exhaustive match over the variants): keys and edits,
`Scroll`, `ScrollView`, `Resize` and folds follow the caret; a host's `Edit`, `Command` and
`InsertBlocks` follow only when they moved a caret or changed the text; `Click`, `Drag`,
`SelectWordAt` and `SelectBlock` keep the view while the caret is in it; `Tick`, `Frame`,
`FrameClock`, `Ext`, `ShowStatus`, `Saved`, `SaveFailed`, `External`, `Copy`, `Save` and
`Quit` leave it where it is. The table is in
[messages.md](messages.md#which-messages-move-the-view).

### Map your own positions through each message

A host that keeps char positions of its own (an anchor for a hint, a bookmark, a remote
cursor, a search hit) needs to know how each message moved the text. `update_with_changes`
does what `update` does and also returns the message's text changes: one `ChangeSet` from the
text before the message to the text after it. It covers every way a message changes the text:
typing, deleting, pasting, undo and redo, host commands and input rules, and `Msg::External`;
several edits in one message are composed into one. It is `None` when the text didn't change
(a motion, a tick, a refused edit, a command that only changed marks). Nothing is kept in the
state, so map your positions right after each message.

`changes.map_pos(pos, assoc)` maps a char position. `Assoc` says which side of an insertion
exactly at `pos` it keeps: `Assoc::Before` stays before the inserted text, `Assoc::After` moves
past it. Use `After` for the start of a range and `Before` for its end, and a range keeps
covering what it covered.

```rust
use caretline::{update_with_changes, Assoc, By, Dir, Msg, State, Viewport};

let mut state = State::new("hello world", None, Viewport { width: 40, height: 5 });
let (mut from, mut to) = (6, 11);                                   // "world"
state.view.selection = caretline::helix::Selection::point(6);
let (_effects, changes) = update_with_changes(&mut state, Msg::InsertText { text: "big ".into() });
if let Some(cs) = &changes {
    from = cs.map_pos(from, Assoc::After);
    to = cs.map_pos(to, Assoc::Before);
}
assert_eq!(state.doc.text.slice(from..to).to_string(), "world");

// A motion changes no text.
let (_, changes) = update_with_changes(&mut state, Msg::Move { dir: Dir::Forward, by: By::Word, extend: false });
assert!(changes.is_none());
```

`update_doc_with_changes(doc, views, acting, msg)` is the same for
[several views](#several-views-of-one-document): whichever view acted, the changes are the
document's, so they map positions for every view and for the host. `Session::apply_with_changes`
and `Session::apply_on_with_changes` are the [Session](#session)'s. `ChangeSet` and `Assoc` are
helix's, re-exported at the crate root; the rest of helix's API is under `caretline::helix`.

## Read the text and the selection

Positions are **char indices** into the rope (Unicode scalar values, not bytes). Every
position `update` produces is on a grapheme boundary.

```rust
use caretline::update::selection_text;
use caretline::{replay, script_to_msgs, State, Viewport};

let mut state = State::new("one\ntwo three", None, Viewport { width: 40, height: 5 });
replay(&mut state, script_to_msgs("<down><s-a-right>", 0).unwrap());

let text = state.doc.text.slice(..);
let range = state.view.selection.primary();       // anchor and head
assert_eq!((range.anchor, range.head), (4, 7));
assert_eq!((range.from(), range.to()), (4, 7)); // ordered ends
assert_eq!(range.slice(text).to_string(), "two");
assert_eq!(selection_text(&state).as_deref(), Some("two"));

// Line and column of the caret (0-based):
let line = text.char_to_line(state.caret());
let col = state.caret() - text.line_to_char(line);
assert_eq!((line, col), (1, 3));

// Every range, for multi-range selections:
for r in state.view.selection.iter() {
    let _ = (r.anchor, r.head);
}
```

### Set the selection yourself

Replace `state.view.selection` with a Helix `Selection`, then call `sanitize` to snap it to
grapheme boundaries and the text's length, and `layout::ensure_caret_visible` to bring the
primary caret into view (nothing else will: a `Tick` never moves the view):

```rust
use caretline::helix::{Range, Selection, SmallVec};
use caretline::layout::ensure_caret_visible;
use caretline::{update, Msg, State, Viewport};

let mut state = State::new("a-b-c", None, Viewport { width: 40, height: 5 });
// Two carets, after "a" and after "b". The second is primary.
let ranges: SmallVec<[Range; 1]> = [Range::point(1), Range::point(3)].into_iter().collect();
state.view.selection = Selection::new(ranges, 1);
state.sanitize();
ensure_caret_visible(&mut state);
update(&mut state, Msg::InsertText { text: "!".into() });
assert_eq!(state.doc.text.to_string(), "a!-b!-c");
assert_eq!(state.view.selection.len(), 2);
```

## Render

`view` returns a `Frame`: `height` rows of `width` cells, and the caret's cell when it's on
screen. Each cell has a grapheme `symbol` and a `role`. A wide grapheme takes two cells; the
second has an empty symbol.

```rust
use caretline::view::Role;
use caretline::{view, State, Viewport};

let state = State::new("hi 漢字", None, Viewport { width: 12, height: 2 });
let frame = view(&state);
assert_eq!(frame.cell(3, 0).symbol, "漢");
assert_eq!(frame.cell(4, 0).symbol, "");          // the second half of 漢
assert_eq!(frame.cell(0, 1).role, Role::Status);  // the last row is the status bar
assert_eq!(frame.cursor, Some((0, 0)));
print!("{}", frame.to_text());                     // or frame.to_ansi()
```

| `Role` | Meaning |
|---|---|
| `Text` | Document text |
| `Selection` | Selected text |
| `Caret` | A caret with no selection other than the primary, drawn as a cell (the primary is `frame.cursor`, the terminal's cursor). Only a focused view draws them |
| `Status` | The status bar |
| `StatusAccent` | The dirty marker `[+]` in the status bar |
| `Hang` | A block's hang, with the [outline layout](structure.md#the-outline-layout) |
| `Named(i)` | A role a host named in a [decoration](structure.md#decorations) or a frame pass: `frame.role_name(role)` |

The engine never picks colours. Your renderer maps roles to styles. Each cell also has the
`char_idx` of the document char it shows (none for blank cells and the status bar) and
`flags` (`CellFlags`: `DIM`, `RING`, set only by a host's [frame pass](embedding.md#frame-passes),
for your renderer to draw dimmed or ringed), and the frame has one `RowInfo` per row: what the
row shows (a line's text with its char range, a block's blank row, a host's row, past the end,
the status bar). `view::hit(doc, view, col, row)` says what a cell means for a click, and
`frame.region_at(x, y)` which region a frame pass put there.

`view(&state)` and `view::render(doc, view)` run the host's frame passes;
`view::render_plain(doc, view)` draws without them, and `view::render_skipping(doc, view,
&["name"])` without the named ones. `Frame::new(width, height)` makes a blank frame, and its
writers (`set`, `restyle`, `flag`, `role`) never split a wide grapheme.

### Where a position is on screen

`view::locate(doc, view, pos)` is the inverse of `hit`, through the same layout: the cell a
char position is drawn at, `Locate::At { x, y }` (where `hit` finds the position again), or
which way it lies: `Above`, `Below`, `Left` or `Right` (a line that doesn't wrap, scrolled
sideways) or `Folded { block }` (the outermost folded block hiding it). Use it to put
something of yours next to the text: a hint, a remote caret, a badge. Its JSON is
`{"kind": "at", "x": 1, "y": 1}`.

```rust
use caretline::view::{hit, locate, Hit, Locate};
use caretline::{State, Viewport};

let state = State::new("one\ntwo\nthree\n", None, Viewport { width: 20, height: 3 });
// Two text rows and the status bar: "one" and "two" show, "three" is below.
assert_eq!(locate(&state.doc, &state.view, 5), Locate::At { x: 1, y: 1 });
assert_eq!(hit(&state.doc, &state.view, 1, 1), Hit::Text { pos: 5 });
assert_eq!(locate(&state.doc, &state.view, 9), Locate::Below);
```

### A caret after hanging space

In an outline's prose wrapping, the space after a word that fills its row hangs past the
column instead of starting the next row, and so does the end of the text (or a line end)
right after it: typing a space at the end of a full row adds no row. The caret there is drawn
right after the hanging space when the view has a cell for it, else in the view's last column,
on the same row; `view::locate` gives that cell, and a click in the last cell (`view::hit`,
`Msg::Click`) puts the caret at the row's end. The next character starts the next row.
`Layout::of(doc, view).hangs(line)` says whether a line wraps this way (outline content, not a
fence or a plain document).

```rust
use caretline::helix::Selection;
use caretline::layout::Layout;
use caretline::outline::markdown;
use caretline::view::{locate, Locate};
use caretline::{update, view, Msg, OutlineConfig, OutlineLayout, Viewport};

// A bullet whose 72 chars fill the column: its content runs from x = 6 to 77.
let md = format!("- {}", "a".repeat(72));
let mut s = markdown::load(&md, None, Viewport { width: 80, height: 4 }, OutlineConfig::default());
s.view.layout = Some(OutlineLayout::default());
s.view.selection = Selection::point(s.doc.text.len_chars());
update(&mut s, Msg::InsertText { text: " ".into() });
// The space hangs in cell 78 and the caret stays on row 0, after it.
assert!(Layout::of(&s.doc, &s.view).hangs(0));
assert_eq!(view(&s).cursor, Some((79, 0)));
assert_eq!(locate(&s.doc, &s.view, s.doc.text.len_chars()), Locate::At { x: 79, y: 0 });
```

### The cell's size in pixels

A host that draws pixels over the cells (an image, a soft shadow) needs the cell's size.
It arrives as a message, so whatever is drawn from it is a function of the state and replays
the same on any machine: `Msg::Resize`'s optional `cell_px` sets `View::cell_px`, and every
frame carries it as `Frame::cell_px`. A resize without it keeps the view's;
`Msg::resize(width, height)` builds one.

```rust
use caretline::{update, view, CellPx, Msg, State, Viewport};

let mut state = State::new("hi", None, Viewport { width: 80, height: 24 });
update(&mut state, Msg::Resize { width: 80, height: 24, cell_px: Some(CellPx::new(8, 16)) });
assert_eq!(view(&state).cell_px, Some(CellPx { w: 8, h: 16 }));
update(&mut state, Msg::resize(100, 30)); // no pixels: keeps the view's
assert_eq!(state.view.cell_px, Some(CellPx::new(8, 16)));
```

## Handle effects

| Effect | Do this | Then send |
|---|---|---|
| `WriteFile { path, text }` | Write `text` to `path` | `Msg::Saved`, or `Msg::SaveFailed { err }` |
| `ClipboardSet { text }` | Put `text` on the system clipboard | nothing |
| `Quit` | Close the editor | nothing |

```rust
use caretline::{update, Effect, Msg, State};

fn dispatch(state: &mut State, msg: Msg) -> bool {
    let mut queue = vec![msg];
    while let Some(msg) = queue.pop() {
        for effect in update(state, msg) {
            match effect {
                Effect::WriteFile { path, text } => queue.push(match std::fs::write(&path, text) {
                    Ok(()) => Msg::Saved,
                    Err(e) => Msg::SaveFailed { err: e.to_string() },
                }),
                Effect::ClipboardSet { text } => { let _ = text; /* your clipboard */ }
                Effect::Quit => return false,
                _ => {} // notices, outline effects, refused, and kinds added later
            }
        }
    }
    true
}
```

The pure keymap maps a paste key to `Msg::Paste { text: None }`, which pastes the internal
register. To paste from the system clipboard, read it in your runtime and send
`Msg::Paste { text: Some(..) }`. That way the message records exactly what was pasted.

`Effect` is `#[non_exhaustive]`: keep a wildcard arm.

## Several views of one document

A `State` is one `Document` (text, marks, undo, outline) and one `View` (selection, scroll,
viewport, folds). To show one document in several places (a main editor and a side panel, or a
person's caret and an agent's), keep one `Document` and several `View`s, and apply messages
with `update_doc`. An edit through one view maps every other view's selection; undo is the
document's; a read-only view is refused edits.

```rust
use caretline::view::render;
use caretline::{update_doc, Effect, ExtChange, Msg, State, View, Viewport};

let mut doc = State::new("one\ntwo\n", None, Viewport { width: 40, height: 5 }).doc;
let mut views = [View::new(Viewport { width: 40, height: 5 }), View::new(Viewport { width: 20, height: 3 }).read_only(true)];
views[1].selection = caretline::helix::Selection::point(4);    // on "two"
update_doc(&mut doc, &mut views, 0, Msg::InsertText { text: "zero\n".into() });
assert_eq!(views[1].caret(), 9);                                    // still on "two"
assert_eq!(update_doc(&mut doc, &mut views, 1, Msg::DeleteBackward), vec![Effect::Refused]);

// A change from elsewhere maps every view and stays out of undo.
update_doc(&mut doc, &mut views, 0, Msg::External { changes: vec![ExtChange::Replace { from: 0, to: 0, text: "> ".into() }] });
update_doc(&mut doc, &mut views, 0, Msg::Undo);
assert_eq!(doc.text.to_string(), "> one\ntwo\n");
let _panel = render(&doc, &views[1]);
```

`update_doc_with_changes` also returns the message's text changes
([Map your own positions](#map-your-own-positions-through-each-message)).
`State::from_parts(doc, view)` and `state.into_parts()` move between the two forms. A view kept
apart from `update_doc` misses the rebases; `View::fit(&doc)` at least clamps it to the
document.

## Serialize and replay

```rust
use caretline::trace::{parse_msgs, replay_trace, TraceLine};
use caretline::{update, State, Viewport};

let start = State::new("", None, Viewport { width: 40, height: 5 });

// A state round-trips through JSON, undo history included.
let back = State::from_json(&start.to_json()).unwrap();
assert_eq!(back, start);

// Messages parse from JSON Lines (or a JSON array).
let msgs = parse_msgs(r#"{"msg":"insert_text","text":"hi"}
{"msg":"move","dir":"backward","by":"word","extend":true}"#).unwrap();

// A trace is the initial state, then every message.
let mut live = start.clone();
let mut trace = TraceLine::State(Box::new(start)).to_line() + "\n";
for msg in msgs {
    trace += &(TraceLine::Msg(msg.clone()).to_line() + "\n");
    update(&mut live, msg);
}
let (replayed, count) = replay_trace(&trace).unwrap();
assert_eq!((replayed, count), (live, 2));
```

`State::from_json` repairs hand-edited states (see
[architecture.md](architecture.md#rehydration)). `to_json` is pretty-printed; use
`serde_json::to_string(&state)` for one line.

A restored state keeps the scroll it was saved with. If you restore into a different
viewport, or change the selection, scroll or viewport of a state yourself (`state.set` over
the protocol does the same), call `layout::ensure_caret_visible(&mut state)` or send
`Msg::resize(width, height)` to bring the caret into view: no passive message (`Tick`,
`Frame`, `Ext`, …) will.

Deserializing goes through `state::StateInput`, where every field is optional, so a minimal
state works:

```rust
use caretline::State;

let s = State::from_json(r#"{"text":"hello\n","viewport":{"width":40,"height":10}}"#).unwrap();
assert!(!s.doc.dirty && s.doc.history.len() == 1); // clean, with a fresh history
```

Undo grouping constants live in `caretline::state`: `RUN_GAP_MS` (1500), `RUN_MAX_CHARS`
(256) and `RUN_WORD_BREAK_CHARS` (128). See
[architecture.md](architecture.md#undo-grouping-worked-through).

## Block marks

`state.doc.marks` holds ids at line starts that follow their lines through every edit, undo and
redo (see [architecture.md](architecture.md#block-marks)). Add marks directly, outside the
undo history, when you load a document; use `update::mark_only_edit` for a change of marks
that should be one undo step.

```rust
use caretline::{update, MarkAttrs, Msg, State, Viewport};

let mut s = State::new("Groceries\nmilk\n", None, Viewport { width: 40, height: 5 });
let list = s.doc.marks.mint(0);                        // MarkId(0) on line 0
let milk = s.doc.marks.mint(s.doc.text.line_to_char(1));   // MarkId(1) on line 1
s.doc.marks.set_attrs(milk, MarkAttrs { gap: Some(false), data: Some(serde_json::json!({ "row": 7 })) });

update(&mut s, Msg::InsertText { text: "Weekly ".into() });  // at the start of line 0
assert_eq!(s.doc.marks.pos(list), Some(0));                      // still line 0
update(&mut s, Msg::Move { dir: caretline::Dir::Forward, by: caretline::By::DocEnd, extend: false });
update(&mut s, Msg::Undo);                                   // the marks come back exactly
assert_eq!(s.doc.marks.pos(milk), Some(s.doc.text.line_to_char(1)));
```

| `Marks` method | Does |
|---|---|
| `mint(pos)`, `mint_with(pos, attrs)` | A new id at a line start (or the line's existing mark) |
| `insert(Mark)` | Put a known id back; refuses a taken line or a live id |
| `remove(id)`, `remove_at(pos)`, `remove_range(from, to)` | Take marks out, returning them |
| `at(pos)`, `mark_at(pos)`, `pos(id)`, `get(id)`, `in_range(from, to)`, `at_or_before(pos)` | Look marks up |
| `attrs(id)`, `set_attrs(id, attrs)`, `set_gap(id, gap)`, `set_data(id, data)` | A block's attributes: its blank row and the host's payload |
| `iter()`, `len()`, `next_id()` | Walk them in document order |

## Block documents

`markdown::load` opens Markdown as a [block document](structure.md); `state.enable_outline`
turns an existing state into one. `state.blocks()` gives the derived blocks, each with its
mark id. `save` writes Markdown back.

```rust
use caretline::outline::markdown;
use caretline::{update, By, Dir, Msg, OutlineConfig, Viewport};

let mut s = markdown::load("- Pay rent\n", None, Viewport { width: 40, height: 6 }, OutlineConfig::default());
update(&mut s, Msg::Move { dir: Dir::Forward, by: By::LineEnd, extend: false });
update(&mut s, Msg::InsertNewline);                 // a new item below
update(&mut s, Msg::InsertText { text: "Call Ana".into() });
update(&mut s, Msg::Indent);                        // nested under the first
assert_eq!(markdown::to_file(&s), "- Pay rent\n  - Call Ana\n");
```

### What changed since you last looked

A host that mirrors the blocks (a database row per block, a search index) asks
`state.doc.take_touched()` after each message and re-reads only that range. It returns the
chars that changed since the last call, as `[from, to)` in the current text, or `None` when
nothing did; the first call after a load or a repair (and after `enable_outline`) gives the
whole text. In a block document the range covers whole blocks: every block whose text, kind,
depth, tag, blank row or mark (its payload too) changed, and the blocks a change reshaped past
its own chars, such as the block after one whose kind changed or the lines a code fence takes
in. Text an edit put back as it was doesn't count, so the range is as small as the change and
never misses a block that changed ([performance.md](performance.md#touched-ranges)).

A host's own edit is applied as the least change: rewrite the whole text to change one block,
and only that block is touched; every other block keeps its mark, id and payload.

```rust
use caretline::outline::markdown;
use caretline::{update, Msg, OutlineConfig, Viewport};

let mut s = markdown::load("- a\n- b\n- c\n", None, Viewport { width: 40, height: 6 }, OutlineConfig::default());
s.doc.take_touched();                               // the whole text, after the load
let ids: Vec<_> = s.blocks().unwrap().blocks.iter().map(|b| b.id).collect();

let all = s.doc.text.len_chars();
update(&mut s, Msg::Edit { changes: vec![(0, all, "- a\n- B\n- c".into())], join: false });
assert_eq!(s.doc.take_touched(), Some((4, 7)));     // "- B": the one block that changed
let after: Vec<_> = s.blocks().unwrap().blocks.iter().map(|b| b.id).collect();
assert_eq!(ids, after);                             // every block kept its id
assert_eq!(s.doc.take_touched(), None);
```

## Session

A `Session` wraps a `State` with a revision counter and an in-memory trace. It is the library
face of the [state protocol](protocol.md).

| Method | Does |
|---|---|
| `Session::new(state)` | Starts at rev 0, with the state as the trace's first line |
| `state()`, `rev()`, `trace()`, `trace_jsonl()` | Read the state, the rev and every trace line kept |
| `segment_trace()`, `segment_rev()` | The current segment (from the latest `state` line) and the rev it starts at |
| `trace_since(rev)`, `trace_start_rev()` | The lines after `rev` (`None` once trimmed), and the oldest rev it answers for |
| `checkpoint()` | Starts a new segment with the current state; the rev doesn't change |
| `set_trace_limit(lines)` | Bounds the kept trace (default `session::DEFAULT_TRACE_LIMIT`, 100,000): older segments go first, and a segment that alone outgrows it is cut by an automatic checkpoint |
| `trace_lines_total()`, `trace_lines_from(n)` | Lines recorded so far (dropped ones included) and the lines from absolute position `n`, for copying the trace to a file |
| `apply(msg) -> Vec<Effect>` | Applies one message (rev + 1), returns effects unperformed |
| `apply_all(msgs)` | Applies several |
| `apply_with(msg, exec)` | Applies, performs effects with `exec`, and applies the messages `exec` returns (such as `Saved`) |
| `keys(script)` | Runs a key script; returns the messages and effects |
| `set_state(state)` | Replaces the state (sanitized) and starts a new trace segment; rev + 1. Replaces every caret and the undo history: not for use while someone types |
| `set_text(text)`, `set_text_on(id, text)`, `text_change(text)` | Puts in a whole new text, changing only what differs, as one change from elsewhere (`Msg::External`, outside the undo history); every view keeps its caret, selection, scroll and folds on its text. Returns the message applied (`None` when nothing differs). `text_change` builds it without applying it |
| `open_view(view) -> id`, `close_view(id)`, `views()`, `view(id)`, `state_of(id)` | Other views of the document (view 0 is the state's own); opening and closing is a change and is traced |
| `apply_on(id, msg)`, `apply_with_on`, `keys_on(id, script)` | Apply through view `id`; every other view is rebased |
| `apply_with_changes(msg)`, `apply_on_with_changes(id, msg) -> (Vec<Effect>, Option<ChangeSet>)` | `apply` and `apply_on`, also returning the message's text changes ([Map your own positions](#map-your-own-positions-through-each-message)); `None` when the text didn't change or there is no such view |
| `render_view(id, size)` | The frame of view `id` |
| `state_lines()` | The lines a segment starts with: the state and a `view_open` for each other view |
| `frame()` | `view` of the current state |
| `render(w, h)` | The frame at another size, without changing the session |
| `handle(line, exec)` | Answers one protocol request line (`protocol::Handled`): the response line, the `Change` it made and any `subscribe` control |
| `handle_at(line, exec, clock_ms)` | `handle` for a runtime with a clock: ticks to `clock_ms` before a request's messages (see [Time](protocol.md#time)) |
| `handle_client(line, exec, clock_ms, own)` | `handle_at` for one client of a server with a person on view 0: `msgs`, `keys` and `text.set` without a `view` go through the client's own view `own` (opened on first use; the caller closes it when the client goes). See [Collaborating with a person](protocol.md#collaborating-with-a-person) |

`protocol::event_line(&session, &change, &subscription, source)` builds the event a
subscriber receives for a change.

```rust
use caretline::{Session, State, Viewport};

let mut s = Session::new(State::new("", None, Viewport { width: 30, height: 4 }));
let (msgs, effects) = s.keys("hi<c-s>").unwrap();
assert_eq!((msgs.len(), s.rev()), (3, 3));
assert!(effects.is_empty()); // no path, so save only sets a status message

let reply = s.handle(r#"{"id":1,"op":"render","w":30,"h":4}"#, None);
println!("{}", reply.response); // {"id":1,"result":{"rev":3,"w":30,…}}
```

## A complete program

This is [`examples/basic.rs`](../crates/caretline/examples/basic.rs). It builds a
state, drives it with messages and keys, handles effects, renders, round-trips the state
through JSON and replays the trace.

```rust
use caretline::trace::{replay_trace, TraceLine};
use caretline::update::selection_text;
use caretline::{script_to_msgs, update, view, By, Dir, Effect, Msg, State, Viewport};

fn main() {
    // 1. A state: the text, an optional file path (where `save` writes) and a viewport.
    let start = State::new(
        "hello world\n",
        Some("draft.md".into()),
        Viewport {
            width: 30,
            height: 4,
        },
    );
    let mut state = start.clone();
    let mut trace = vec![TraceLine::State(Box::new(start))];

    // 2. Messages are plain values. The clock arrives as a message too.
    let mut msgs = vec![
        Msg::Tick { now_ms: 1_000 },
        Msg::Move {
            dir: Dir::Forward,
            by: By::Word,
            extend: false,
        },
        Msg::Move {
            dir: Dir::Forward,
            by: By::Word,
            extend: true,
        },
    ];
    // 3. Or keys, through the pure keymap (the same notation as `caretline --keys`).
    msgs.extend(script_to_msgs("<c-c>", 1_000).expect("valid key script"));

    // 4. Apply them one by one. `update` never performs I/O; it returns effects.
    for msg in msgs {
        trace.push(TraceLine::Msg(msg.clone()));
        for effect in update(&mut state, msg) {
            match effect {
                Effect::ClipboardSet { text } => println!("effect: copy {text:?} to the clipboard"),
                Effect::WriteFile { path, .. } => println!("effect: write {path}"),
                Effect::Quit => println!("effect: quit"),
                // Outline documents also report notices and block changes; host commands their own.
                other => println!("effect: {other:?}"),
            }
        }
    }

    // 5. Read the result.
    let primary = state.view.selection.primary();
    println!("text:      {:?}", state.doc.text.to_string());
    println!("selection: anchor {} head {}", primary.anchor, primary.head);
    println!("selected:  {:?}", selection_text(&state));

    // 6. Type over the selection, then save. The runtime performs the write and answers
    //    with `Saved` (or `SaveFailed`), which clears the dirty flag.
    for msg in [Msg::InsertText { text: " there".into() }, Msg::Save] {
        trace.push(TraceLine::Msg(msg.clone()));
        for effect in update(&mut state, msg) {
            if let Effect::WriteFile { path, text } = effect {
                println!("would write {} bytes to {path}", text.len());
                trace.push(TraceLine::Msg(Msg::Saved));
                update(&mut state, Msg::Saved);
            }
        }
    }
    println!("dirty:     {}", state.doc.dirty);

    // 7. Render. `view` is pure: a grid of cells plus the caret's cell.
    let frame = view(&state);
    print!("{}", frame.to_text());
    println!("cursor at  {:?}", frame.cursor);

    // 8. The whole state round-trips through JSON, history included.
    let json = state.to_json();
    let mut back = State::from_json(&json).expect("state parses");
    assert_eq!(back, state);
    update(&mut back, Msg::Undo);
    println!("after undo: {:?}", back.doc.text.to_string());

    // 9. A trace (initial state + every message) replays to the same state.
    let jsonl: String = trace.iter().map(|l| l.to_line() + "\n").collect();
    let (replayed, count) = replay_trace(&jsonl).expect("trace replays");
    assert_eq!(replayed, state);
    println!("replayed {count} messages: same state");
}
```

Its output:

```text
effect: copy " world" to the clipboard
text:      "hello world\n"
selection: anchor 5 head 11
selected:  Some(" world")
would write 12 bytes to draft.md
dirty:     false
hello there


 draft.md  saved draft.m 1:12
cursor at  Some((11, 0))
after undo: "hello world\n"
replayed 7 messages: same state
```

The status bar clips its message (`saved draft.md`) so the `line:col` on the right stays
visible.
