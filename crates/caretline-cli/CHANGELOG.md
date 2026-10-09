# Changelog

## Unreleased

### Added

- `caretline sim TEXT KEYS`: runs a key script through a simulated Ghostty (its 1.3.1 default
  bindings, or yours with `--mine`, with or without the keyboard protocol), the editor and a
  terminal emulator, in process, and prints what each key did (kept by the terminal, or the
  bytes it sent and the messages they became), the selection after it, and the screen it
  left, checked against the editor's state after every key. The same harness runs as tests,
  in milliseconds, for the bugs found by hand (Cmd-A kept by Ghostty, carets that didn't
  show).

  Try it: `curl -fsSL https://caretline.app/install.sh | sh -s -- --ref main -- sim 'hello ▮world' '<d-a>x' --config 'keybind = super+a=unbind'`

- `caretline doctor --keys`: press keys and see, live, the bytes the terminal sent, the key
  caretline read and the command it runs. A key that prints nothing never reached caretline.
  `install.sh` takes `--version X.Y.Z`, `--ref REF` (build a branch or commit from source,
  leaving the installed caretline alone) and `-- ARGS` (then run `caretline ARGS`), so every
  change's "Try it" line is one command.

  Try it: `curl -fsSL https://caretline.app/install.sh | sh -s -- --ref main -- doctor --keys`

### Fixed

- In a terminal without the kitty keyboard protocol, Shift-Enter, Ctrl-Enter, Shift-Esc and
  the other keys Ghostty sends in xterm's modifyOtherKeys form (`CSI 27;mods;code ~`) were
  dropped; they now arrive (Shift-Enter is the outline's soft break). `CARETLINE_KEYBOARD=legacy`
  leaves the keyboard protocol off, to see those forms.

  Try it: `curl -fsSL https://caretline.app/install.sh | CARETLINE_KEYBOARD=legacy sh -s -- --ref main -- doctor --keys`
  (press Shift-Enter: `⎋[27;2;13~`)
- `caretline doctor` misses no key Ghostty keeps: it runs each binding through a model of
  Ghostty's keyboard (its 1.3.1 defaults, its encoding checked against 180 measured key
  presses), so it now also lists Cmd-Shift-Up and Cmd-Shift-Down (`jump_to_prompt`).

  Try it: `curl -fsSL https://caretline.app/install.sh | sh -s -- --ref main -- doctor`

### Changed

- One input decoder: the editor reads the terminal's bytes with its own parser always, not
  crossterm's event reader (which only demos without layers used). It decodes the kitty
  keyboard protocol's `CSI … u` keys with every modifier (Cmd, hyper and meta included,
  caps and num lock ignored) and the legacy xterm forms, as crossterm did; one startup probe
  asks for the keyboard protocol, and for pixels when a demo wants them. Demos with layers
  now get the keyboard protocol too, so Esc no longer waits out the lone-ESC timeout there.
  Where crossterm dropped a key the parser now reads it: Alt-`[` and Alt-Shift-O at the end
  of a read, and two ESCs as two Esc presses.

  Try it: `curl -fsSL https://caretline.app/install.sh | sh -s -- --ref main -- doctor --keys`
  (Esc shows as `⎋[27u`, Cmd-A as `⎋[97;9u` once Ghostty passes it)

## 0.5.0 — 2026-10-09

### Added

- `caretline doctor`: in Ghostty, the keys Ghostty's own bindings take before caretline
  sees them (by default Cmd-Up and Cmd-Down, Cmd-Home and Cmd-End, Cmd-Z and Cmd-Shift-Z;
  Cmd-A, which selected the whole terminal screen), compared with caretline's keymap, and
  the config lines that hand them to caretline.

### Fixed

- Several carets show: the others are drawn as reverse cells beside the terminal's cursor
  (caretline 0.5), and the pane under the editor draws its view's caret in the same role.

## 0.4.5 — 2026-10-09

### Fixed

- With `install.sh --demo`, Enter on a chapter of the welcome hid the page's overlay but
  started the chapter only on a second key. On macOS poll(2) can't wait on `/dev/tty`, so
  the welcome's key reader sat in a blocking read when it stopped; it now waits with
  select(2). A test starts a chapter with one Enter, keys from `/dev/tty`.

