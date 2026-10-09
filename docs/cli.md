# The `caretline` command

`caretline` is the interactive editor and a headless tool for states, snapshots and replay.
It comes from the `caretline-cli` crate. Every output on this page is from a real run.

## Install

```sh
curl -fsSL https://caretline.app/install.sh | sh -s -- --demo   # a prebuilt binary, then the welcome demo
cargo install caretline-cli   # or build it
cargo install --locked --path crates/caretline-cli   # from a checkout
cargo run -p caretline-cli -- draft.md                # or run it from the checkout
```

See the [Quickstart](quickstart.md) for what the installer does and what the demos show.

## Commands at a glance

| Command | Does |
|---|---|
| `caretline demo [welcome\|tour\|scenes\|agent\|layers\|showcase]` | Built-in demos, no files needed; with no name, [the welcome](#the-welcome). See the [Quickstart](quickstart.md) and [Layers over the tour](#layers-over-the-tour). `--snapshot WxH` prints a demo's first frame; `demo agent --headless` runs the agent against a headless editor and prints a JSON report |
| `caretline [FILE]` | Edit FILE interactively (created on first save) |
| `caretline FILE --trace T.jsonl` | Edit, recording the session to a trace |
| `caretline FILE --no-mouse` | Edit without capturing the mouse (by default: click, Shift-click, drag, and a drag held on an edge row keeps scrolling, see [messages.md](messages.md#selecting-by-dragging)) |
| `caretline --new-state FILE [--size WxH]` | Print an initial state for FILE as JSON |
| `caretline --state S.json` | Edit a saved state interactively |
| `caretline --state S.json --keys SCRIPT …` | Apply a key script, headless |
| `caretline --state S.json --msgs M.jsonl …` | Apply messages, headless (`-` reads stdin) |
| `caretline --replay T.jsonl …` | Start from a trace with all its messages applied, headless |
| `… --snapshot WxH [--format text\|ansi]` | Print the rendered frame |
| `… --dump-state OUT.json` | Write the final state (`-` for stdout) |
| `… --effects` | Print the effects `update` returned, one JSON per line |
| `… --size WxH` | Resize before applying messages |
| `caretline serve`, `caretline send`, `--listen` | The state protocol, see [protocol.md](protocol.md) |
| `caretline bench` | Protocol throughput and latency (build with `--release`) |
| `caretline --outline FILE` | Edit FILE as [Markdown blocks](markdown.md): lists, headings and blocks with their own keys, Markdown in and out (also for `--new-state` and `serve`) |
| `caretline --layout FILE` | As `--outline`, with the [outline layout](structure.md#the-outline-layout): markers in a hang with plain glyphs, a column per depth (also for `serve`) |
| `caretline keys [--outline] [--json]` | Every key and the command it runs, from caretline's [command catalog](keys.md). In the editor, F1 or Alt-? shows the same |
| `… --no-status-bar` | Hide the status bar (`config.status_bar = false`): every row shows text |
| `… --max-fps FPS` | Editor: repaint at most this many times a second, coalescing changes in between (default 120; `0` repaints after every batch of input). See [performance.md](performance.md#how-the-live-editor-paints) |
| `… --frame-clock FPS` | Editor: start with a frame clock (a `frame` message FPS times a second). Off by default |
| `… --stats` | Editor: print the repaint count and mean repaint time on exit |
| `… --trace-limit LINES` | Editor and `serve`: bound the in-memory trace `trace.get` serves (default 100,000 lines; see [protocol.md](protocol.md#traces)) |
| `caretline serve … --exit-with-parent` | Exit (removing the socket) once the process that started the server is gone, even if it was killed: for servers a test or a supervisor starts (see [protocol.md](protocol.md#transports)) |

A run is **headless** when any of `--snapshot`, `--dump-state`, `--msgs`, `--keys` or
`--replay` is given. Headless runs never perform effects: no file is written and the
clipboard isn't touched. `--effects` shows what would have happened.

## Edit a file

```sh
caretline draft.md
```

The keys are macOS text-field keys with Ctrl twins (see [messages.md](messages.md#the-keymap)).
`Ctrl-S` saves and `Ctrl-Q` quits; with unsaved changes, press `Ctrl-Q` twice. Cmd (⌘) keys
need a terminal that speaks the kitty keyboard protocol (kitty, WezTerm, Ghostty, iTerm2 with
the option on). caretline turns it on when the terminal supports it. Copy also writes the
system clipboard (`pbcopy`, `wl-copy`, `xclip` or `xsel`, else OSC 52), and paste reads it
when it can.

Saving writes a temporary sibling file and renames it over the target, so a failed write
never leaves half a file.

## The state, snapshot and replay workflow

This walks through every headless flag on a small file.

### 1. Capture a state

```console
$ printf '# Plan\n\nShip the editor docs.\n' > plan.md
$ caretline --new-state plan.md --size 40x6 > s0.json
$ caretline --state s0.json --snapshot 40x6
# Plan

Ship the editor docs.


 plan.md                            1:1
```

`--new-state` uses 80x24 unless you pass `--size`. The state is pretty-printed JSON: the
text, the selection, the scroll, the viewport, the undo history and the config. You can edit
it by hand; loading repairs anything out of range.

You can also write a state from scratch. Only `text` is needed; everything else takes
`--new-state`'s defaults (a caret at 0, 80x24, a fresh history, a clean document):

```console
$ cat > min.json <<'EOF'
{"text": "hello\nworld\n", "selection": {"ranges": [{"anchor": 0, "head": 5}]}, "viewport": {"width": 20, "height": 4}}
EOF
$ caretline --state min.json --keys 'bye' --snapshot 20x4
bye
world

 [scratch] [+]  1:4
```

See [architecture.md](architecture.md#rehydration) for every default.

### 2. Drive it with keys, and save the result

```console
$ caretline --state s0.json --keys '<d-down>Then review them.' --snapshot 40x6 --dump-state s1.json
# Plan

Ship the editor docs.
Then review them.

 plan.md [+]                       4:18
```

`<d-down>` is ⌘↓ (document end). The status bar shows the file name, `[+]` for unsaved
changes, and `line:col`.

### 3. Select, undo, inspect

```console
$ caretline --state s1.json --keys '<s-a-left><s-a-left>' --snapshot 40x6
# Plan

Ship the editor docs.
Then review them.

 plan.md [+]                12 sel  4:6
```

```console
$ caretline --state s1.json --keys '<c-z>' --snapshot 40x6
# Plan

Ship the editor docs.


 plan.md                            4:1
```

The undo history is part of the state, so `s1.json` can still undo the typing it recorded.
After the undo, the document matches the saved revision again, so `[+]` is gone.

### 4. See effects instead of performing them

```console
$ caretline --state s1.json --keys '<c-s>' --effects --dump-state /dev/null
{"effect":"write_file","path":"plan.md","text":"# Plan\n\nShip the editor docs.\nThen review them."}
$ cat plan.md
# Plan

Ship the editor docs.
```

The file is unchanged: headless runs report effects and never perform them.

### 5. Apply a message file

```console
$ cat m.jsonl
{"msg":"move","dir":"backward","by":"word","extend":true}
{"msg":"copy"}
{"msg":"move","dir":"forward","by":"doc_end"}
{"msg":"insert_newline"}
{"msg":"paste"}
$ caretline --state s1.json --msgs m.jsonl --effects --snapshot 40x6
{"effect":"clipboard_set","text":"them."}
# Plan

Ship the editor docs.
Then review them.
them.
 plan.md [+]                        5:6
```

`--msgs -` reads the messages from stdin. A JSON array works too.

### 6. Resize

`--size` resizes before the messages; `--snapshot` resizes after them if its size differs.
At 20 columns the long line wraps:

```console
$ caretline --state s1.json --size 20x6 --snapshot 20x6
# Plan

Ship the editor
docs.
Then review them.
 plan.md [+]   4:18
```

### 7. Record and replay a session

```sh
caretline draft.md --trace t.jsonl          # edit, then quit
caretline --replay t.jsonl --snapshot 80x24 # the final frame, exactly
caretline --replay t.jsonl --dump-state -   # the final state
```

`--trace` appends, so restarting into the same file adds a new `state` line and replay
continues from it. The file gets every line, also past `--trace-limit`, which bounds only
the in-memory trace. `--replay` reads any trace: a full file, the output of
`caretline send trace.get --raw` (one segment), or of `trace.get all`; a `state` line in the
middle (a `state.set` or checkpoint) restarts the replay from that state. The fixture `session.trace.jsonl` is a recorded session:

```console
$ caretline --replay crates/caretline-cli/fixtures/session.trace.jsonl --snapshot 36x8

Shift and the arrows grow one from
the caret's end,
and a plain arrow collapses it to
the edge.

arrows grow one from the caret's
 select.md [+]                 5:33
```

## Snapshot formats

`--format text` (the default) prints the rows with trailing spaces trimmed. `--format ansi`
adds styling: the selection in reverse colours, the status bar highlighted, and the caret as
an underlined reverse cell (a snapshot has no terminal cursor).

```console
$ caretline --state s1.json --keys '<s-a-left>' --snapshot 40x6 --format ansi | sed -n 4p | cat -v
^[[0mThen review ^[[0m^[[7;4mt^[[0m^[[30;46mhem.^[[0m                       ^[[0m
```

Here `^[[7;4m` marks the caret cell and `^[[30;46m` the rest of the selection.

## Fixtures

[`crates/caretline-cli/fixtures`](../crates/caretline-cli/fixtures) holds saved states
with their text and ANSI snapshots. The CLI tests check that each one still renders the
same.

| Fixture | Shows |
|---|---|
| `wrapped-paragraph` | A Markdown paragraph soft-wrapped at 44 columns, caret on a wrapped row |
| `emoji-line` | Emoji (skin tone, ZWJ family, flags), combining accents and wide CJK, with a selection |
| `mid-selection` | A selection across lines, ending inside a wrapped row |
| `after-undo` | An edit undone, with the redo step still in the history |
| `no-wrap-table` | Wrapping off: a long table row scrolled sideways |
| `session.trace.jsonl` | A recorded session; `session.snapshot.txt` is its replay at 36x8 |
| `outline-trip` | `trip.md` opened as blocks: a heading, paragraphs, a nested list, an image block and a numbered list, with blank rows drawn as virtual rows |
| `outline-edited` | The same after keys: a nested item added under another, an item moved up |

```console
$ caretline --state crates/caretline-cli/fixtures/emoji-line.state.json --keys '<s-right><s-right>' --snapshot 40x6
Emoji 👍🏽 and family 👨‍👩‍👧, accents café
and résumé, wide 漢字かな, flags 🇫🇷🇯🇵.
Second line: ½ ⅓ → ✓


 emoji.md                   5 sel  1:19
```

```console
$ caretline --state crates/caretline-cli/fixtures/no-wrap-table.state.json --keys '<home>' --snapshot 48x6
| Name | Role | Notes |
|---|---|---|
| Ada | engine | Wrapping is off here, so long t
| Lin | review | Short row. |

 nowrap.md                                  3:1
```

The status bar's `N sel` counts Unicode scalar values, while `line:col` counts graphemes.

## Collaborating with a person

While someone types in `caretline FILE --listen`, push changes so their caret never moves:

```sh
caretline send --latest set-text draft-v2.md     # text.set: only what differs changes
caretline send --latest keys '<d-down>- a line'  # through this connection's own view
```

- `set-text` (`text.set`) diffs your text against the live one and applies only the changes,
  outside the undo history. Every caret, selection, scroll and fold stays on its text, and
  text put in exactly at the person's caret goes after it. Pass `if_rev` in a raw request to
  write only if nothing changed since you read.
- `msgs` and `keys` go through the connection's own view (a copy of the person's, closed
  when `send` exits), not the person's. `--view 0` acts as the person, for demos and tests.
- `set-state` (`state.set`) replaces the whole state, the person's caret and undo history
  included: for time travel and hand-off, not for collaborating. The `frame` op (below)
  replaces the screen on purpose too.

See [protocol.md](protocol.md#collaborating-with-a-person).

## Animate a live editor

`caretline demo scenes` plays six ASCII scenes (warp, donut, cube, tunnel, plasma, fire) inside
the editor, pushed by a client on its own socket with the [`frame`](protocol.md#frames) op (see
the [Quickstart](quickstart.md)). With `--bench`, it plays them into an editor that is already
running instead, paced against absolute deadlines, and reports the frames per second it
achieved:

```sh
caretline draft.md --listen                                       # in one terminal
caretline demo scenes --bench                                     # in another
caretline demo scenes --bench --fps 120 --scene donut,plasma --seconds 5
```

| Flag | Does |
|---|---|
| `--fps 60,120,0` | Target rates to play each scene at (`0` is unthrottled). Default `60,120,0`. Without `--bench`, one rate (default 60) |
| `--seconds S` | How long each scene plays at each rate (default 3; without `--bench`, 8 per scene) |
| `--scene NAMES` | Some of `warp,donut,cube,tunnel,plasma,fire` (default all) |
| `--socket PATH` | The editor's socket (default: the newest live editor) |
| `--frames N`, `--size WxH` | Frames precomputed per scene (180), the scene size (the editor's text area) |

For 120 fps on screen, the terminal must paint that fast too: in WezTerm set
`config.max_fps = 120`, on a 120 Hz display.

## The welcome

`caretline demo` (or `caretline demo welcome`; new in 0.4.1, where 0.4.0 opened the tour) opens the welcome page: what caretline is, every
other demo as a chapter, and commands to try next. `↑`/`↓` (or `j`/`k`, `Tab`) choose, `Enter`
or `1`–`5` start a chapter, `p` switches the callout between pixels and cells, `q` quits. A
chapter that ends comes back to the page, ticked. `--snapshot WxH [--keys …]` prints the page
headless; `--dir`, `--no-mouse` and `--reduced-motion` pass through to the chapters.

## Timed interactive showcase

`caretline demo showcase` (or `./scripts/demo.sh` from a checkout) runs twelve timed
slides with real message-driven edits, multiple carets, overlays, a shared-document pane,
guarded remote writes, structural edits, serialization/replay checks, branded typography,
ASCII scenes, fractional-pixel motion and a continuous warp-logo finale. No agent is needed.
The [Quickstart](quickstart.md#caretline-demo-showcase-the-interactive-presentation) has the
slide sequence, all controls and examples for the live listener.

- Default: 14 seconds per slide, automatic advance, remain on the animated logo finale.
- `--reduced-motion`: freeze continuous graphics/ASCII animation; slides still advance.
- `--font-licenses`: print the bundled fonts' copyright notices and full SIL Open Font
  Licenses without opening the editor (also embedded in the standalone binary).
- `--seconds 2..300`: seconds per slide.
- `←`/`→` or `1`–`9`: jump to a slide, run its script and hold. `Space` pauses, `a` toggles
  automatic advance, `r` replays the slide, `Home` restarts, `End`/`0` jumps to the finale,
  `q` quits.
- `s` toggles spotlight; `p` compares pixels and cells; `t` switches pixel transport.
- `--socket PATH`: listen at this socket instead of the default discoverable editor socket.
  `send --latest subscribe state` receives real editor events throughout the presentation.
- `--headless`: run every cue without sleeping, print JSON with 13 measured invariant checks,
  and exit nonzero if any check fails. No socket or terminal is opened in this mode.
- `--snapshot WxH --keys '4<wait:7000>'`: preview the fourth slide at seven seconds, in cells.
  `<wait:MS>` advances this demo's actual timeline, including automatic slide transitions.

The deck is read-only to ordinary printable keys; presentation shortcuts drive it. Socket
clients can edit the underlying state. Slide navigation resets the document and closes its
secondary views. Use a direct Ghostty terminal at 100×30 for the best pixel presentation;
small windows and unsupported terminals use the same content with adaptive cell overlays.
Terminal pastes are consumed, while explicit socket edits remain allowed. Typography samples
use the site's licensed Geist/Geist Mono outlines at three raster sizes, not terminal font
escape sequences. The pixel motion is anti-aliased fractional-device-pixel geometry (not
RGB/LCD subpixel text rendering), targeted at 24 updates/s. The ASCII section reuses the
existing scene renderers; the finale loops until paused or quit. `--reduced-motion` freezes
that continuous motion. The showcase keeps at most 512 trace lines to bound continuous-frame
history; subscribers can still collect the full stream.

## Layers over the tour

`caretline demo layers` shows a hint with an
arrow, a ring and a spotlight over the tour's text, placed by
[`caretline-layers`](layers.md). The hint points at a word in the "Move" section and follows it
as the view scrolls; the text is read-only here.

- **In Ghostty** (and kitty) it draws in pixels: a sharp, neutral panel under the box's
  words, an anti-aliased arrow and ring without glow, and a veil with feathered holes, sent
  with the kitty graphics protocol inside each frame's synchronized update. The status bar
  shows the mode and controls first, then the cell size, transport and last pixel frame's
  bytes, rasters and time as space allows.
- **Elsewhere**, inside tmux or screen, and with `--snapshot`, it draws in cells: a crisp
  box, the arrow in box-drawing glyphs, the anchor in a ring style and everything else dimmed.
  The status bar says why pixels are off.

| Key | Does |
|---|---|
| `↑` `↓` `PgUp` `PgDn` | Scroll; the hint follows its word |
| `←` `→` `Home` `End` | Move the caret |
| `s` | Spotlight on or off |
| `p` | Pixels or cells (pixels only where the probe allowed them) |
| `t` | The pixel transport: inline (`t=d`, works over SSH) or temporary files (`t=t`, local) |
| `q`, `Esc`, `⌃C` | Quit |

At startup it probes the terminal (a graphics query, XTVERSION and the cell size, fenced by
DA1, 200 ms at most) and uses pixels when the graphics query says OK, the cell size comes back
and the terminal is Ghostty or kitty. `CARETLINE_LAYERS=auto|pixels|cells` overrides that
(`pixels` also tries an untested terminal, or one inside a multiplexer). After a font-size
change it asks for the cell size again.

```sh
caretline demo layers
CARETLINE_LAYERS=cells caretline demo layers
caretline demo layers --snapshot 80x24                 # the first frame, in cells
caretline demo layers --snapshot 60x20 --keys '<down><down>s'   # scrolled, spotlight off
```

An 80×24 or larger window gives the hint and controls room. For a short presentation, toggle
`s`, scroll with `↓`/`↑`, compare modes with `p`, try `t` in pixel mode, and quit with `q`.
`F1` or `Alt-?` covers the demo with the keys overlay (including its images); `Esc` closes
that overlay and restores the demo. With no keys overlay open, `Esc` quits the layers demo.

## Errors

Errors go to stderr with exit code 1:

```console
$ caretline --state s0.json --snapshot 80
caretline: bad size "80": expected WIDTHxHEIGHT, like 80x24
$ caretline --state s0.json --keys '<nope>'
caretline: unknown key <nope>
$ echo '{"msg":"jump"}' | caretline --state s0.json --msgs -
caretline: line 1: unknown variant `jump`, expected one of `insert_text`, `insert_newline`, … at line 1 column 13
```

## Limits

- One document of plain text or Markdown (`--outline` reads it as blocks). No syntax
  highlighting, search or multiple buffers yet.
- The engine edits multiple selections, but no key creates them yet.
- `End` on a wrapped row stops before the space where the row wraps.
- Display widths follow Helix's table (`unicode-width` 0.1.12). A terminal with different
  emoji widths can misplace the caret on those graphemes.
