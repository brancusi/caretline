# Testing

Because `update` and `view` are pure, a caretline test is just data: a starting state, some
keys or messages, and the expected result. No terminal, no timing, no mocks.

## Where the tests live

| File | What it checks |
|---|---|
| [`caretline/tests/goldens.rs`](../crates/caretline/tests/goldens.rs) | Behaviour goldens: text and selection before, keys, text and selection after (and frames where it matters). The baseline is a macOS text field |
| [`caretline/tests/rehydrate.rs`](../crates/caretline/tests/rehydrate.rs) | A state survives JSON exactly; a recorded session replays to the live result |
| [`caretline/tests/fuzz.rs`](../crates/caretline/tests/fuzz.rs) | Property tests: seeded random documents and messages, with invariants checked after every step |
| [`caretline/tests/marks.rs`](../crates/caretline/tests/marks.rs) | Block marks: each mapping rule, undo and redo restoring marks exactly, cut and paste keeping ids, serialization; random sessions with marks at line starts and unique ids after every step, every undo restoring the marks |
| [`caretline/tests/outline.rs`](../crates/caretline/tests/outline.rs) | Block goldens in block notation (` ‖ ` blocks with a blank row between, ` ¦ ` without, `⏎` a soft break), with block ids: Enter, Backspace and Delete at block edges, Tab, tags, moves, atomic images, copy and paste, blank rows, host messages and effects |
| [`caretline/tests/host.rs`](../crates/caretline/tests/host.rs) | The extension points with a made-up host: commands (undo, traces, replay with the host, the protocol), input rules, mark payloads through cut, paste, undo and JSON, decorations and hit-testing, tags |
| [`caretline/tests/hooks.rs`](../crates/caretline/tests/hooks.rs) | The generic hooks with a made-up host (a counter and a pin that follows the text): view values through JSON, the protocol and replay; `Msg::Ext` and observers (passive, read-only views, the changes `update_with_changes` returns, every view holding a key); `cell_px` through `resize`, old traces included; frame passes and the `Frame` writers around wide graphemes; `locate` as the inverse of `hit`, with folds; catalog entries and host ops over the protocol |
| [`caretline/tests/layout.rs`](../crates/caretline/tests/layout.rs) | The outline layout (markers in a hang, columns per depth, virtual rows, folds, row info, hit-testing) and prose rows at the column's edge: none wider than the column, wide graphemes included |
| [`caretline/tests/commands.rs`](../crates/caretline/tests/commands.rs) | The command catalog and the keymap as data: ids, messages, bindings, agreement with the hand-written keymap it replaced, the protocol ops |
| [`caretline/tests/serde_features.rs`](../crates/caretline/tests/serde_features.rs) | The engine's JSON under serde_json features a host may turn on: the engine's manifest enables neither `preserve_order` nor `arbitrary_precision`, responses keep their key order, marks, payloads, clipboards, messages, protocol requests and traces round-trip |
| [`caretline/tests/single_line.rs`](../crates/caretline/tests/single_line.rs) | One-line documents: Enter, flattened typing, paste, CRLF, host edits and changes from elsewhere, undo across a flattening paste, loading a state with line breaks, Up and Down, sideways scrolling, several carets, the protocol, and random sessions that never break the line |
| [`caretline/tests/scope.rs`](../crates/caretline/tests/scope.rs) | No host concept (task, status vocabulary, journal…) named in the engine's sources |
| [`caretline/tests/docs.rs`](../crates/caretline/tests/docs.rs) | The examples in these docs run |
| [`caretline/tests/outline_fuzz.rs`](../crates/caretline/tests/outline_fuzz.rs) | Outline properties over random outlines and messages: blocks and marks agree, ids unique, no caret in a marker or an image, undo and redo exact (marks included), kind changes move no other block, host commands (a made-up retagging host) included, cut and paste in place keeps ids, Markdown files round-trip. `CARETLINE_OUTLINE_SEEDS` runs more seeds |
| [`caretline/tests/keymap.rs`](../crates/caretline/tests/keymap.rs) | Key bindings and the key-script parser |
| [`caretline/tests/common/mod.rs`](../crates/caretline/tests/common/mod.rs) | Shared helpers: caret notation, `golden`, `keys`, `send`, `frame`, random generators |
| [`caretline-layers/tests/conformance.rs`](../crates/caretline-layers/tests/conformance.rs) | The shared host conformance kit: placement invariants, deliberately broken plans, determinism, JSON, replay, mapping, protocol contract, stable snapshots and inspector agreement (feature `conformance`) |
| [`caretline-layers/tests/head.rs`](../crates/caretline-layers/tests/head.rs) | Row-bound arrow heads, clear-cell routing and stable reasons when no head fits |
| [`caretline-tour/tests/conformance.rs`](../crates/caretline-tour/tests/conformance.rs) | Every example walkthrough step checked on the host's scenes at several sizes (feature `caretline-layers/conformance`) |
| `caretline/src/helix/**` | Helix's own unit tests, vendored with the code |
| [`caretline-cli/tests/cli.rs`](../crates/caretline-cli/tests/cli.rs) | The binary: fixtures render to their saved snapshots, traces replay, `--keys` and `--dump-state` round-trip, effects are reported and never performed |
| [`caretline-cli/tests/demo.rs`](../crates/caretline-cli/tests/demo.rs) | Demo frames against `tests/goldens`, live PTY controls, socket subscriptions and command-line edits. The showcase's `--headless` report verifies all 13 invariants |
| [`caretline-cli/tests/common/mod.rs`](../crates/caretline-cli/tests/common/mod.rs) | Shared by the binary's and the MCP server's tests: child processes and scratch directories that clean up after themselves, a pseudo-terminal, deadlines (see [Tests that start processes](#tests-that-start-processes)) |

Run them:

```sh
cargo test -p caretline
cargo test -p caretline-cli
```

Cargo turns a dependency's features on for a whole build, so a host that enables serde_json's
`preserve_order` (insertion-ordered objects) or `arbitrary_precision` (exact numbers) enables
it in the engine too. The engine enables neither itself (it would change JSON for every crate
in the host's build), and its output must not depend on them: protocol responses are structs,
whose keys come out in declaration order, never `json!` maps. CI runs the engine's tests with
neither and with each:

```sh
cargo test -p caretline
cargo test -p caretline --features serde_json/preserve_order
cargo test -p caretline --features serde_json/arbitrary_precision
```

`cargo test -p caretline` on its own runs with neither; `cargo test --workspace` runs it with
`preserve_order`, which the binaries enable for their own output.

## Host layers integration tests

The layers crate's optional `conformance` feature gives every host the same checks over its
own screens. Build `conformance::Scene` values using your actual renderer measurements and
anchor resolver, then run `check_sizes` at wide, normal and narrow sizes. Add replay,
anchor-mapping and protocol contract checks and commit snapshots of representative screens.
The [integration guide](layers.md#testing-your-integration) describes the helpers;
[the inspector](layers.md#inspect-a-placement) explains placement failures.

CI runs the kit against the layers fixtures, ratatui example, CLI layers demo and walkthrough
example on both Linux and macOS:

```sh
cargo test --workspace --features caretline-layers/conformance --locked
```

The default workspace tests do not enable the kit; keep this feature-enabled run in a host's
CI too. Narrow strips, off-screen chips and omitted arrows can be valid fallbacks: the
report lists them separately from invariant violations so your goldens can document what
people see.

## Tests that start processes

The binary's tests and the MCP server's start real processes: editors on a pseudo-terminal,
`caretline serve` on a socket, `caretline-mcp`. Through the helpers in
[`caretline-cli/tests/common`](../crates/caretline-cli/tests/common/mod.rs), none outlives its
test:

- A `Proc`, a `Pty` and an `Mcp` kill and reap their child when dropped, and a `Scratch`
  directory is removed, so a failed assertion cleans up as a pass does.
- `Drop` doesn't run when the test process is killed, so each child is also tied to it: an
  editor on a pseudo-terminal gets SIGHUP when the test's end closes, `caretline-mcp` and
  `serve` on stdio see their stdin end, and `serve --socket` runs with `--exit-with-parent`.
  The scratch directories of test processes that are gone are swept at the next run.
- A child's stdout and stderr go to pipes the test owns, never the test's own, so nothing
  left behind can hold a `cargo test | …` pipe open. Its stderr is printed if the test fails.

Waits are for output, not on a clock: a pseudo-terminal wakes the waiting test as output
arrives, and requests wait for their response. Each wait still has a deadline, so a hang
fails with the screen it got to. Deadlines are nominal times multiplied by
`CARETLINE_TEST_TIMEOUT_SCALE` (default 6, so a nominal 10 s is a minute), which leaves room
for a loaded machine; set it to 1 to find a slow wait, or higher on a slow CI runner.

The showcase can also validate itself directly, without a terminal or real-time waits:

```sh
cargo run -p caretline-cli -- demo showcase --headless
cargo run -p caretline-cli -- demo showcase --snapshot 100x30 --keys '4<wait:7000>'
```

Its unit tests compare frequent and delayed polling, verify pause/navigation clocks,
check text-preserving overlay placement, rapid pixel cleanup, live paste suppression,
bounded/paused finale frames, distinct bundled font outlines and small windows. The CLI
palette is checked against the site's design tokens, and pixel panels/arrows/rings have PNG
goldens.

## Caret notation

Goldens write the text and the selection as one string:

| Notation | Means |
|---|---|
| `▮` | The caret (the selection's head) |
| `⟦abc▮⟧` | `abc` selected left to right: the anchor before `a`, the caret after `c` |
| `⟦▮abc⟧` | `abc` selected right to left: the caret before `a` |

`state("Hello ⟦wor▮⟧ld")` builds a state at 80x24 from notation; `state_wh(…, w, h)` picks the
size. `show(&state)` prints the primary selection back in notation.

## Goldens

A golden is a before, a key script and an after:

```rust
#[test]
fn e01_left_collapses_to_start() {
    golden("Hello ⟦wor▮⟧ld", "<left>", "Hello ▮world");
}
```

When a case needs more than text and selection, drive the state and assert on what you need:
effects, the clipboard, the history, the frame or the caret's cell.

```rust
#[test]
fn e37_cut_is_one_undo_step() {
    let mut s = state("Hello ⟦wor▮⟧ld");
    let fx = keys(&mut s, "<c-x>");
    assert_eq!(fx, vec![Effect::ClipboardSet { text: "wor".into() }]);
    assert_eq!(show(&s), "Hello ▮ld");
    keys(&mut s, "<c-z>");
    assert_eq!(show(&s), "Hello ⟦wor▮⟧ld");
}
```

| Helper | Does |
|---|---|
| `golden(before, script, after)` | Builds, runs the keys, compares the notation |
| `keys(&mut state, script)` | Runs a key script through the keymap; returns the effects |
| `send(&mut state, msgs)` | Sends messages; returns the effects |
| `frame(&state)` | The rendered frame as text |
| `cursor(&state)` | The caret's screen cell |

Use `<wait:MS>` in a script when undo grouping matters: `"one<wait:2000> two<c-z>"` undoes
only `" two"`.

## Rehydration and replay tests

`rehydrate.rs` checks two promises.

- **A state survives JSON.** `round_trip` asserts that `from_json(to_json(s))` equals `s`,
  renders the same frame and serializes to the same JSON. It covers fresh and edited states,
  the undo history, the goal column, an open typing run, and hand-broken states that load
  repaired.
- **A trace replays exactly.** A `Recorder` stands in for the interactive runtime: it writes
  the initial state and every message (including the `saved` its effects produce) to a trace.
  `replay_trace` must then give the identical state and frame, for scripted sessions and for
  random ones.

## The property fuzzer

`fuzz.rs` runs 24 seeds of 500 random messages each (`SEEDS`, `STEPS`). Documents mix ASCII,
tabs, CRLF, emoji with skin tones, ZWJ families, flags, combining accents, CJK, zero-width and
control characters. Viewports range from 1x1 to 200 columns. Now and then a step starts from a
random multi-range selection.

After **every** step it checks:

| Invariant | |
|---|---|
| Positions | Every anchor and head is within the text and on a grapheme boundary |
| EI1 | A motion without Shift leaves no selection |
| EI2 | A motion with Shift never moves the anchor |
| EI4 | Copy changes nothing but the clipboard |
| EI5 | An edit that made a new revision undoes to exactly the state before it and redoes to exactly the state after |
| EI6, EI7 | Typing over a selection replaces exactly it; a delete with a selection removes exactly it |
| Effects | Effects are plain values that match the message |
| `view` | Never panics, at the state's size and at extreme sizes |
| Serialization | At random points, the state round-trips to the same state and frame |
| Undo all | Undoing everything gives back the original text |

Separate tests check EI3 and EI8 (cut-then-paste and copy-then-paste in place are
identities) and EI12 (select all, delete, one undo restores everything).

A failure names its seed and step (`seed 7 step 312: …`). The generator is seeded, so the same
seed fails the same way every time. To focus on it, temporarily run only that seed in
`random_sessions_keep_every_invariant`.

## Snapshot fixtures

[`crates/caretline-cli/fixtures`](../crates/caretline-cli/fixtures) holds
`NAME.state.json` files with `NAME.snapshot.txt` and `NAME.snapshot.ansi` beside them.
`fixtures_render_their_snapshots` finds every `*.state.json`, renders it at its own
viewport in both formats, and compares. Adding a fixture needs no code:

```sh
cd crates/caretline-cli/fixtures
printf 'First line\nA second line that is long enough to wrap.\n' > my-case.md
caretline --new-state my-case.md --size 30x6 > my-case.state.json && rm my-case.md
caretline --state my-case.state.json --keys '<down><s-end>' --dump-state my-case.state.json
caretline --state my-case.state.json --snapshot 30x6 > my-case.snapshot.txt
caretline --state my-case.state.json --snapshot 30x6 --format ansi > my-case.snapshot.ansi
```

The snapshot size must be the state's viewport, because the test renders each fixture at its
own viewport.

`--new-state` records the path you give it, so run it from the fixtures directory (or edit
`path` afterwards) to keep machine paths out of the fixture. Review the snapshot by eye
before committing it: from then on, the test holds the engine to it.

## From a recorded trace to a test

When something goes wrong in a real session, record it and turn it into a test.

**1. Record it.**

```sh
caretline draft.md --trace bug.jsonl
```

**2. Check that it replays.** The replay is exact, so the bug shows up in the final frame or
state.

```sh
caretline --replay bug.jsonl --snapshot 80x24
caretline --replay bug.jsonl --dump-state -
```

**3. Split it** into a starting state and a message file. Then you can trim messages until
only the ones that matter remain:

```sh
head -1 bug.jsonl | jq .state > bug.state.json
tail -n +2 bug.jsonl | jq -c .msg > bug.msgs.jsonl
caretline --state bug.state.json --msgs bug.msgs.jsonl --snapshot 80x24   # same frame as the replay
```

`--msgs` and `--replay` agree as long as the trace has a single `state` line. A trace that
was appended to by several sessions has one `state` line per session; split at the last one.

**4. Write the test.** Pick one:

- **A golden**, when the bug is about text and selection. Turn the state's text and selection
  into notation, and the messages into a key script (or use `send` with the messages).
- **A fixture**, when the bug is about the frame. Save the trimmed state with
  `--dump-state` as a fixture, as above.
- **A trace test**, when the exact sequence matters (timing, effects, resizes). Commit the
  trimmed trace and assert on its replay:

```rust
use caretline::trace::replay_trace;
use caretline::view;

#[test]
fn recorded_session_replays_to_the_saved_frame() {
    let trace = include_str!("../../caretline-cli/fixtures/session.trace.jsonl");
    let (state, _) = replay_trace(trace).unwrap();
    // The trace ends with a resize to 36x8, the snapshot's size.
    assert_eq!(
        view(&state).to_text(),
        include_str!("../../caretline-cli/fixtures/session.snapshot.txt")
    );
}
```

Run it before the fix to see it fail, then fix the engine and keep the test.
