//! Brand typography and fractional-pixel motion. The host draws the pictures;
//! kitty owns their placement/lifecycle. Font outlines are bundled, licensed data.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use caretline::Frame;
use caretline_layers::kitty::{CellPx, Cells, Picture, Z, shape_key};
use caretline_layers::{
    Anchor, Grid, Layer, LayerOp, Layers, Limits, Rect, Renderers, ScreenPos, apply, plan,
};
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

use super::super::canvas::Canvas;
use super::deck::Art;
use crate::layers::{self, ACCENT, DIM, INK, PANEL, RULE, VOID};
use crate::runtime::{Decor, Gfx};

pub(super) fn licenses() -> String {
    format!(
        "Geist\n{}\nGeist Mono\n{}",
        include_str!("../../../assets/typography/geist.LICENSE.txt"),
        include_str!("../../../assets/typography/geist-mono.LICENSE.txt")
    )
}

struct Glyph {
    advance: f32,
    path: Option<Path>,
}
struct Face {
    glyphs: BTreeMap<char, Glyph>,
}

impl Face {
    fn load(data: &str) -> Self {
        let value: serde_json::Value = serde_json::from_str(data).expect("bundled outline JSON");
        let glyphs = value["glyphs"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(character, glyph)| {
                let mut pb = PathBuilder::new();
                for command in glyph["commands"].as_array().unwrap() {
                    let n = |i| command[i].as_f64().unwrap() as f32;
                    match command[0].as_str().unwrap() {
                        "M" => pb.move_to(n(1), n(2)),
                        "L" => pb.line_to(n(1), n(2)),
                        "Q" => pb.quad_to(n(1), n(2), n(3), n(4)),
                        "C" => pb.cubic_to(n(1), n(2), n(3), n(4), n(5), n(6)),
                        "Z" => pb.close(),
                        _ => unreachable!("generated path command"),
                    }
                }
                (
                    character.chars().next().unwrap(),
                    Glyph {
                        advance: glyph["advance"].as_f64().unwrap() as f32,
                        path: pb.finish(),
                    },
                )
            })
            .collect();
        Self { glyphs }
    }

    fn width(&self, text: &str, size: f32) -> f32 {
        text.chars()
            .filter_map(|c| self.glyphs.get(&c))
            .map(|g| g.advance * size)
            .sum()
    }

    fn draw(
        &self,
        pm: &mut Pixmap,
        text: &str,
        x: f32,
        baseline: f32,
        size: f32,
        colour: (u8, u8, u8),
    ) {
        let mut x = x;
        for c in text.chars() {
            if let Some(glyph) = self.glyphs.get(&c) {
                if let Some(path) = &glyph.path {
                    let transform = Transform::from_row(size, 0.0, 0.0, size, x, baseline);
                    pm.fill_path(
                        path,
                        &paint(colour, 1.0),
                        FillRule::Winding,
                        transform,
                        None,
                    );
                }
                x += glyph.advance * size;
            }
        }
    }
}

fn mono() -> &'static Face {
    static FACE: OnceLock<Face> = OnceLock::new();
    FACE.get_or_init(|| Face::load(include_str!("../../../assets/typography/geist-mono.json")))
}
fn sans() -> &'static Face {
    static FACE: OnceLock<Face> = OnceLock::new();
    FACE.get_or_init(|| Face::load(include_str!("../../../assets/typography/geist.json")))
}
fn paint(c: (u8, u8, u8), alpha: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.0, c.1, c.2, (alpha * 255.0) as u8);
    p.anti_alias = true;
    p
}

