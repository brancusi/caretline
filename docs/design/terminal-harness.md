# Design: the terminal harness

Status: in progress. Step 0 shipped in caretline 0.5 (`Role::Caret`, the fuzz suite's screen
invariant EI13, `caretline doctor`). Steps 1 (one decoder), 2 (keyboard model) and 3 (headless
screen) are done. Steps 4 and 5 are next.

## Why

The engine's tests check that the **state** is right: goldens, fuzz, outline fuzz, replay. Each
bug found by hand in Ghostty sat outside them, at one of the two edges where caretline meets
the terminal:

```
physical key ─▶ terminal bindings and encoding ─▶ bytes ─▶ decoder ─▶ Key ─▶ keymap ─▶ Msg
   ─▶ update ─▶ state ─▶ view ─▶ Frame ─▶ styles and diff ─▶ bytes ─▶ terminal screen
```

- **The input edge.** Ghostty's default `super+a=select_all` took Cmd-A before caretline saw
  it, so the whole terminal screen was selected. Ghostty also rewrites keys (Cmd-Left is sent
  as `^A`, Alt-Left as `ESC b`).
- **The output edge.** The frame placed only the primary caret, so other carets were in the
  state and invisible.

The goal: fast, in-process tests (thousands of cases a second, no Ghostty window, no PTY)
that drive a key from the keyboard to the pixels-as-cells a terminal shows, and assert that
what the person **sees** matches the state. Then fuzz that path, so a class of bug is caught
once rather than a test written per bug.

We don't reimplement editing: Helix (vendored) does the text, and those tests stay. This is
the terminal layer Helix's own renderer covered and caretline doesn't use.

## What exists today

| Piece | Where | State |
|---|---|---|
| Key notation, keymap as data | `caretline/src/keymap.rs`, `commands.rs` (`default_keymap`, `binding_key`, `key_notation`, `command_for`) | Pure, tested |
| Engine goldens and fuzz, now with EI13 (screen shows the selection) | `caretline/tests/fuzz.rs` | Checks the `Frame`, not terminal bytes |
| Byte parser for keys (legacy and kitty `CSI … u`), mouse, paste, replies | `caretline-cli/src/rawin.rs` (`Parser`) | Pure; the **only** decoder (step 1), tested as a `bytes → Event → Key` table |
| `to_key`, `terminal_msgs` | `runtime.rs` | Pure (`terminal_msgs` takes the clipboard read as an argument) |
| `draw` | `runtime.rs` | ratatui over crossterm: roles to styles, diff, synchronized update |
| A toy screen model | `caretline-cli/tests/common/mod.rs` (`screen`) | Cursor moves, clears, text; no styles, no cursor, no wide chars |
| Ghostty bindings parser | `caretline-cli/src/doctor.rs` (`parse_ghostty`, `conflicts`) | Pure, unit tested |
| PTY tests | `caretline-cli/tests/live.rs`, `demo.rs` | Real processes, slow, timing-bound |

## The plan

### Step 1. One decoder (prerequisite)

Two decoders read the same bytes: crossterm's in the editor, `rawin::Parser` in the demos.
Under "instantiate, don't fork", the editor should use `rawin::Parser` always (the probe path
already does), and `Parser` becomes the one tested decoder.

- Make `rawin::Parser` the only input path in `run_interactive`. Delete the crossterm
  `event::read` branch.
- Give `Parser` the kitty keyboard protocol's `CSI … u` forms with all modifier bits (super
  = 8, plus hyper/meta if a terminal sends them) and the legacy forms Ghostty sends without
  it. Check against crossterm's decoder over a corpus first, so the switch changes no
  behaviour, then remove crossterm's.
- Split `terminal_msgs` so the clipboard read is an input (passed in), and the function is
  pure: `(state, Event, clipboard) -> Vec<Msg>`.

Done when the editor has one decoder, `cargo test` covers it as a table of `bytes → Event →
Key`, and the PTY tests still pass.

