//! The CLI's renderers for `caretline-layers` (the CLI is one host; another draws its own
//! way). The `hint` kind: a rounded box with a title and wrapped text, an arrow along the
//! route, a ring round the anchor and a spotlight.
//!
//! - **Cells** (every terminal, snapshots, goldens): box drawing, `▲▼◀▶` arrow heads, the
//!   anchor's cells in the ring role, and a dim mask outside the spotlight's holes.
//! - **Pixels** (Ghostty, probed): the box's cells hold only its words; a panel image with a
//!   soft shadow goes under them (z below text), and an anti-aliased arrow, ring and a
//!   translucent veil with feathered holes over the text. Rasterised here with tiny-skia;
//!   `caretline_layers::kitty` turns the images into terminal bytes.

use std::collections::HashMap;

use caretline::helix::Tendril;
use caretline::view::{Cell, Role};
use caretline::Frame;
use caretline_layers::kitty::{CellPx, Cells, Image, KittyState, Picture, Z, shape_key};
use caretline_layers::{Anchor, Dir, Edge, Hint, Layers, Mode, Off, Plan, Planned, Rect, Renderer, Size, width};
use serde_json::Value;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};
use unicode_segmentation::UnicodeSegmentation;

/// The widest a hint's text runs, in columns.
pub const TEXT_COLS: usize = 44;

/// Raster version: goes into every shape key, so a change to the drawing re-sends.
const RASTER: &[u8] = b"cli-raster-1";

pub const ACCENT: (u8, u8, u8) = (138, 164, 255);
pub const PANEL: (u8, u8, u8) = (31, 36, 48);
pub const INK: (u8, u8, u8) = (216, 220, 230);

/// What the renderer draws in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Everything: box drawing, the arrow, ring and dimming.
    Cells,
    /// Only the words: pixels draw the rest.
    TextOnly,
}

/// A hint's lines at `inner` columns: the title (if any), then the text, wrapped by words.
pub fn hint_lines(data: &Value, inner: usize) -> (Option<String>, Vec<String>) {
    let h: Hint = serde_json::from_value(data.clone()).unwrap_or_default();
    (h.title, wrap(&h.text, inner.max(1)))
}

fn str_width(s: &str) -> usize {
    s.graphemes(true).map(width).sum()
}

/// Words wrapped to `w` columns; a word longer than that is cut.
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let ww = str_width(word);
            if !cur.is_empty() && str_width(&cur) + 1 + ww > w {
                lines.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(&truncate(word, w));
        }
        lines.push(cur);
    }
    lines
}

fn truncate(s: &str, w: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for g in s.graphemes(true) {
        let gw = width(g);
        if used + gw > w {
            break;
        }
        out.push_str(g);
        used += gw;
    }
    out
}

/// The `hint` kind's renderer: measures boxes and edge chips. Pure.
pub struct HintRenderer;

impl Renderer for HintRenderer {
    fn measure(&self, data: &Value, avail: Size) -> Size {
        let inner = (avail.w.saturating_sub(4) as usize).min(TEXT_COLS);
        if inner < 8 {
            return Size::new(0, 0);
        }
        let (title, lines) = hint_lines(data, inner);
        let w = title.iter().chain(&lines).map(|l| str_width(l)).max().unwrap_or(0).min(inner);
        let h = lines.len() + title.is_some() as usize;
        Size::new(w as u16 + 4, h as u16 + 2)
    }

    fn chip(&self, _data: &Value, _anchor: &Anchor, off: Off) -> Size {
        Size::new(str_width(&chip_label(off)) as u16 + 2, 1)
    }
}

fn chip_label(off: Off) -> String {
    match off {
        Off::Above { .. } => "↑ here",
        Off::Below { .. } => "↓ here",
        Off::Left { .. } => "← here",
        Off::Right { .. } => "→ here",
    }
    .to_string()
}

/// The role named `name` in this frame.
pub fn role(frame: &mut Frame, name: &str) -> Role {
    let i = match frame.roles.iter().position(|r| r == name) {
        Some(i) => i,
        None => {
            frame.roles.push(name.to_string());
            frame.roles.len() - 1
        }
    };
    Role::Named(i as u16)
}

