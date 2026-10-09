# Quickstart

Try caretline in one line. This installs the `caretline` binary and opens a guided tour:

```sh
curl -fsSL https://caretline.app/install.sh | sh && caretline demo
```

The installer picks the build for your machine (macOS or Linux, Apple silicon, ARM or x86-64),
checks its sha256 and puts `caretline` in `~/.local/bin`. Nothing else is touched, except a
line that adds `~/.local/bin` to your `PATH` in your shell's startup file when it isn't there
yet. If the shell you ran it in doesn't have that directory on its `PATH`, the installer prints
the `export` line to run first.

Prefer to build it yourself? With a Rust toolchain:

```sh
cargo install caretline-cli
```

## Five demos

The tour, scenes and agent demos write their files to a fresh temporary directory, so there
is nothing to set up. Layers and showcase write no document files; showcase opens a local
editor socket and removes it on exit.

### `caretline demo showcase`: the interactive presentation

Build from the current checkout (not in the installed release yet). Run this in Ghostty for
pixel overlays, or any terminal for the cell fallback:

```sh
./scripts/demo.sh
# Equivalent:
cargo run -p caretline-cli -- demo showcase
```

The executable script also works from another directory when invoked by its absolute path;
it finds the checkout itself. No agent, credentials or network service is needed. The first
run compiles the binary; after that the command starts immediately.

Twelve timed slides introduce Caretline and demonstrate **real engine operations**, not
pre-rendered screenshots:

1. **Meet Caretline** — an embeddable, pure editor whose whole state is a value.
2. **Live input** — timed message chunks, wrapping and Unicode.
3. **Many carets** — three simultaneous edits, undo and redo with all carets restored.
4. **Overlay choreography** — multiple rings, two callouts, arrows and a spotlight with
   holes for every active annotation. Placement preserves the text it explains.
5. **Shared views** — two panes, one document, independent carets, a live external update.
6. **Safe co-editing** — an actual stale revision refusal, a guarded retry, and undo that
   keeps the remote text while removing the local edit.
7. **Structure** — fold, unfold and move a subtree while verifying its stable mark.
8. **Proof** — serialize/load the editor and replay its trace, comparing the entire state.
9. **Typography** — the site's real Geist Mono and Geist outlines, each at three sizes.
   These are pixel-rendered samples; the terminal's configured font is not changed.
10. **ASCII art** — the existing donut, cube, tunnel and plasma renderers, pushed as real
    editor frames with selected highlights.
11. **Fractional-pixel motion** — an anti-aliased Bezier path and moving caret within the
    cell grid. This is fractional device-pixel geometry, not RGB/LCD font subpixel hinting.
12. **The logo finale** — the original warp field, spaced selected wordmark and blinking
    caret. It keeps animating until you pause or quit.

By default the presentation advances every **14 seconds**, then stays on the animated logo
finale. Use `--seconds 8` for a faster presentation, or `--reduced-motion` to freeze continuous
ASCII and pixel animation while retaining timed slide advance. Prefer **100×30** for the full layout; 80×24 also
works, and smaller windows use compact overlays.

| Key | Action |
|---|---|
| `←` / `→`, `1`–`9` | Jump to a slide; its script runs, then holds (manual mode) |
| `Space` | Pause/resume the current animation |
| `a` | Toggle automatic slide advance and resume playback |
| `r` | Replay the current slide from its initial state |
| `Home` | Restart the whole presentation and clear its check results |
| `End` / `0` | Jump to the logo finale |
| `↑` / `↓`, `PgUp` / `PgDn` | Scroll the slide; its overlays follow their anchors |
| `s`, `p`, `t` | Spotlight, pixels/cells, inline/file pixel transport |
| `F1` / `Alt-?`, then `Esc` | Open/close the keys overlay |
| `q`, `Esc`, `Ctrl-C` | Quit (Esc closes help first if it is open) |

