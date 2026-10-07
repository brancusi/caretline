//! `caretline demo layers`: a hint with an arrow, a ring and a spotlight over the tour's text,
//! placed by `caretline-layers` and drawn by the CLI's renderers: in pixels in Ghostty (a
//! panel under the words, an anti-aliased arrow, ring and veil over them), in cells
//! everywhere else.
//!
//! Keys: ↑ ↓ PgUp PgDn scroll (the hint follows its word), s spotlight, p pixels or cells,
//! t the pixel transport (inline or temporary files), q quits. The text is read-only here.

use std::time::Instant;

use caretline::{Frame, Key, KeyCode, Msg};
use caretline_layers::kitty::{KittyState, Options, TempFiles, Transport};
use caretline_layers::{
    Anchor, Content, FrameResolver, Grid, HINT, Layer, LayerOp, Layers, Limits, Renderers,
    Spotlight, apply, plan,
};

use crate::hub::Hub;
use crate::layers::{self, HintRenderer, Rasters, Surface};
use crate::runtime::{Decor, Demo, Gfx, KeyAction, dispatch_demo};

/// The word the hint points at, in the tour's "Move" section.
const SECTION: &str = "## 2 · Move";
const WORD: &str = "word";

pub(crate) const TITLE: &str = "Jump by word";
pub(crate) const TEXT: &str = "⌥← and ⌥→ move the caret one word at a time. This box, its arrow, the ring and the spotlight are layers: placed by caretline-layers, drawn by the CLI.";

/// What one pixel frame cost, for the status bar.
#[derive(Default, Clone, Copy)]
struct Cost {
    bytes: usize,
    sent: usize,
    micros: u128,
}

pub(crate) struct LayersDemo {
    spotlight: bool,
    /// The person's choice; pixels show only when the probe allows them.
    pixels: bool,
    kitty: KittyState,
    rasters: Rasters,
    generation: u64,
    cost: Cost,
}

/// `t=t` files in the system temp dir (`$TMPDIR` on macOS, as Ghostty requires).
struct TmpFiles;

impl TempFiles for TmpFiles {
    fn write(&mut self, name: &str, data: &[u8]) -> Option<String> {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, data).ok()?;
        Some(path.to_string_lossy().into_owned())
    }
}

impl LayersDemo {
    pub(crate) fn new() -> LayersDemo {
        LayersDemo {
            spotlight: true,
            pixels: true,
            kitty: KittyState::new(Options::default()),
            rasters: Rasters::default(),
            generation: 0,
            cost: Cost::default(),
        }
    }

    /// The layers for the current text: the hint at the word, found afresh, so it follows
    /// whatever the text does.
    pub(crate) fn layers(&self, text: &str) -> Layers {
        let mut layers = Layers::default();
        let Some(s) = text.find(SECTION) else {
            return layers;
        };
        let Some(w) = text[s..].find(WORD) else {
            return layers;
        };
        let from = text[..s + w].chars().count();
        let mut l = Layer::new(Anchor::Text {
            from,
            to: from + WORD.chars().count(),
        })
        .with_content(Content::hint(Some(TITLE), TEXT))
        .with_arrow()
        .with_ring();
        l.id = "hint".into();
        if self.spotlight {
            l.spotlight = Some(Spotlight::default());
        }
        let _ = apply(&mut layers, LayerOp::Push(l), None, 0, &Limits::default());
        layers
    }

    fn bump(&mut self) -> KeyAction {
        self.generation += 1;
        KeyAction::Consumed
    }

    /// Draws the layers over `frame`, in pixels when `gfx` allows and the person wants them.
    pub(crate) fn paint(&mut self, text: &str, frame: &mut Frame, gfx: &Gfx) -> Decor {
        let layers = self.layers(text);
        let grid = Grid::from_frame(frame);
        let renderers = Renderers::new().register(HINT, HintRenderer);
        let p = plan(&layers, &FrameResolver::new(frame), &grid, &renderers);
        let px = gfx.cell_px.filter(|_| self.pixels);
        let surface = if px.is_some() {
            Surface::TextOnly
        } else {
            Surface::Cells
        };
        let dim = layers::draw(frame, &p, &layers, surface);
        let bytes = match px {
            Some(cell) => {
                let t = Instant::now();
                let made = self.rasters.made;
                let pics = layers::pictures(&p, cell, grid.area, &self.kitty, &mut self.rasters);
                let file = self.kitty.options().transport == Transport::File;
                let mut files = TmpFiles;
                let out =
                    self.kitty
                        .frame(&p, &pics, cell, if file { Some(&mut files) } else { None });
                if !out.bytes.is_empty() {
                    self.cost = Cost {
                        bytes: out.bytes.len(),
                        sent: (self.rasters.made - made) as usize,
                        micros: t.elapsed().as_micros(),
                    };
                }
                out.bytes
            }
            None => self.kitty.clear(),
        };
        self.status(frame, gfx);
        Decor { dim, bytes }
    }

