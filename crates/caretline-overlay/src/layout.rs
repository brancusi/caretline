//! Layout: layers and resolved anchors in, a [`Scene`] out. Pure, and all geometry is in cell
//! coordinates, so a cell renderer ([`compose`](crate::compose)) and a later pixel renderer
//! draw the same thing.

use serde::{Deserialize, Serialize};

use crate::geom::{Rect, Side};
use crate::model::{Item, Layer, Layers, Pulse};
use crate::place::{Grid, MAX_WIDTH, Measure, Mode, Region, Spot, plan};
use crate::resolve::{Off, Resolve, Resolved};
use crate::route;
use crate::text::{self, KeyLabels, NoKeys, Piece, Tok};

/// The glyph set. `Ascii` is for terminals that draw box drawing or ambiguous-width symbols
/// wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Glyphs {
    #[default]
    Rounded,
    Square,
    Ascii,
}

impl Glyphs {
    /// Corners and edges: top-left, top-right, bottom-left, bottom-right, horizontal, vertical.
    pub fn frame(self) -> [&'static str; 6] {
        match self {
            Glyphs::Rounded => ["╭", "╮", "╰", "╯", "─", "│"],
            Glyphs::Square => ["┌", "┐", "└", "┘", "─", "│"],
            Glyphs::Ascii => ["+", "+", "+", "+", "-", "|"],
        }
    }

    fn dots(self) -> (&'static str, &'static str) {
        match self {
            Glyphs::Ascii => ("*", "-"),
            _ => ("●", "○"),
        }
    }

    fn pointer(self, off: &Off) -> &'static str {
        match (off, self == Glyphs::Ascii) {
            (Off::Above { .. }, false) => "↑",
            (Off::Below { .. }, false) => "↓",
            (Off::Left { .. }, false) => "◀",
            (Off::Right { .. }, false) => "▶",
            (Off::Above { .. }, true) => "^",
            (Off::Below { .. }, true) => "v",
            (Off::Left { .. }, true) => "<",
            (Off::Right { .. }, true) => ">",
        }
    }

    fn sep(self) -> &'static str {
        match self {
            Glyphs::Ascii => " - ",
            _ => " · ",
        }
    }
}

/// How layout draws: the glyph set and the key labels for `{{key:…}}`.
pub struct Opts<'a> {
    pub glyphs: Glyphs,
    pub keys: &'a dyn KeyLabels,
}

impl Opts<'static> {
    pub fn new(glyphs: Glyphs) -> Opts<'static> {
        Opts {
            glyphs,
            keys: &NoKeys,
        }
    }
}

impl Default for Opts<'static> {
    fn default() -> Self {
        Opts::new(Glyphs::Rounded)
    }
}

/// A run of text in one role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    pub role: String,
}

/// Spans drawn from (`x`, `y`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub x: u16,
    pub y: u16,
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelKind {
    /// A bordered box.
    Callout,
    /// One row across the area (narrow mode).
    Strip,
    /// A small tag at the edge an off-screen anchor lies beyond.
    Chip,
}

/// A box, strip or chip: its cells are filled in `fill`, a callout's border drawn in `border`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    pub kind: PanelKind,
    pub rect: Rect,
    pub fill: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    /// The side of the anchor it's on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    /// Where an arrow leaves the border, and its glyph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub junction: Option<(u16, u16, String)>,
    pub runs: Vec<Run>,
}

/// One cell of an arrow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteCell {
    pub x: u16,
    pub y: u16,
    pub g: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arrowed {
    pub role: String,
    /// From the callout to the head.
    pub cells: Vec<RouteCell>,
}

/// A layer laid out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub id: String,
    pub z: i16,
    pub agent: bool,
    /// Where its anchor resolved (`None` for a screen-placed layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Resolved>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrow: Option<Arrowed>,
    pub panels: Vec<Panel>,
}

/// A ring around an anchor's cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RingMark {
    pub layer: String,
    pub agent: bool,
    pub rects: Vec<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse: Option<Pulse>,
}

/// Everything the overlays draw this frame, in cell coordinates. Serializable: a pixel
/// renderer, a test or another process can draw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scene {
    pub width: u16,
    pub height: u16,
    pub glyphs: Glyphs,
    pub spots: Vec<Spot>,
    pub rings: Vec<RingMark>,
    /// In draw order (z, then push order).
    pub layers: Vec<Placed>,
    pub regions: Vec<Region>,
    /// Layers none of whose anchors resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

impl Scene {
    pub fn is_empty(&self) -> bool {
        self.spots.is_empty() && self.rings.is_empty() && self.layers.is_empty()
    }
}

/// The topmost region at a cell.
pub fn hit(scene: &Scene, x: u16, y: u16) -> Option<&Region> {
    scene.regions.iter().rev().find(|r| r.rect.contains(x, y))
}