## 0.4.4 — 2026-10-09

### Fixed

- The tour and the agent demo always show the way out at the end of their status line:
  "⌃Q quit", or "⌃Q next demo" when started from the welcome. In the agent demo one ⌃Q
  leaves: its document is scratch the agent keeps changing, so there is no "unsaved
  changes" to confirm. The tour's last step no longer names a command to type.

## 0.4.3 — 2026-10-09

### Fixed

- `install.sh --demo` hung on macOS when a chapter started (the tour, the agent): with keys
  coming from `/dev/tty`, crossterm's kqueue input source can't poll it and its start-up
  query spun forever. crossterm now reads `/dev/tty` with poll(2) (its `use-dev-tty`
  feature). A test runs the tour the way the installer does.

## 0.4.2 — 2026-10-09

### Fixed

- The release binaries for 0.4.1's changes (the welcome demo, `install.sh --demo`, the
  terminal reader fix). 0.4.1's GitHub release failed its smoke test, which still expected
  the tour as the default demo, so 0.4.1 is on crates.io only.

## 0.4.1 — 2026-10-09

### Fixed

- An editor that quits stops reading the terminal before it returns. Its input thread
  used to stay behind and take the next key, so the first key of a demo chapter started
  from the welcome (and the first key back on the welcome) could be lost.

### Added

- `caretline demo` opens a welcome page, where a new user starts: what caretline is, the five
  demos as chapters (showcase, tour, agent, layers, scenes) and commands to take away. Enter
  starts the chapter under the highlight and returns to the page when it ends, the chapter
  ticked and the next one chosen. The page is an outline document: the highlight is a block
  selection, and the callout beside it a layer (pixels in Ghostty, cells elsewhere).
- The installer takes `--demo`: `curl -fsSL https://caretline.app/install.sh | sh -s -- --demo`
  installs and starts the welcome, reading keys from the terminal, even before the install
  directory is on `PATH`.

### Changed

- The tour is no longer the default demo: run it with `caretline demo tour`.

## 0.4.0 — 2026-10-09

### Added

- Held-pointer auto-scroll: a drag held still on the first or last text row (or past it,
  on the status bar) keeps scrolling. While the button is down and the last drag scrolled
  the view, the editor sends the same `drag` again about every 50 ms (faster past the last
  text row), through the normal message path, so the trace records each one and a replay
  scrolls the same. It stops on release, when the pointer moves off the edge, or when the
  view can't scroll further.
- `caretline serve --exit-with-parent`: the server exits (removing its socket) once the
  process that started it is gone, even if that process was killed and could clean nothing
  up. Checked every 100 ms (macOS has no parent-death signal). Off by default: a server
  started in the background from a shell still outlives the shell. The tests start every
  `serve --socket` with it, so a killed `cargo test` leaves no server behind.
- `caretline demo layers`: a hint with an arrow, a ring and a spotlight over the tour's
  text, placed by `caretline-layers`. In Ghostty (and kitty) it draws in pixels: a panel with
  a soft shadow under the box's words, and an anti-aliased arrow, ring and a veil with
  feathered holes over the text, rasterised with tiny-skia and sent with the kitty graphics
  protocol inside the frame's synchronized update. Elsewhere, and in `--snapshot`, it draws in
  cells: a rounded box, the arrow in box glyphs, the anchor in a ring style and the rest
  dimmed. Keys: ↑ ↓ PgUp PgDn scroll, `s` spotlight, `p` pixels or cells, `t` the transport
  (inline or temporary files), `q` quits.
- The pixel probe: with layers, the runtime asks the terminal at startup (a graphics query,
  XTVERSION and the cell size, fenced by DA1, 200 ms at most) and uses pixels only when the
  graphics query says OK, the cell size comes back and the terminal is Ghostty or kitty.
  `CARETLINE_LAYERS=auto|pixels|cells` overrides it; inside tmux or screen it stays in cells.
  It asks for the cell size again when the window's pixels stop matching (a font change).
- With layers, the runtime reads the terminal's input itself, so replies that arrive
  mid-session become messages to the runtime, never keys. Images are deleted by id on exit
  and while the keys overlay shows.