    /// The demo's own status line, over the editor's.
    fn status(&self, frame: &mut Frame, gfx: &Gfx) {
        let Some(y) = frame.height.checked_sub(1) else {
            return;
        };
        let mode = match (gfx.cell_px, self.pixels) {
            (Some(c), true) => {
                let via = if self.kitty.options().transport == Transport::File {
                    "t=t"
                } else {
                    "t=d"
                };
                let last = if self.cost.bytes > 0 {
                    format!(
                        " · last {} B, {} raster, {:.1} ms",
                        self.cost.bytes,
                        self.cost.sent,
                        self.cost.micros as f64 / 1e3
                    )
                } else {
                    String::new()
                };
                format!("pixels {}×{} {via}{last} · p cells", c.w, c.h)
            }
            (Some(_), false) => "cells · p pixels".into(),
            (None, _) => format!(
                "cells ({})",
                if gfx.why.is_empty() {
                    "no pixels"
                } else {
                    &gfx.why
                }
            ),
        };
        let spot = if self.spotlight {
            "s spotlight off"
        } else {
            "s spotlight"
        };
        let text = format!(" layers · {mode} · ↑↓ scroll · {spot} · q quit");
        layers::status(frame, y, &text);
    }
}

impl Demo for LayersDemo {
    fn key(&mut self, hub: &mut Hub, key: &Key) -> KeyAction {
        let plain = !key.mods.ctrl && !key.mods.alt && !key.mods.cmd;
        let rows = hub.session.state().text_rows() as i32;
        let scroll = |hub: &mut Hub, rows: i32| {
            dispatch_demo(hub, vec![Msg::ScrollView { rows }]);
            KeyAction::Consumed
        };
        match key.code {
            KeyCode::Up if plain => scroll(hub, -1),
            KeyCode::Down if plain => scroll(hub, 1),
            KeyCode::PageUp => scroll(hub, -(rows - 2).max(1)),
            KeyCode::PageDown => scroll(hub, (rows - 2).max(1)),
            KeyCode::Char('s') if plain => {
                self.spotlight = !self.spotlight;
                self.bump()
            }
            KeyCode::Char('p') if plain => {
                self.pixels = !self.pixels;
                self.bump()
            }
            KeyCode::Char('t') if plain => {
                let transport = match self.kitty.options().transport {
                    Transport::Direct => Transport::File,
                    Transport::File => Transport::Direct,
                };
                self.kitty.set_options(Options {
                    transport,
                    ..self.kitty.options()
                });
                // Send everything again the new way.
                self.kitty.reset();
                self.bump()
            }
            KeyCode::Char('q') | KeyCode::Esc if plain => KeyAction::Quit,
            // The caret moves; nothing edits.
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => KeyAction::Pass,
            KeyCode::Char('c' | 'q') if key.mods.ctrl => KeyAction::Quit,
            _ => KeyAction::Consumed,
        }
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn wants_pixels(&self) -> bool {
        true
    }

    fn decorate(&mut self, hub: &Hub, frame: &mut Frame, gfx: &Gfx) -> Decor {
        let text = hub.session.state().doc.text.to_string();
        self.paint(&text, frame, gfx)
    }

    fn hide(&mut self) -> Vec<u8> {
        self.kitty.clear()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caretline::{Session, Viewport};
    use caretline_layers::kitty::CellPx;

    fn hub(w: u16, h: u16) -> Hub {
        Hub::new(
            Session::new(super::super::layers_state(
                None,
                Viewport {
                    width: w,
                    height: h,
                },
            )),
            None,
        )
    }

    fn pixels(c: CellPx) -> Gfx {
        Gfx {
            cell_px: Some(c),
            terminal: Some("ghostty 1.3.1".into()),
            why: String::new(),
            local: true,
        }
    }

    fn frame(demo: &mut LayersDemo, hub: &Hub, gfx: &Gfx) -> (Frame, Decor) {
        let mut f = crate::runtime::compose(hub, 0);
        let d = demo.decorate(hub, &mut f, gfx);
        (f, d)
    }

    fn text(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    fn press(demo: &mut LayersDemo, hub: &mut Hub, code: KeyCode) {
        demo.key(
            hub,
            &Key {
                code,
                mods: Default::default(),
            },
        );
    }

    #[test]
    fn pixels_send_once_then_scrolls_only_re_place() {
        let gfx = pixels(CellPx::new(8, 16));
        let mut demo = LayersDemo::new();
        let mut hub = hub(100, 30);
        let (f, d) = frame(&mut demo, &hub, &gfx);
        let first = text(&d.bytes);
        assert_eq!(
            first.matches("a=t,").count(),
            4,
            "panel, veil, arrow and ring"
        );
        assert!(d.dim.is_empty(), "the veil dims, not the cells");
        // The box's cells hold its words and no border.
        let t = f.to_text();
        assert!(t.contains("Jump by word") && !t.contains('╭'), "{t}");
        // Nothing changed: nothing sent.
        assert!(frame(&mut demo, &hub, &gfx).1.bytes.is_empty());
        // A scroll: everything moves together, re-placed and re-cropped, no pixels.
        press(&mut demo, &mut hub, KeyCode::Down);
        let b = text(&frame(&mut demo, &hub, &gfx).1.bytes);
        assert!(
            !b.is_empty() && !b.contains("a=t,") && !b.contains("a=d"),
            "{b}"
        );
        // Spotlight off: the veil is deleted, nothing re-rasterised.
        press(&mut demo, &mut hub, KeyCode::Char('s'));
        let b = text(&frame(&mut demo, &hub, &gfx).1.bytes);
        assert!(
            !b.contains("a=t,") && b.matches("a=d,d=I").count() == 1,
            "{b}"
        );
        // Cells: every image deleted; the box drawn in box glyphs.
        press(&mut demo, &mut hub, KeyCode::Char('p'));
        let (f, d) = frame(&mut demo, &hub, &gfx);
        assert_eq!(text(&d.bytes).matches("a=d,d=I").count(), 3);
        assert!(f.to_text().contains('╭'));
        assert!(demo.hide().is_empty());
    }

    #[test]
    fn the_same_frames_give_the_same_bytes() {
        let run = || {
            let gfx = pixels(CellPx::new(8, 16));
            let mut demo = LayersDemo::new();
            let mut hub = hub(90, 28);
            let mut all = Vec::new();
            for k in [
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Char('s'),
                KeyCode::Up,
                KeyCode::Char('s'),
            ] {
                all.push(frame(&mut demo, &hub, &gfx).1.bytes);
                press(&mut demo, &mut hub, k);
            }
            all
        };
        assert_eq!(run(), run());
    }

    /// A picture of a pixel frame for a person to look at: text cells as grey bars, the
    /// pictures composited where the terminal would place them.
    /// `LAYERS_PNG=out.png cargo test -p caretline-cli composite_png -- --ignored`.
    #[test]
    #[ignore]
    fn composite_png() {
        use tiny_skia::{Pixmap, PixmapPaint, Transform};
        let Ok(out) = std::env::var("LAYERS_PNG") else {
            return;
        };
        let cell = CellPx::new(8, 16);
        let mut demo = LayersDemo::new();
        let mut hub = hub(100, 30);
        for _ in 0..3 {
            press(&mut demo, &mut hub, KeyCode::Down);
        }
        let mut f = crate::runtime::compose(&hub, 0);
        let text = hub.session.state().doc.text.to_string();
        let layers = demo.layers(&text);
        let grid = Grid::from_frame(&f);
        let p = plan(
            &layers,
            &FrameResolver::new(&f),
            &grid,
            &Renderers::new().register(HINT, HintRenderer),
        );
        layers::draw(&mut f, &p, &layers, Surface::TextOnly);
        let (w, h) = (f.width as u32 * 8, f.height as u32 * 16);
        let mut pm = Pixmap::new(w, h).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(18, 20, 26, 255));
        let mut pics = layers::pictures(
            &p,
            cell,
            grid.area,
            &KittyState::default(),
            &mut Rasters::default(),
        );
        pics.sort_by_key(|p| p.z == caretline_layers::kitty::Z::Above);
        let draw = |pm: &mut Pixmap, pic: &caretline_layers::kitty::Picture| {
            let img = pic.image.as_ref().unwrap();
            let mut data = Vec::new();
            for c in img.rgba.chunks(4) {
                let a = c[3] as u32;
                data.extend_from_slice(&[
                    (c[0] as u32 * a / 255) as u8,
                    (c[1] as u32 * a / 255) as u8,
                    (c[2] as u32 * a / 255) as u8,
                    c[3],
                ]);
            }
            let src =
                Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(img.w, img.h).unwrap()).unwrap();
            let sx = pic.at.w as f32 * 8.0 / img.w as f32;
            let sy = pic.at.h as f32 * 16.0 / img.h as f32;
            let clip = pic
                .clip
                .unwrap_or(caretline_layers::Rect::new(0, 0, 100, 30));
            let mut mask = tiny_skia::Mask::new(w, h).unwrap();
            let r = tiny_skia::Rect::from_xywh(
                clip.x as f32 * 8.0,
                clip.y as f32 * 16.0,
                clip.w as f32 * 8.0,
                clip.h as f32 * 16.0,
            )
            .unwrap();
            mask.fill_path(
                &tiny_skia::PathBuilder::from_rect(r),
                tiny_skia::FillRule::Winding,
                false,
                Transform::identity(),
            );
            let t = Transform::from_row(
                sx,
                0.0,
                0.0,
                sy,
                pic.at.x as f32 * 8.0,
                pic.at.y as f32 * 16.0,
            );
            pm.draw_pixmap(0, 0, src.as_ref(), &PixmapPaint::default(), t, Some(&mask));
        };
        for pic in pics
            .iter()
            .filter(|p| p.z == caretline_layers::kitty::Z::Below)
        {
            draw(&mut pm, pic);
        }
        let mut glyph = tiny_skia::Paint::default();
        glyph.set_color_rgba8(200, 205, 215, 255);
        for y in 0..f.height {
            for x in 0..f.width {
                if !f.cell(x, y).symbol.trim().is_empty() {
                    let r = tiny_skia::Rect::from_xywh(
                        x as f32 * 8.0 + 1.0,
                        y as f32 * 16.0 + 5.0,
                        6.0,
                        8.0,
                    )
                    .unwrap();
                    pm.fill_rect(r, &glyph, Transform::identity(), None);
                }
            }
        }
        for pic in pics
            .iter()
            .filter(|p| p.z == caretline_layers::kitty::Z::Above)
        {
            draw(&mut pm, pic);
        }
        pm.save_png(out).unwrap();
    }

    /// Sizes and timings at the spike's size (158×37 cells of 16×34 device pixels):
    /// `cargo test -p caretline-cli --release pixel_costs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn pixel_costs() {
        let gfx = pixels(CellPx::new(16, 34));
        let mut demo = LayersDemo::new();
        let mut hub = hub(158, 37);
        let t = Instant::now();
        let first = frame(&mut demo, &hub, &gfx).1.bytes.len();
        println!(
            "first frame (raster + zlib + bytes): {first} B in {:.2} ms",
            t.elapsed().as_secs_f64() * 1e3
        );
        let mut scroll = Vec::new();
        for _ in 0..6 {
            press(&mut demo, &mut hub, KeyCode::Down);
            let t = Instant::now();
            let b = frame(&mut demo, &hub, &gfx).1.bytes.len();
            scroll.push((b, t.elapsed().as_micros()));
        }
        println!("scroll steps (B, µs incl. plan and draw): {scroll:?}");
        let t = Instant::now();
        let same = frame(&mut demo, &hub, &gfx).1.bytes.len();
        println!(
            "unchanged frame: {same} B in {} µs",
            t.elapsed().as_micros()
        );
        press(&mut demo, &mut hub, KeyCode::Char('s'));
        println!(
            "spotlight off: {} B",
            frame(&mut demo, &hub, &gfx).1.bytes.len()
        );
        press(&mut demo, &mut hub, KeyCode::Char('s'));
        let t = Instant::now();
        let b = frame(&mut demo, &hub, &gfx).1.bytes.len();
        println!(
            "spotlight on again (cached raster): {b} B in {:.2} ms",
            t.elapsed().as_secs_f64() * 1e3
        );
        let mut fresh = LayersDemo::new();
        fresh.kitty.set_options(Options {
            transport: Transport::File,
            ..Options::default()
        });
        let t = Instant::now();
        let b = frame(&mut fresh, &hub, &gfx).1.bytes.len();
        println!(
            "first frame with t=t: {b} B in {:.2} ms",
            t.elapsed().as_secs_f64() * 1e3
        );
        for p in std::fs::read_dir(std::env::temp_dir()).unwrap().flatten() {
            if p.file_name()
                .to_string_lossy()
                .starts_with("caretline-tty-graphics-protocol-")
            {
                let _ = std::fs::remove_file(p.path());
            }
        }
    }
}