/// The full name of an overlay role: `overlay.<base>`, or `overlay.agent.<base>` for an agent's.
pub fn role(base: &str, agent: bool) -> String {
    if agent {
        format!("overlay.agent.{base}")
    } else {
        format!("overlay.{base}")
    }
}

/// Below this many columns a strip drops its dots.
const DOTS_COLS: u16 = 24;
/// A callout's content, measured.
struct Content {
    /// Lines of pieces; the first is the title line when there's a title or steps.
    lines: Vec<Vec<Piece>>,
    /// For each chip on the chips line: its id and its columns within the line.
    chips: Vec<(String, u16, u16)>,
    chips_line: Option<usize>,
    width: u16,
}

fn pieces(s: &str, role: &'static str) -> Vec<Piece> {
    use unicode_segmentation::UnicodeSegmentation;
    s.graphemes(true)
        .map(|g| {
            let p = text::printable(g);
            Piece {
                g: p.to_string(),
                w: text::width(p) as u16,
                role,
            }
        })
        .collect()
}

fn dots(of: u16, at: u16, glyphs: Glyphs) -> Vec<Piece> {
    let (on, off) = glyphs.dots();
    let mut v = pieces(&format!("{at} of {of}  "), "dots");
    for i in 1..=of.min(24) {
        v.extend(pieces(
            if i <= at { on } else { off },
            if i <= at { "dots.on" } else { "dots" },
        ));
    }
    v
}

fn content(layer: &Layer, inner: u16, opts: &Opts) -> Content {
    let c = layer.callout().cloned().unwrap_or_default();
    let steps = layer.items.iter().find_map(|i| match i {
        Item::Steps { of, at } => Some((*of, *at)),
        _ => None,
    });
    let mut lines: Vec<Vec<Piece>> = Vec::new();
    if c.title.is_some() || steps.is_some() {
        let d = steps
            .map(|(of, at)| dots(of, at, opts.glyphs))
            .unwrap_or_default();
        let dw = text::line_width(&d);
        let room = inner.saturating_sub(if dw > 0 { dw + 2 } else { 0 }) as usize;
        let mut line = pieces(
            &text::truncate(c.title.as_deref().unwrap_or(""), room, opts.glyphs),
            "callout.title",
        );
        if dw > 0 {
            let tw = text::line_width(&line);
            let gap = inner.saturating_sub(tw + dw).max(2);
            line.extend(std::iter::repeat_n(
                Piece {
                    g: " ".into(),
                    w: 1,
                    role: "callout",
                },
                gap as usize,
            ));
            line.extend(d);
        }
        lines.push(line);
    }
    if !c.body.is_empty() {
        lines.extend(text::wrap(
            &text::tokens(&c.body, "callout", opts.keys, opts.glyphs),
            inner,
        ));
    }
    for i in &layer.items {
        if let Item::Keys(ids) = i {
            let mut toks = Vec::new();
            for (k, id) in ids.iter().enumerate() {
                if k > 0 {
                    toks.push(Tok::Space);
                }
                toks.push(Tok::Badge(text::badge(id, opts.keys, opts.glyphs)));
            }
            lines.extend(text::wrap(&toks, inner));
        }
    }
    let mut chips = Vec::new();
    let mut chips_line = None;
    if !c.chips.is_empty() {
        let mut line: Vec<Piece> = Vec::new();
        for (k, ch) in c.chips.iter().enumerate() {
            if k > 0 {
                line.extend(pieces(opts.glyphs.sep(), "callout"));
            }
            let x0 = text::line_width(&line);
            for t in text::tokens(&ch.text, "chip", opts.keys, opts.glyphs) {
                match t {
                    Tok::Word(ps) | Tok::Badge(ps) => line.extend(ps),
                    Tok::Space => line.push(Piece {
                        g: " ".into(),
                        w: 1,
                        role: "chip",
                    }),
                    Tok::Break => {}
                }
            }
            chips.push((ch.id.clone(), x0, text::line_width(&line)));
        }
        // Chips that don't fit are cut, never wrapped: each must stay one click target.
        let mut w = 0;
        line.retain(|p| {
            w += p.w;
            w <= inner
        });
        chips.retain(|c| c.2 <= inner);
        chips_line = Some(lines.len());
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    let width = lines.iter().map(|l| text::line_width(l)).max().unwrap_or(0);
    Content {
        lines,
        chips,
        chips_line,
        width,
    }
}

fn spans(line: &[Piece], agent: bool) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for p in line {
        let r = role(p.role, agent);
        match out.last_mut() {
            Some(s) if s.role == r => s.text.push_str(&p.g),
            _ => out.push(Span {
                text: p.g.clone(),
                role: r,
            }),
        }
    }
    out
}

