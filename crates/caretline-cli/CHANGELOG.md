# Changelog

## Unreleased

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