/// Writes one grapheme, never leaving half of a wide one: a wide grapheme cut by this write
/// becomes a space.
fn put(frame: &mut Frame, x: u16, y: u16, g: &str, r: Role) -> u16 {
    let (fw, fh) = (frame.width, frame.height);
    if x >= fw || y >= fh {
        return 0;
    }
    let w = width(g).max(1) as u16;
    if x + w > fw {
        return 0;
    }
    let at = |x: u16| y as usize * fw as usize + x as usize;
    // Overwriting the second half of a wide grapheme: its first half goes.
    if x > 0 && frame.cells[at(x)].symbol.is_empty() {
        frame.cells[at(x - 1)].symbol = Tendril::from(" ");
    }
    // Overwriting a wide grapheme's first half with its second half left over.
    let end = x + w;
    if end < fw && frame.cells[at(end)].symbol.is_empty() {
        frame.cells[at(end)].symbol = Tendril::from(" ");
    }
    frame.cells[at(x)] = Cell { symbol: Tendril::from(g), role: r, char_idx: None };
    for cx in x + 1..end {
        frame.cells[at(cx)] = Cell { symbol: Tendril::new(), role: r, char_idx: None };
    }
    w
}

fn put_str(frame: &mut Frame, x: u16, y: u16, s: &str, limit: u16, r: Role) {
    let mut cx = x;
    for g in s.graphemes(true) {
        let w = width(g) as u16;
        if cx + w > limit {
            break;
        }
        put(frame, cx, y, g, r);
        cx += w;
    }
}

fn restyle(frame: &mut Frame, x: u16, y: u16, r: Role) {
    if x < frame.width && y < frame.height {
        let i = y as usize * frame.width as usize + x as usize;
        frame.cells[i].role = r;
    }
}

fn arrow_glyph(enter: Dir, leave: Dir, head: bool) -> &'static str {
    if head {
        return match leave {
            Dir::Up => "▲",
            Dir::Down => "▼",
            Dir::Left => "◀",
            Dir::Right => "▶",
        };
    }
    match (enter, leave) {
        (Dir::Up | Dir::Down, Dir::Up | Dir::Down) => "│",
        (Dir::Left | Dir::Right, Dir::Left | Dir::Right) => "─",
        (Dir::Right, Dir::Down) | (Dir::Up, Dir::Left) => "╮",
        (Dir::Left, Dir::Down) | (Dir::Up, Dir::Right) => "╭",
        (Dir::Right, Dir::Up) | (Dir::Down, Dir::Left) => "╯",
        _ => "╰",
    }
}

/// Draws the plan over the frame. Returns the cells to dim (empty without a spotlight, and
/// always in pixels, where the veil dims).
pub fn draw(frame: &mut Frame, plan: &Plan, layers: &Layers, surface: Surface) -> Vec<bool> {
    let cells = surface == Surface::Cells;
    let mut dim = Vec::new();
    if cells && !plan.spots.is_empty() {
        dim = (0..frame.height)
            .flat_map(|y| (0..frame.width).map(move |x| (x, y)))
            .map(|(x, y)| plan.dimmed(x, y))
            .collect();
    }
    for l in &plan.layers {
        let data = layers.get(&l.id).and_then(|x| x.content.as_ref()).map(|c| c.data.clone());
        if cells {
            let ring = role(frame, "layer.ring");
            for r in &l.ring {
                for x in r.x..r.right() {
                    restyle(frame, x, r.y, ring);
                }
            }
            if let Some(rt) = &l.route {
                let arrow = role(frame, "layer.arrow");
                let n = rt.steps.len();
                for (k, s) in rt.steps.iter().enumerate() {
                    put(frame, s.x, s.y, arrow_glyph(s.enter, s.leave, k + 1 == n), arrow);
                }
            }
        }
        if let (Some(r), Some(data)) = (l.rect, &data) {
            match l.mode {
                Some(Mode::Strip) => draw_strip(frame, r, data),
                _ => draw_box(frame, l, r, data, surface),
            }
        }
        if let (Some(c), Some(off)) = (l.chip, l.anchor.as_ref().and_then(|a| a.off)) {
            let chip = role(frame, "layer.chip");
            put_str(frame, c.x, c.y, &format!(" {:<w$}", chip_label(off), w = c.w.saturating_sub(1) as usize), c.right(), chip);
        }
    }
    dim
}