/// Sizes the built-in text content (callout, key badges, step dots) for [`plan`].
pub struct TextMeasure<'a, 'b>(pub &'a Opts<'b>);

/// The chip label for an anchor that lies `off` screen: ` ↓ 2/11 here `.
fn chip_label(layer: &Layer, off: &Off, glyphs: Glyphs) -> String {
    let steps = layer.items.iter().find_map(|i| match i {
        Item::Steps { of, at } => Some(format!("{at}/{of} ")),
        _ => None,
    });
    format!(
        " {} {}here ",
        glyphs.pointer(off),
        steps.unwrap_or_default()
    )
}

fn inner(max_w: u16) -> u16 {
    max_w.saturating_sub(4).max(8)
}

impl Measure for TextMeasure<'_, '_> {
    fn size(&self, layer: &Layer, max: (u16, u16)) -> Option<(u16, u16)> {
        let ct = content(layer, inner(max.0), self.0);
        Some((ct.width.max(8) + 4, ct.lines.len() as u16 + 2))
    }

    fn chip(&self, layer: &Layer, off: &Off) -> u16 {
        text::str_width(&chip_label(layer, off, self.0.glyphs)) as u16
    }
}

/// Lays out every visible layer with the built-in text content: [`plan`] for the geometry,
/// then the text, glyphs and roles. Pure: the same layers, anchors and grid give the same scene.
pub fn layout(layers: &Layers, anchors: &dyn Resolve, grid: &Grid, opts: &Opts) -> Scene {
    let plan = plan(layers, anchors, grid, &TextMeasure(opts));
    let mut scene = Scene {
        width: grid.width,
        height: grid.height,
        glyphs: opts.glyphs,
        spots: plan.spots,
        rings: Vec::new(),
        layers: Vec::new(),
        regions: Vec::new(),
        missing: plan.missing,
    };
    let ascii = opts.glyphs == Glyphs::Ascii;
    for p in plan.layers {
        let Some(layer) = layers.get(&p.id) else {
            continue;
        };
        let agent = p.agent;
        let mut placed = Placed {
            id: p.id.clone(),
            z: p.z,
            agent,
            anchor: p.anchor.clone(),
            arrow: None,
            panels: Vec::new(),
        };
        if !p.ring.is_empty() {
            scene.rings.push(RingMark {
                layer: p.id.clone(),
                agent,
                rects: p.ring.clone(),
                pulse: p.pulse,
            });
        }
        if let (Some(r), Some(off)) = (p.chip, p.anchor.as_ref().and_then(|a| a.off)) {
            scene.regions.push(Region {
                rect: r,
                id: format!("{}/reveal", p.id),
            });
            placed.panels.push(Panel {
                kind: PanelKind::Chip,
                rect: r,
                fill: role("chip", agent),
                border: None,
                side: None,
                junction: None,
                runs: vec![Run {
                    x: r.x,
                    y: r.y,
                    spans: vec![Span {
                        text: chip_label(layer, &off, opts.glyphs),
                        role: role("chip", agent),
                    }],
                }],
            });
        }
        if let Some(rt) = &p.route {
            let n = rt.steps.len();
            placed.arrow = Some(Arrowed {
                role: role("arrow", agent),
                cells: rt
                    .steps
                    .iter()
                    .enumerate()
                    .map(|(k, s)| RouteCell {
                        x: s.x,
                        y: s.y,
                        g: route::glyph(s.enter, s.leave, k + 1 == n, ascii).to_string(),
                    })
                    .collect(),
            });
        }
        match (p.rect, p.mode) {
            (Some(r), Some(Mode::Box)) => {
                let max_w = layer
                    .callout()
                    .and_then(|c| c.max_width)
                    .unwrap_or(MAX_WIDTH)
                    .min(grid.area.w * 2 / 3);
                let ct = content(layer, inner(max_w), opts);
                let mut panel = callout_panel(r, p.side, &ct, agent, &p.id, &mut scene);
                if let (Some(rt), Some(side)) = (&p.route, p.side) {
                    panel.junction = Some((
                        rt.junction.0,
                        rt.junction.1,
                        route::junction(side, ascii).to_string(),
                    ));
                }
                placed.panels.push(panel);
            }
            (Some(r), Some(Mode::Strip)) => {
                scene.regions.push(Region {
                    rect: r,
                    id: p.id.clone(),
                });
                let off = p.anchor.as_ref().and_then(|a| a.off);
                placed
                    .panels
                    .push(strip(layer, r, off.as_ref(), grid, opts, agent));
            }
            _ => {}
        }
        if !placed.panels.is_empty() || placed.arrow.is_some() {
            scene.layers.push(placed);
        }
    }
    scene
}