fn line(pm: &mut Pixmap, from: (f32, f32), to: (f32, f32), colour: (u8, u8, u8), width: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(from.0, from.1);
    pb.line_to(to.0, to.1);
    if let Some(path) = pb.finish() {
        pm.stroke_path(
            &path,
            &paint(colour, 1.0),
            &Stroke {
                width,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
}

/// Deliberately bounded: a very large terminal doesn't allocate a full-screen 4K raster.
fn area(frame: &Frame) -> Rect {
    let w = frame.width.saturating_sub(4).min(96).max(1);
    let h = frame.height.saturating_sub(8).min(20).max(1);
    Rect::new(
        frame.width.saturating_sub(w) / 2,
        4.min(frame.height.saturating_sub(1)),
        w,
        h,
    )
}

fn typography(w: u32, h: u32, cell: CellPx) -> Pixmap {
    let mut pm = Pixmap::new(w.max(1), h.max(1)).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(PANEL.0, PANEL.1, PANEL.2, 255));
    let margin = cell.w as f32;
    let column = w as f32 / 2.0;
    line(
        &mut pm,
        (column, margin),
        (column, h as f32 - margin),
        RULE,
        1.0,
    );
    for (i, face) in [mono(), sans()].iter().enumerate() {
        let x = i as f32 * column + margin;
        let available = (column - margin * 2.0).max(1.0);
        let label_size = (cell.h as f32 * 0.42).min(available / 7.0).max(1.0);
        mono().draw(
            &mut pm,
            if i == 0 { "GEIST MONO" } else { "GEIST" },
            x,
            cell.h as f32 * 0.8,
            label_size,
            DIM,
        );
        for (row, scale) in [1.0, 1.6, 2.5].iter().enumerate() {
            let top = cell.h as f32 + row as f32 * (h as f32 - cell.h as f32) / 3.0;
            let size = (cell.h as f32 * scale)
                .min(available / face.width("State", 1.0).max(0.01))
                .min((h as f32 - cell.h as f32) / 3.0 * 0.68)
                .max(1.0);
            face.draw(&mut pm, "State", x, top + size, size, INK);
            mono().draw(
                &mut pm,
                &format!("{size:.0}px / 500"),
                x,
                top + size + label_size * 1.6,
                label_size,
                DIM,
            );
        }
    }
    pm
}

/// A vector path on fractional DEVICE pixels, not RGB/LCD subpixel font hinting.
fn motion(w: u32, h: u32, cell: CellPx, t: f64) -> Pixmap {
    let mut pm = Pixmap::new(w.max(1), h.max(1)).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(VOID.0, VOID.1, VOID.2, 255));
    for x in (0..w).step_by(cell.w.max(1) as usize) {
        line(&mut pm, (x as f32, 0.0), (x as f32, h as f32), RULE, 0.5);
    }
    for y in (0..h).step_by(cell.h.max(1) as usize) {
        line(&mut pm, (0.0, y as f32), (w as f32, y as f32), RULE, 0.5);
    }
    let wf = w as f32;
    let hf = h as f32;
    let t = t as f32;
    let mut pb = PathBuilder::new();
    pb.move_to(wf * 0.08, hf * 0.65);
    pb.cubic_to(
        wf * 0.28,
        hf * (0.15 + 0.12 * (t * 0.8).sin()),
        wf * 0.62,
        hf * (0.92 + 0.07 * (t * 1.2).cos()),
        wf * 0.92,
        hf * 0.38,
    );
    if let Some(path) = pb.finish() {
        pm.stroke_path(
            &path,
            &paint(INK, 0.85),
            &Stroke {
                width: (cell.w as f32 / 9.0).max(1.0),
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
    let x = wf * (0.5 + 0.38 * (t * 0.65).sin());
    let y = hf * (0.5 + 0.24 * (t * 0.9).cos());
    if let Some(rect) =
        tiny_skia::Rect::from_xywh(x - 1.25, y - cell.h as f32 * 0.65, 2.5, cell.h as f32 * 1.3)
    {
        pm.fill_rect(rect, &paint(ACCENT, 1.0), Transform::identity(), None);
    }
    mono().draw(
        &mut pm,
        "FRACTIONAL PIXELS",
        cell.w as f32,
        cell.h as f32,
        (cell.h as f32 * 0.52).max(1.0),
        INK,
    );
    mono().draw(
        &mut pm,
        &format!("x {x:.2} / y {y:.2}"),
        cell.w as f32,
        hf - cell.h as f32 * 0.4,
        (cell.h as f32 * 0.42).max(1.0),
        DIM,
    );
    pm
}

pub(super) fn paint_graphics(
    canvas: &mut Canvas,
    art: Art,
    frame: &Frame,
    gfx: &Gfx,
    seconds: f64,
) -> Decor {
    let Some(cell) = gfx.cell_px.filter(|_| canvas.pixels) else {
        return Decor {
            dim: Vec::new(),
            bytes: canvas.hide(),
        };
    };
    let area = area(frame);
    let mut layers = Layers::default();
    let mut layer = Layer::new(Anchor::Screen(ScreenPos::Center)).with_ring();
    layer.id = "gallery".into();
    apply(
        &mut layers,
        LayerOp::Push(layer),
        None,
        0,
        &Limits::default(),
    )
    .unwrap();
    let p = plan(
        &layers,
        &caretline_layers::FrameResolver::new(frame),
        &Grid::from_frame(frame).with_area(area),
        &Renderers::new(),
    );
    let phase = if art == Art::Motion {
        (seconds * 24.0).floor() as u64
    } else {
        0
    };
    let key = shape_key(&[
        b"showcase-brand-gallery-1",
        &[art as u8],
        &area.w.to_le_bytes(),
        &area.h.to_le_bytes(),
        &phase.to_le_bytes(),
    ]);
    let image = if canvas.kitty.holds(key, cell) {
        None
    } else {
        let (w, h) = (area.w as u32 * cell.w as u32, area.h as u32 * cell.h as u32);
        let pm = if art == Art::Typography {
            typography(w, h, cell)
        } else {
            motion(w, h, cell, phase as f64 / 24.0)
        };
        Some(layers::straight(&pm))
    };
    let picture = Picture {
        layer: "gallery".into(),
        part: "content".into(),
        key,
        z: Z::Above,
        at: Cells::from(area),
        clip: Some(area),
        image,
    };
    Decor {
        dim: Vec::new(),
        bytes: canvas.emit(&p, &[picture], cell),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn write_graphics_previews() {
        let Ok(directory) = std::env::var("SHOWCASE_PREVIEWS") else {
            return;
        };
        let directory = std::path::Path::new(&directory);
        typography(1536, 680, CellPx::new(16, 34))
            .save_png(directory.join("typography.png"))
            .unwrap();
        motion(1536, 680, CellPx::new(16, 34), 2.35)
            .save_png(directory.join("motion.png"))
            .unwrap();
    }
    #[test]
    fn bundled_faces_are_real_distinct_outlines_at_several_sizes() {
        assert!(mono().glyphs[&'S'].path.is_some() && sans().glyphs[&'S'].path.is_some());
        assert_ne!(mono().width("State", 1.0), sans().width("State", 1.0));
        let a = typography(768, 320, CellPx::new(8, 16));
        assert!(
            a.pixels()
                .iter()
                .any(|p| p.red() == INK.0 && p.green() == INK.1)
        );
        assert_ne!(
            motion(400, 240, CellPx::new(8, 16), 0.0).data(),
            motion(400, 240, CellPx::new(8, 16), 0.15).data()
        );
    }
}