/// Replaces row `y` with a status line.
pub fn status(frame: &mut Frame, y: u16, text: &str) {
    if y >= frame.height {
        return;
    }
    for x in 0..frame.width {
        put(frame, x, y, " ", Role::Status);
    }
    put_str(frame, 0, y, text, frame.width, Role::Status);
}

fn draw_strip(frame: &mut Frame, r: Rect, data: &Value) {
    let strip = role(frame, "layer.chip");
    let (title, lines) = hint_lines(data, usize::MAX / 2);
    let text = title.into_iter().chain(lines).collect::<Vec<_>>().join(" · ");
    for x in r.x..r.right() {
        put(frame, x, r.y, " ", strip);
    }
    put_str(frame, r.x + 1, r.y, &text, r.right().saturating_sub(1), strip);
}

fn draw_box(frame: &mut Frame, l: &Planned, r: Rect, data: &Value, surface: Surface) {
    let cells = surface == Surface::Cells;
    let (fill, border, title_role) = if cells {
        (role(frame, "layer.callout"), role(frame, "layer.border"), role(frame, "layer.title"))
    } else {
        (Role::Text, Role::Text, role(frame, "layer.title.px"))
    };
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            put(frame, x, y, " ", fill);
        }
    }
    if r.w < 2 || r.h < 2 {
        return;
    }
    let (x1, y1) = (r.right() - 1, r.bottom() - 1);
    if cells {
        for x in r.x + 1..x1 {
            put(frame, x, r.y, "─", border);
            put(frame, x, y1, "─", border);
        }
        for y in r.y + 1..y1 {
            put(frame, r.x, y, "│", border);
            put(frame, x1, y, "│", border);
        }
        put(frame, r.x, r.y, "╭", border);
        put(frame, x1, r.y, "╮", border);
        put(frame, r.x, y1, "╰", border);
        put(frame, x1, y1, "╯", border);
        if let Some(rt) = &l.route {
            let j = match rt.attach.edge {
                Edge::Top => "┴",
                Edge::Bottom => "┬",
                Edge::Left => "┤",
                Edge::Right => "├",
            };
            put(frame, rt.junction.0, rt.junction.1, j, border);
        }
    }
    let inner = r.w.saturating_sub(4) as usize;
    let (title, lines) = hint_lines(data, inner);
    let mut y = r.y + 1;
    if let Some(t) = title {
        put_str(frame, r.x + 2, y, &t, x1.saturating_sub(1), title_role);
        y += 1;
    }
    for line in lines {
        if y >= y1 {
            break;
        }
        put_str(frame, r.x + 2, y, &line, x1.saturating_sub(1), fill);
        y += 1;
    }
}

// ---------- pixels ----------

fn paint(c: (u8, u8, u8), a: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.0, c.1, c.2, (a.clamp(0.0, 1.0) * 255.0).round() as u8);
    p.anti_alias = true;
    p
}