**Done.** One startup probe asks for the keyboard protocol (`CSI ? u`), and for pixels when a
demo wants them, fenced by DA1; the reader is always `rawin::spawn`. Before the switch the
parser ran beside a copy of crossterm 0.29's parser over a generated corpus of about 8,600
sequences (every control byte and Alt pair, xterm modifier forms 1 to 16 with event types,
`CSI n ~` keys, kitty keys with modifiers, kinds and alternates, the keypad, mouse reports,
pastes): it agreed on all but a few, kept on purpose and listed in the table test
(`rawin::tests::bytes_to_keys`): Alt-`[` and Alt-Shift-O at the end of a read and a double
ESC (crossterm drops one), `CSI 1;m R` as F3 (crossterm reads a cursor report), and pastes
with `\r\n` made `\n`. A run in Ghostty, keys sent with its AppleScript `send key`, decoded
Alt-Left (`ESC b`), Alt-Shift-Left (`CSI 1;4D`), Esc (`CSI 27u`, no timeout) and Alt-Backspace
(`CSI 127;3u`). Ghostty's `send key` sends no bytes for letter keys, so the harness (step 4),
not AppleScript, is the way to test those.

### Step 2. The keyboard model: a simulated Ghostty input

A pure module (test support in `caretline-cli`, or a small `caretline-term` dev crate if the
MCP server's tests want it too):

```rust
struct TermKeyboard { bindings: Vec<TermBinding>, kitty_flags: u8 }
enum Sent { Bytes(Vec<u8>), Taken(String /* the terminal's action */) }
fn press(&self, key: Key) -> Sent
```

- `bindings` come from `doctor::parse_ghostty`. The default table is a checked-in fixture
  generated by `ghostty +list-keybinds --default` (with the version in its header), plus a
  user config on top. Performable actions are modelled as `doctor::PASSES` does: Ghostty
  takes the key only when it can act.
- Encoding: kitty protocol (`CSI codepoint;mods u`, the DISAMBIGUATE flag caretline pushes)
  and legacy xterm (`CSI 1;mods D`, `ESC` prefix for Alt, control bytes for Ctrl).
  `text:`/`esc:`/`csi:` actions produce their bytes.
- Move `doctor`'s parsing into this module, so the doctor and the tests share it.

Tests that become possible:
- **Reachability**: for every default binding, `press` it on the default Ghostty, decode with
  `Parser`, map with the keymap, and assert either it runs its command or it's in an
  explicit, reviewed list of keys Ghostty keeps. A new Ghostty default that steals a key
  fails a test.
- **Round trip**: for every `Key` the keymap names, kitty-encode → `Parser` → `to_key` gives
  the same `Key`. Legacy encoding gives the same key or a documented lossy one.

**Done.** `caretline-cli/src/keyboard/` holds `TermKeyboard` (`press(key) -> Sent::Bytes |
Paste | Taken`), the Ghostty 1.3.1 defaults fixture, and a table of what Ghostty 1.3.1 really
sent for 180 key presses (Enter, Tab, Backspace, Delete, Esc, arrows, Home, End, PageUp under
nine modifier sets, with and without the keyboard protocol), measured with AppleScript's
`send key` into a raw reader. The model reproduces every row (`matches_what_ghostty_sent`).
The measurement found that legacy Ghostty sends modified Enter, Tab and Esc as xterm's
`CSI 27;m;code ~`, which the decoder dropped; it reads them now. Tests:
`kitty_round_trips_every_key`, `legacy_round_trips_or_is_a_known_loss` (the losses listed),
`every_binding_reaches_its_command` (default Ghostty keeps a reviewed list; with doctor's fixes
all reach but Cmd-Q). `doctor::conflicts` is now this model, so the doctor and the tests can't
disagree. AppleScript sends no bytes for character keys, so letters follow the kitty spec
unmeasured.

Answered: Ghostty keeps Cmd-Up/Down (and Cmd-Shift-Up/Down) for `jump_to_prompt` even in an
alternate-screen app (measured: no bytes).

### Step 3. The screen model: a headless terminal emulator

Feed the runtime's real output bytes into an emulator and read back cells, styles and the
cursor.

- Use the `vt100` crate (pure Rust, cells with attrs, cursor position and visibility,
  alternate screen, wide chars) behind a small trait (`Screen::feed(&[u8])`,
  `cell(x, y) -> {symbol, fg, bg, reverse, underline}`, `cursor() -> Option<(x, y)>`), so it
  could later be swapped for libghostty-vt (Ghostty's own parser, a C API built with Zig) if
  we need Ghostty's exact behaviour. Check first whether libghostty-vt is easy enough to
  build in CI to use from the start. vt100 is the low-risk default.
- Make `draw` testable without a TTY: build it on a `ratatui::Terminal` over a
  `CrosstermBackend<Vec<u8>>` (or any `Write`), so the test captures the exact bytes the
  editor would write, diffs between frames included.
- Replace `tests/common::screen` with the emulator.

**Done.** `caretline-cli/src/screen.rs` (test only): a `Screen` trait (`feed`, `cell`,
`cursor`, `row`, `text`) over `vt100`, and `Painter`, the runtime's own `draw` (now generic
over its writer) painting into memory, so a test gets the exact bytes, diffs included. Tests:
`the_screen_shows_the_frame` (text, colours and reverse of every cell and the cursor match the
frame, across a selection over a line break, a wide grapheme, and a second diffed paint) and
`other_carets_are_reverse_cells`. The PTY tests' screen is `vt100` too.

libghostty-vt has a Rust wrapper (`ghostty-vt`), but it builds Ghostty's source with Zig and
needs Rust 1.96: every contributor and CI would carry that toolchain. `vt100` it is, behind the
trait, until a test needs Ghostty's exact behaviour.

### Step 4. The harness: key to screen, in process

```rust
let mut h = Harness::new("hello ▮world", Terminal::ghostty_default(), 40, 6);
h.press("<d-a>");          // through TermKeyboard → bytes → Parser → keymap → update → draw → emulator
h.type_text("x");
assert_eq!(h.screen_text(), "...");
h.assert_screen_matches_state();   // the screen-truth oracle
```

- One `Harness` drives the same functions the runtime calls (`Parser`, `terminal_msgs`,
  `update`, `compose`, `draw`), with no thread, clock or TTY. Time arrives as messages, as in
  the engine.
- **The screen-truth oracle** (EI13 lifted from `Frame` to the emulated screen): every caret is
  the terminal cursor (primary) or a reverse cell (others); every selected char has the
  selection's colours; nothing else does; the text in each row equals the frame's text;
  the cursor is visible when focused. Checked after every step.
- Clipboard: a fake in the harness (copy and paste go through it), so copy → paste round trips
  are testable end to end, OSC 52 included.
- Mouse: `TermMouse` encodes clicks and drags as SGR reports, through the same path.

### Step 5. Fuzz, and a corpus of the bugs seen by hand

- A property test over the harness: random documents, sizes, and random sequences of
  **physical** key presses (weighted toward editing: selection, word motion, multi-caret
  add, typing, cut/copy/paste, undo), on both a default and a "fixed" Ghostty config, with
  the oracle after every step. Seeded, `CARETLINE_HARNESS_SEEDS` for longer runs, failures
  print a key script that replays the case.
- Shrinking: on failure, drop keys while it still fails, then print the minimal script.
- Goldens in `caretline-cli/tests/goldens/harness/`: each bug found by hand becomes one script
  plus the expected screen (`text` and `ansi` snapshots, as `--snapshot` prints them). Start
  with: Cmd-A on default Ghostty (taken), Cmd-A unbound (selects all), three carets visible,
  a multi-line selection's colours, wide graphemes under a caret.

### Later, not in this plan

- Other terminals as data (kitty, WezTerm, iTerm2 binding tables), once Ghostty works.
- A pixel step: render the emulated screen to PNG for visual goldens (the headless-screens
  approach), only if cell-level checks miss something.
- A startup warning in the editor when `doctor` would find conflicts. Stay with
  `caretline doctor` until the harness shows the conflict list is reliable.

## Order and size

| Step | Size | Depends on |
|---|---|---|
| 1 One decoder | 1–2 days | — |
| 2 Keyboard model | 1–2 days | 1 |
| 3 Emulator and capturable `draw` | 1 day | — (parallel with 1–2) |
| 4 Harness and oracle | 2 days | 1, 2, 3 |
| 5 Fuzz, shrinking, goldens | 1–2 days | 4 |

Each step is its own PR with its CHANGELOG line, and the docs-and-site pass after each merge.
`docs/testing.md` gets a row per new test file.

## Open questions

- Does Ghostty take Cmd-Up/Down (`jump_to_prompt`) and Cmd-Z (`undo`) inside an alternate-
  screen app, or are they performable in practice? `+list-keybinds` doesn't show the flag.
  Check once by hand; record the answer in the keyboard model's fixture.
- vt100 vs libghostty-vt: decide in step 3 on build cost.
- ~~Does `rawin::Parser` need a timeout for a lone `ESC`?~~ It has one: the reader flushes
  an unfinished escape after 30 ms without bytes. With the keyboard protocol on, Esc is
  `CSI 27 u` and never waits.