fn callout_panel(
    r: Rect,
    side: Option<Side>,
    ct: &Content,
    agent: bool,
    id: &str,
    scene: &mut Scene,
) -> Panel {
    // The callout's region goes under its chips' (hit takes the topmost).
    scene.regions.push(Region {
        rect: r,
        id: id.to_string(),
    });
    let mut runs = Vec::new();
    for (k, line) in ct.lines.iter().enumerate() {
        let y = r.y + 1 + k as u16;
        if y + 1 >= r.bottom() {
            break;
        }
        if !line.is_empty() {
            runs.push(Run {
                x: r.x + 2,
                y,
                spans: spans(line, agent),
            });
        }
        if ct.chips_line == Some(k) {
            for (cid, x0, x1) in &ct.chips {
                let cx = r.x + 2 + x0;
                let w = (x1 - x0).min(r.right().saturating_sub(2).saturating_sub(cx));
                if w > 0 {
                    scene.regions.push(Region {
                        rect: Rect::new(cx, y, w, 1),
                        id: format!("{id}/chip/{cid}"),
                    });
                }
            }
        }
    }
    Panel {
        kind: PanelKind::Callout,
        rect: r,
        fill: role("callout", agent),
        border: Some(role("callout.border", agent)),
        side,
        junction: None,
        runs,
    }
}

/// The text of a layer for one row: title, or else the body's first line.
fn short_text(layer: &Layer, opts: &Opts) -> Vec<Piece> {
    let c = layer.callout().cloned().unwrap_or_default();
    let src = match c.title.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => return pieces(t, "strip"),
        None => c.body.lines().next().unwrap_or("").to_string(),
    };
    let mut out = Vec::new();
    for t in text::tokens(&src, "strip", opts.keys, opts.glyphs) {
        match t {
            Tok::Word(ps) | Tok::Badge(ps) => out.extend(ps),
            Tok::Space => out.push(Piece {
                g: " ".into(),
                w: 1,
                role: "strip",
            }),
            Tok::Break => break,
        }
    }
    out
}

/// One row across the area: steps, the text, where an off-screen anchor is, the chips.
fn strip(
    layer: &Layer,
    rect: Rect,
    off: Option<&Off>,
    grid: &Grid,
    opts: &Opts,
    agent: bool,
) -> Panel {
    let area = grid.area;
    let mut line: Vec<Piece> = vec![Piece {
        g: " ".into(),
        w: 1,
        role: "strip",
    }];
    if let Some((of, at)) = layer.items.iter().find_map(|i| match i {
        Item::Steps { of, at } => Some((*of, *at)),
        _ => None,
    }) {
        line.extend(pieces(&format!("{at}/{of} "), "strip"));
        if area.w >= DOTS_COLS && off.is_none() {
            let (on, offg) = opts.glyphs.dots();
            for i in 1..=of.min(24) {
                line.extend(pieces(
                    if i <= at { on } else { offg },
                    if i <= at { "dots.on" } else { "dots" },
                ));
            }
            line.push(Piece {
                g: " ".into(),
                w: 1,
                role: "strip",
            });
        }
    }
    line.extend(short_text(layer, opts));
    if let Some(o) = off {
        line.extend(pieces(opts.glyphs.sep(), "strip"));
        let word = match o {
            Off::Above { .. } => "above",
            Off::Below { .. } => "below",
            Off::Left { .. } => "left",
            Off::Right { .. } => "right",
        };
        line.extend(pieces(
            &format!("{} {word}", opts.glyphs.pointer(o)),
            "strip",
        ));
    }
    if let Some(c) = layer.callout() {
        for ch in &c.chips {
            line.extend(pieces(opts.glyphs.sep(), "strip"));
            for t in text::tokens(&ch.text, "strip", opts.keys, opts.glyphs) {
                match t {
                    Tok::Word(ps) | Tok::Badge(ps) => line.extend(ps),
                    Tok::Space => line.push(Piece {
                        g: " ".into(),
                        w: 1,
                        role: "strip",
                    }),
                    Tok::Break => {}
                }
            }
        }
    }
    let room = area.w.saturating_sub(1);
    if text::line_width(&line) > room {
        let ell = if opts.glyphs == Glyphs::Ascii {
            "..."
        } else {
            "…"
        };
        let ew = text::str_width(ell) as u16;
        let mut w = 0;
        line.retain(|p| {
            w += p.w;
            w + ew <= room
        });
        line.extend(pieces(ell, "strip"));
    }
    Panel {
        kind: PanelKind::Strip,
        rect,
        fill: role("strip", agent),
        border: None,
        side: None,
        junction: None,
        runs: vec![Run {
            x: rect.x,
            y: rect.y,
            spans: spans(&line, agent),
        }],
    }
}