fn stroke(w: f32) -> Stroke {
    Stroke { width: w, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Signed distance from a point to a rounded rect (x0, y0, x1, y1), radius `rad`.
fn sd_rrect(px: f32, py: f32, r: (f32, f32, f32, f32), rad: f32) -> f32 {
    let (cx, cy) = ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0);
    let (hw, hh) = ((r.2 - r.0) / 2.0, (r.3 - r.1) / 2.0);
    let rad = rad.min(hw).min(hh).max(0.0);
    let qx = (px - cx).abs() - (hw - rad);
    let qy = (py - cy).abs() - (hh - rad);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - rad
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let k = 0.5523 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

/// tiny-skia's premultiplied pixels as straight-alpha RGBA.
pub fn straight(pm: &Pixmap) -> Image {
    let mut v = Vec::with_capacity(pm.data().len());
    for p in pm.pixels() {
        let c = p.demultiply();
        v.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Image::new(pm.width(), pm.height(), v).expect("a pixmap's size")
}

/// The callout panel for a box of `w`×`h` cells, in an image one cell wider on each side and
/// one row taller (for the shadow): a soft shadow, a rounded fill and a 1 px rim.
pub fn raster_panel(w: u16, h: u16, cell: CellPx) -> Pixmap {
    let (cw, ch) = (cell.w as f32, cell.h as f32);
    let (iw, ih) = ((w as u32 + 2) * cell.w as u32, (h as u32 + 1) * cell.h as u32);
    let mut pm = Pixmap::new(iw.max(1), ih.max(1)).expect("a panel size");
    let (x, y) = (cw + cw * 0.15, ch * 0.12);
    let (pw, ph) = (w as f32 * cw - cw * 0.3, h as f32 * ch - ch * 0.24);
    let rad = (ch * 0.42).min(pw / 2.0);
    let (blur, off) = (ch * 0.45, ch * 0.18);
    let rect = (x, y, x + pw, y + ph);
    let data = pm.data_mut();
    for py in 0..ih {
        for px in 0..iw {
            let d = sd_rrect(px as f32 + 0.5, py as f32 + 0.5 - off, rect, rad);
            let a = 0.5 * (1.0 - smoothstep(-blur * 0.5, blur, d));
            data[((py * iw + px) * 4 + 3) as usize] = (a * 255.0) as u8;
        }
    }
    let id = Transform::identity();
    if let Some(p) = rrect(x, y, pw, ph, rad) {
        pm.fill_path(&p, &paint(PANEL, 0.97), FillRule::Winding, id, None);
    }
    if let Some(p) = rrect(x + 0.5, y + 0.5, pw - 1.0, ph - 1.0, rad - 0.5) {
        pm.stroke_path(&p, &paint(ACCENT, 0.7), &stroke(1.0_f32.max(cw / 12.0)), id, None);
    }
    pm
}

/// An arrow through cell points (relative to the image's top-left cell): rounded bends,
/// a filled head ending at `tip`, over a dark casing for contrast.
pub fn raster_arrow(cols: u16, rows: u16, pts: &[(f32, f32)], tip: (f32, f32), cell: CellPx) -> Pixmap {
    let (cw, ch) = (cell.w as f32, cell.h as f32);
    let mut pm = Pixmap::new(cols as u32 * cell.w as u32, rows as u32 * cell.h as u32).expect("an arrow size");
    let px = |p: (f32, f32)| (p.0 * cw, p.1 * ch);
    let mut all: Vec<(f32, f32)> = pts.iter().map(|&p| px(p)).collect();
    let tip = px(tip);
    all.push(tip);
    let n = all.len();
    let (hl, hw) = (ch * 0.42, ch * 0.24);
    // The head's base, `hl` back from the tip along the last segment.
    let last = all[n - 2];
    let (dx, dy) = (tip.0 - last.0, tip.1 - last.1);
    let dl = dx.hypot(dy).max(0.001);
    let (ux, uy) = (dx / dl, dy / dl);
    let base = (tip.0 - ux * hl, tip.1 - uy * hl);
    let mut line: Vec<(f32, f32)> = all[..n - 1].to_vec();
    line.push(base);
    let mut pb = PathBuilder::new();
    pb.move_to(line[0].0, line[0].1);
    let rad = cw.min(ch) * 0.9;
    for i in 1..line.len() {
        let p = line[i];
        if i + 1 < line.len() {
            let (a, c) = (line[i - 1], line[i + 1]);
            let l1 = (p.0 - a.0).hypot(p.1 - a.1);
            let l2 = (c.0 - p.0).hypot(c.1 - p.1);
            let r = rad.min(l1 / 2.0).min(l2 / 2.0);
            let b1 = (p.0 - (p.0 - a.0) / l1.max(0.001) * r, p.1 - (p.1 - a.1) / l1.max(0.001) * r);
            let b2 = (p.0 + (c.0 - p.0) / l2.max(0.001) * r, p.1 + (c.1 - p.1) / l2.max(0.001) * r);
            pb.line_to(b1.0, b1.1);
            pb.quad_to(p.0, p.1, b2.0, b2.1);
        } else {
            pb.line_to(p.0, p.1);
        }
    }
    let curve = pb.finish();
    let mut pb = PathBuilder::new();
    pb.move_to(tip.0, tip.1);
    pb.line_to(base.0 - uy * hw, base.1 + ux * hw);
    pb.line_to(base.0 + uy * hw, base.1 - ux * hw);
    pb.close();
    let head = pb.finish();
    let id = Transform::identity();
    let w = (ch / 11.0).max(1.5);
    if let Some(c) = &curve {
        pm.stroke_path(c, &paint((0, 0, 0), 0.45), &stroke(w * 2.4), id, None);
    }
    if let Some(h) = &head {
        pm.stroke_path(h, &paint((0, 0, 0), 0.45), &stroke(w * 1.4), id, None);
    }
    if let Some(c) = &curve {
        pm.stroke_path(c, &paint(ACCENT, 1.0), &stroke(w), id, None);
    }
    if let Some(h) = &head {
        pm.fill_path(h, &paint(ACCENT, 1.0), FillRule::Winding, id, None);
    }
    pm
}

/// A ring round each rect (cells, relative to the image, which is one cell larger on every
/// side): a faint fill, a glow and an anti-aliased outline.
pub fn raster_ring(cols: u16, rows: u16, rects: &[Rect], cell: CellPx) -> Pixmap {
    let (cw, ch) = (cell.w as f32, cell.h as f32);
    let mut pm = Pixmap::new(cols as u32 * cell.w as u32, rows as u32 * cell.h as u32).expect("a ring size");
    let id = Transform::identity();
    let w = (ch / 14.0).max(1.2);
    for r in rects {
        let (x, y) = (r.x as f32 * cw - cw * 0.35, r.y as f32 * ch + ch * 0.04);
        let (pw, ph) = (r.w as f32 * cw + cw * 0.7, ch * 0.92);
        let Some(path) = rrect(x, y, pw, ph, ch * 0.25) else { continue };
        pm.fill_path(&path, &paint(ACCENT, 0.10), FillRule::Winding, id, None);
        for k in (1..=3).rev() {
            pm.stroke_path(&path, &paint(ACCENT, 0.08), &stroke(w + k as f32 * w * 1.6), id, None);
        }
        pm.stroke_path(&path, &paint(ACCENT, 1.0), &stroke(w), id, None);
    }
    pm
}

/// The spotlight's veil at `per` pixels per cell: translucent dark everywhere but the holes
/// (cells, relative to the image), which are rounded and feathered.
pub fn raster_veil(cols: u16, rows: u16, holes: &[Rect], per: (u32, u32)) -> Pixmap {
    let (vx, vy) = (per.0 as f32, per.1 as f32);
    let (w, h) = (cols as u32 * per.0, rows as u32 * per.1);
    let mut pm = Pixmap::new(w.max(1), h.max(1)).expect("a veil size");
    let shade = |a: f32| [(6.0 * a) as u8, (8.0 * a) as u8, (14.0 * a) as u8, (a * 255.0) as u8];
    let full = shade(0.58);
    for c in pm.data_mut().as_chunks_mut::<4>().0 {
        *c = full;
    }
    let (feather, rad) = (vy * 1.0, vy * 0.6);
    let hs: Vec<(f32, f32, f32, f32)> = holes
        .iter()
        .map(|r| {
            let (x0, y0) = (r.x as f32 * vx, r.y as f32 * vy);
            (x0 - vx * 0.2, y0 - vy * 0.1, x0 + r.w as f32 * vx + vx * 0.2, y0 + r.h as f32 * vy + vy * 0.1)
        })
        .collect();
    let data = pm.data_mut();
    for r in &hs {
        let x0 = ((r.0 - feather).floor().max(0.0)) as u32;
        let y0 = ((r.1 - feather).floor().max(0.0)) as u32;
        let x1 = ((r.2 + feather).ceil().max(0.0) as u32).min(w);
        let y1 = ((r.3 + feather).ceil().max(0.0) as u32).min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let d = hs.iter().map(|&r| sd_rrect(fx, fy, r, rad)).fold(f32::MAX, f32::min);
                let i = ((y * w + x) * 4) as usize;
                data[i..i + 4].copy_from_slice(&shade(0.58 * smoothstep(-feather * 0.3, feather, d)));
            }
        }
    }
    pm
}

/// Rasters already made, by shape key, so toggling a part back on doesn't re-rasterise.
#[derive(Default)]
pub struct Rasters {
    map: HashMap<u64, Image>,
    /// How many rasters were made (for measuring).
    pub made: u64,
}

impl Rasters {
    fn get(&mut self, key: u64, make: impl FnOnce() -> Pixmap) -> Image {
        if let Some(i) = self.map.get(&key) {
            return i.clone();
        }
        if self.map.len() > 32 {
            self.map.clear();
        }
        let img = straight(&make());
        self.made += 1;
        self.map.insert(key, img.clone());
        img
    }
}

fn le(v: &[i64]) -> Vec<u8> {
    v.iter().flat_map(|n| n.to_le_bytes()).collect()
}

/// The pictures for a plan in pixels: per layer a panel under its words, then over the text
/// its veil, arrow and ring. Pixels are rasterised only for shape keys the terminal doesn't
/// hold. `area` is the text area (the veil stays inside it).
pub fn pictures(plan: &Plan, cell: CellPx, area: Rect, kitty: &KittyState, rasters: &mut Rasters) -> Vec<Picture> {
    let mut out = Vec::new();
    let cpx = le(&[cell.w as i64, cell.h as i64]);
    let mut want = |layer: &str, part: &str, key: u64, z: Z, at: Cells, clip: Option<Rect>, make: &dyn Fn() -> Pixmap| {
        let image = (!kitty.holds(key, cell)).then(|| rasters.get(key, make));
        out.push(Picture { layer: layer.into(), part: part.into(), key, z, at, clip, image });
    };
    for l in &plan.layers {
        let id = l.id.as_str();
        if let (Some(r), Some(Mode::Box)) = (l.rect, l.mode) {
            let key = shape_key(&[RASTER, b"panel", &cpx, &le(&[r.w as i64, r.h as i64])]);
            let at = Cells::new(r.x as i32 - 1, r.y as i32, r.w + 2, r.h + 1);
            want(id, "panel", key, Z::Below, at, None, &|| raster_panel(r.w, r.h, cell));
        }
        if let Some(s) = plan.spots.iter().find(|s| s.layer == l.id) {
            let a = s.area.intersection(&area);
            if !a.is_empty() {
                // A canvas three times the area's height, with the holes a third of the way
                // down: scrolling moves it and re-crops it, the pixels unchanged.
                let top = s.holes.iter().map(|h| h.y).min().unwrap_or(a.y) as i32;
                let rows = a.h as i32;
                let y0 = top - rows;
                let holes: Vec<Rect> = s
                    .holes
                    .iter()
                    .map(|h| Rect::new(h.x.saturating_sub(a.x), (h.y as i32 - y0) as u16, h.w, h.h))
                    .collect();
                let per = ((cell.w as u32 / 4).max(1), (cell.h as u32 / 4).max(1));
                let mut k = vec![a.w as i64, rows as i64, per.0 as i64, per.1 as i64];
                for h in &holes {
                    k.extend([h.x as i64, h.y as i64, h.w as i64, h.h as i64]);
                }
                let key = shape_key(&[RASTER, b"veil", &le(&k)]);
                let (w, h3) = (a.w, (rows * 3) as u16);
                let at = Cells::new(a.x as i32, y0, w, h3);
                want(id, "veil", key, Z::Above, at, Some(a), &|| raster_veil(w, h3, &holes, per));
            }
        }
        if let Some(rt) = &l.route
            && let Some(last) = rt.steps.last()
        {
            let (jx, jy) = rt.junction;
            let xs = rt.steps.iter().map(|s| s.x).chain([jx]);
            let ys = rt.steps.iter().map(|s| s.y).chain([jy]);
            let (x0, x1) = (xs.clone().min().unwrap_or(jx), xs.max().unwrap_or(jx));
            let (y0, y1) = (ys.clone().min().unwrap_or(jy), ys.max().unwrap_or(jy));
            let (ox, oy) = (x0 as i32 - 1, y0 as i32 - 1);
            let (cols, rows) = (x1 - x0 + 3, y1 - y0 + 3);
            let rel = |x: u16, y: u16| ((x as i32 - ox) as f32 + 0.5, (y as i32 - oy) as f32 + 0.5);
            // Corners only: the junction, every bend, the head.
            let mut pts = vec![rel(jx, jy)];
            for (i, s) in rt.steps.iter().enumerate() {
                let bend = s.enter != s.leave || i + 1 == rt.steps.len();
                if bend {
                    pts.push(rel(s.x, s.y));
                }
            }
            let h = rel(last.x, last.y);
            let tip = match last.leave {
                Dir::Up => (h.0, h.1 - 0.45),
                Dir::Down => (h.0, h.1 + 0.45),
                Dir::Left => (h.0 - 0.45, h.1),
                Dir::Right => (h.0 + 0.45, h.1),
            };
            let mut k = vec![cols as i64, rows as i64];
            for p in pts.iter().chain([&tip]) {
                k.extend([(p.0 * 100.0) as i64, (p.1 * 100.0) as i64]);
            }
            let key = shape_key(&[RASTER, b"arrow", &cpx, &le(&k)]);
            let at = Cells::new(ox, oy, cols, rows);
            want(id, "arrow", key, Z::Above, at, None, &|| raster_arrow(cols, rows, &pts, tip, cell));
        }
        if !l.ring.is_empty() {
            let b = Rect::bounds(&l.ring);
            let (ox, oy) = (b.x as i32 - 1, b.y as i32 - 1);
            let rel: Vec<Rect> = l.ring.iter().map(|r| Rect::new((r.x as i32 - ox) as u16, (r.y as i32 - oy) as u16, r.w, r.h)).collect();
            let (cols, rows) = (b.w + 2, b.h + 2);
            let mut k = vec![cols as i64, rows as i64];
            for r in &rel {
                k.extend([r.x as i64, r.y as i64, r.w as i64, r.h as i64]);
            }
            let key = shape_key(&[RASTER, b"ring", &cpx, &le(&k)]);
            let at = Cells::new(ox, oy, cols, rows);
            want(id, "ring", key, Z::Above, at, None, &|| raster_ring(cols, rows, &rel, cell));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CELL: CellPx = CellPx::new(8, 16);

    fn golden_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens")
    }

    /// Compares a raster with its PNG golden: byte-exact pixels where the golden was made
    /// (macOS arm64), within 2 per channel elsewhere (tiny-skia's SIMD paths may round
    /// differently).
    fn png_golden(name: &str, pm: &Pixmap) {
        let path = golden_dir().join(name);
        if std::env::var("CARETLINE_GOLDENS").as_deref() == Ok("update") {
            std::fs::write(&path, pm.encode_png().unwrap()).unwrap();
            return;
        }
        let want = Pixmap::decode_png(&std::fs::read(&path).unwrap_or_else(|_| panic!("no golden {name}: run with CARETLINE_GOLDENS=update")))
            .unwrap();
        assert_eq!((want.width(), want.height()), (pm.width(), pm.height()), "{name}");
        let worst = want.data().iter().zip(pm.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert!(worst <= 2, "{name}: a channel differs by {worst}");
    }

    #[test]
    fn png_goldens_at_a_pinned_cell_size() {
        png_golden("raster.panel.png", &raster_panel(20, 5, CELL));
        png_golden("raster.arrow.png", &raster_arrow(6, 5, &[(4.5, 1.5), (4.5, 2.5), (1.5, 2.5)], (1.5, 3.95), CELL));
        png_golden("raster.ring.png", &raster_ring(7, 3, &[Rect::new(1, 1, 5, 1)], CELL));
        png_golden("raster.veil.png", &raster_veil(30, 18, &[Rect::new(4, 7, 5, 1), Rect::new(2, 9, 20, 4)], (2, 4)));
    }

    #[test]
    fn rasters_are_the_same_twice() {
        assert_eq!(raster_panel(10, 4, CELL).data(), raster_panel(10, 4, CELL).data());
        assert_eq!(raster_veil(10, 9, &[Rect::new(1, 4, 3, 1)], (2, 4)).data(), raster_veil(10, 9, &[Rect::new(1, 4, 3, 1)], (2, 4)).data());
    }

    #[test]
    fn measure_is_pure_and_wraps() {
        let d = json!({"title": "Jump by word", "text": "⌥← and ⌥→ move one word at a time, and ↑ ↓ by rows."});
        let a = HintRenderer.measure(&d, Size::new(80, 20));
        assert_eq!(a, HintRenderer.measure(&d, Size::new(80, 20)));
        assert!(a.w <= TEXT_COLS as u16 + 4 && a.h >= 4, "{a:?}");
        let narrow = HintRenderer.measure(&d, Size::new(24, 20));
        assert!(narrow.w <= 24 && narrow.h > a.h, "{narrow:?}");
        assert_eq!(HintRenderer.measure(&d, Size::new(10, 20)), Size::new(0, 0));
    }
}