The overlay design follows the public [Caretline brand](https://caretline.app/brand/):
starlight on the void, neutral hairlines, nearly square corners, no panel shadows or ring
glow, and one selection colour. The pixel typography uses bundled, SIL-OFL-licensed ASCII
outlines of the site's Geist faces; no fonts are installed or fetched at runtime. Other
terminals keep the same lesson text but cannot display mixed font sizes in ordinary cells.

The socket stays live throughout. In another terminal, using the same source-built binary:

```sh
cargo run -p caretline-cli -- send --latest subscribe state
cargo run -p caretline-cli -- send --latest state.get
```

To push your own text, navigate to slide 2, let it start typing, press Space, then run:

```sh
cargo run -p caretline-cli -- send --latest --view 0 keys ' + from another terminal'
```

Navigation/replay replaces the slide's document and closes secondary views. A subscriber
sees those state changes as well as the timed edits. The protocol renders the underlying
editor; the CLI's presentation chrome and graphics are not part of socket render responses.
The remote actor in slide 6 is a local script using the real protocol, not a live AI model.
Terminal pastes are consumed so accidental dictation or a paste cannot corrupt the deck.
Socket edits are deliberately still allowed. Continuous frame history is bounded while the
finale runs.
The checks report actual outcomes; outside edits may intentionally change those outcomes.

For repeatable validation or snapshots without a terminal:

```sh
./scripts/demo.sh --headless                              # JSON: 13 invariant checks
./scripts/demo.sh --snapshot 100x30 --keys '4<wait:7000>'   # multiple overlays
./scripts/demo.sh --snapshot 100x30 --keys '<wait:168000>' # the full timed run, no sleeping
```

### `caretline demo`: the tour

A short document you work down with `↓`. The status bar names the keys for the section the
caret is in.

| Section | You try |
|---|---|
| 1. Type | Type past the edge of the window: lines wrap at word boundaries |
| 2. Move | `⌥←`/`⌥→` by word; `↑`/`↓` by visual row, keeping the goal column across short and wrapped rows |
| 3. Select | `⇧` with any arrow selects, `⇧⌥` by word; `←`/`→` collapse to the selection's edge, `Esc` collapses it |
| 4. Multiple carets | `⌃N` adds a caret on the row below, in the same column; typing edits every row at once; `Esc` goes back to one |
| 5. Undo, exactly | `⌃Z` and `⌃Y` bring back the text, every caret and the selection exactly |
| 6. Indent and move | `Tab`/`⇧Tab` indent and outdent a line with the lines under it; `⌥↑`/`⌥↓` move it past its neighbours |
| 7. Folds | `⌃O` folds the lines under the caret's item, and opens them again |
| 8. Marks | The status bar shows the caret's block mark, an id that survives moves, cut and paste, and undo |
| 9. A second view | `⌃G` opens a second view of the same document below: its own caret and scroll |
| 10. One state | `⌃D` writes the whole editor (text, carets, undo history, marks, folds, scroll, clock) to `state.json` and says how big it is |
| 11. Replay | `⌃P` replays your session, from the first state through every message, and checks it lands on the identical state |

`⌃Q` quits (twice to leave without saving).

`⌃N`, `⌃O`, `⌃G`, `⌃D` and `⌃P` are the demo's own keys. In your own program the same things
are a selection in the [state](architecture.md), a [`toggle_fold`](messages.md#views-and-folds)
message, [`view.open`](protocol.md#views), `State::to_json` and [`replay_trace`](api.md).

### `caretline demo scenes`: the frame path

Six ASCII animations (a warp field around the CARETLINE logo, a shaded donut, a wireframe
cube, an XOR tunnel, plasma and fire) play inside the real editor. A client on the editor's
own socket sends each frame as one [`frame`](protocol.md#frames) request: a whole new
document, its brightest cells sent as selections. `←` and `→` change the scene, `q` quits.

With `--bench`, it plays them into an editor that is already running instead and reports the
frame rates achieved (see [Performance](performance.md)):

```sh
caretline draft.md --listen          # in one terminal
caretline demo scenes --bench        # in another
```

### `caretline demo agent`: co-editing

The editor starts with [`--listen`](protocol.md), and a scripted agent connects to its socket
like any other client. It opens a [view](protocol.md#views) of its own, shown as the pane
under yours, with its own caret, and types into its own section while you type in yours:

- **Every keystroke is a guarded write.** The agent reads the revision, then writes with
  [`if_rev`](protocol.md#revisions). If you typed in between, the editor refuses the write as
  `stale` and the agent reads again. Its status line counts the refusals.
- **Undo is yours.** The agent's text arrives as [changes from
  elsewhere](messages.md#changes-from-elsewhere), outside your undo history, so `⌃Z` takes back
  only what you typed.
- **The status bar says who is editing**, and when the agent is done, `⌃P` replays the whole
  session, both of you, to the identical state.

While it runs, the editor is also reachable from another shell:

```sh
caretline send --latest keys '<d-down>hello from another shell'
```

### `caretline demo layers`: visible overlays

Not released yet: run the current checkout rather than the installed release:

```sh
cargo run -p caretline-cli -- demo layers
```

The tour's text becomes a read-only backdrop for a hint box, an arrow, a ring round a word
and a spotlight. In Ghostty or kitty these are pixel graphics; other terminals fall back to
cells. Use a window of at least **80 columns × 24 rows** to see the full box and controls.

Try this short walkthrough:

1. The **Jump by word** hint is visible immediately, pointing at `word` in the Move section.
2. Press `s`: the spotlight turns off, then on again. The status bar shows `spot:off/on`.
3. Press `↓` a few times: the box, arrow and ring follow the word. Scroll it off screen to
   see an edge chip, then `↑` to bring it back.
4. In Ghostty, press `p` to compare pixels with the cell fallback, then `p` again. Press `t`
   to compare inline and temporary-file pixel transport; diagnostics follow the controls.
5. Press `F1` (or `Alt-?`) for the keys overlay, then `Esc` to return. Press `q` to quit.

For a repeatable preview without a terminal:

```sh
cargo run -p caretline-cli -- demo layers --snapshot 80x24
cargo run -p caretline-cli -- demo layers --snapshot 80x24 --keys '<down><down>s'
```

The status bar reports `pixels` or `cells`. Inside tmux or screen it normally uses cells;
run directly in Ghostty for pixels. See [Layers over the tour](cli.md#layers-over-the-tour)
for the probe, overrides and all controls.

## Then

```sh
caretline draft.md               # edit a file
caretline --outline list.md      # lists, blocks and folds
caretline keys                   # every key and its command (F1 in the editor)
caretline --new-state draft.md --size 60x20 > s.json
caretline --state s.json --keys 'Hello<cr>' --snapshot 60x20   # headless
caretline draft.md --trace t.jsonl                             # record a session…
caretline --replay t.jsonl --snapshot 80x24                    # …and replay it
```

- [The caretline command](cli.md): the editor and the headless tools, end to end.
- [Architecture](architecture.md): the one `State`, `Msg`, `update` and `view`.
- [State protocol](protocol.md) and [MCP server](mcp.md): drive a live editor from a
  script or an agent.
- [Embedding](embedding.md): put the engine in your own program with `cargo add caretline`.

## Releases

Each `v*` tag on [brancusi/caretline](https://github.com/brancusi/caretline) builds the binary for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (static) and publishes it on the
[releases page](https://github.com/brancusi/caretline/releases), as
`caretline-<version>-<target>.tar.gz` with a `.sha256` beside it. The installer takes
`CARETLINE_INSTALL_DIR` (default `~/.local/bin`), `CARETLINE_VERSION` (default the newest) and
`CARETLINE_NO_MODIFY_PATH=1`. The macOS builds aren't signed: a binary fetched with `curl`
isn't quarantined, so Gatekeeper doesn't stop it.
