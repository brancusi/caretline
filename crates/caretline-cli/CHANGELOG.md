# Changelog

## Unreleased

### Added

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
