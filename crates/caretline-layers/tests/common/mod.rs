//! Shared helpers: a generic document rendered by caretline, a test host that measures and
//! draws `hint` boxes in plain text (hosts draw; the crate doesn't), and golden files.

#![allow(dead_code)]

use caretline::view::Frame;
use caretline::{State, Viewport, view};
use caretline_layers::*;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

/// A generic help page, long enough to scroll at 24 rows.
pub const DOC: &str = "# Getting around

This page is a short tour of the editor. Read it from top to bottom and
try each key as you go: nothing here can break.

## Moving by words

Hold Option and press an arrow key to jump a word at a time. The caret
stops at the start of each word, so a long line is quick to cross.

## Selecting

Hold Shift with any motion to select as you move. Shift and Option
together select whole words, and the selection grows from where you began.

## Undo

Every change can be undone. Undo steps follow the way you typed: a word
at a time while you type, and one step for a paste.

## Search

Press Control F to search. Matches light up as you type, and Enter jumps
to the next one.

## Saving

Your work is saved when you press Control S. The status bar shows a mark
while there are changes to save.
";

pub fn state(w: u16, h: u16) -> State {
    State::new(
        DOC,
        Some("guide.md".into()),
        Viewport {
            width: w,
            height: h,
        },
    )
}

/// The char range of the `n`th (from 0) occurrence of `needle`.
pub fn find(needle: &str, n: usize) -> Anchor {
    let (b, _) = DOC
        .match_indices(needle)
        .nth(n)
        .unwrap_or_else(|| panic!("{needle:?} not in the page"));
    let from = DOC[..b].chars().count();
    Anchor::Text {
        from,
        to: from + needle.chars().count(),
    }
}

/// Words of `s` wrapped to `w` columns (ASCII test text).
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in s.split_whitespace() {
        let cur = lines.last_mut().unwrap();
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > w {
            lines.push(word.to_string());
        } else {
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
    }
    lines
}

/// The test host's `hint` renderer: a bordered box, one cell of padding, the title then the
/// wrapped text.
pub fn hint_size(data: &Value, avail: Size) -> Size {
    let h: Hint = serde_json::from_value(data.clone()).unwrap_or_default();
    let inner = avail.w.saturating_sub(4).max(8) as usize;
    let mut lines = wrap(&h.text, inner);
    if let Some(t) = &h.title {
        lines.insert(0, t.clone());
    }
    let w = lines
        .iter()
        .map(|l| str_width(l))
        .max()
        .unwrap_or(0)
        .min(inner);
    Size::new(w as u16 + 4, lines.len() as u16 + 2)
}

pub fn renderers() -> Renderers {
    Renderers::new().register(HINT, hint_size)
}

fn str_width(s: &str) -> usize {
    s.graphemes(true).map(width).sum()
}

pub fn hint(title: &str, text: &str) -> Content {
    Content::hint(Some(title), text)
}

pub fn push(layers: &mut Layers, layer: Layer, actor: Option<&str>) -> String {
    apply(
        layers,
        LayerOp::Push(layer),
        actor,
        1_000,
        &Limits::default(),
    )
    .expect("push")
    .layer
    .unwrap()
}

/// Plans `layers` over the state's frame.
pub fn plan_over(s: &State, layers: &Layers) -> (Frame, Plan) {
    let frame = view(s);
    let res = FrameResolver::new(&frame).with_doc(&s.doc);
    let grid = Grid::from_frame(&frame);
    let p = plan(layers, &res, &grid, &renderers());
    (frame, p)
}

/// The test host's drawing: the frame's text with each plan drawn over it in ASCII, and a map
/// of what each cell is: `#` a box, `=` a strip, `c` a chip, `>` an arrow, `*` a ringed cell,
/// `.` a dimmed one, `s` the status row.
pub fn picture(frame: &Frame, p: &Plan, layers: &Layers) -> String {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut text: Vec<Vec<String>> = (0..h)
        .map(|y| {
            (0..w)
                .map(|x| frame.cell(x as u16, y as u16).symbol.to_string())
                .collect()
        })
        .collect();
    let mut map: Vec<Vec<char>> = (0..h)
        .map(|y| {
            (0..w)
                .map(|x| {
                    if matches!(frame.rows[y], caretline::view::RowInfo::Status) {
                        's'
                    } else if p.dimmed(x as u16, y as u16) {
                        '.'
                    } else {
                        ' '
                    }
                })
                .collect()
        })
        .collect();
    let put = |x: u16,
               y: u16,
               s: &str,
               m: char,
               text: &mut Vec<Vec<String>>,
               map: &mut Vec<Vec<char>>| {
        let (x, y) = (x as usize, y as usize);
        if y < h && x < w {
            // Never leave half a wide grapheme behind.
            if text[y][x].is_empty() && x > 0 {
                text[y][x - 1] = " ".into();
            }
            if x + 1 < w && text[y][x + 1].is_empty() {
                text[y][x + 1] = " ".into();
            }
            text[y][x] = s.into();
            map[y][x] = m;
        }
    };
    for l in &p.layers {
        for r in &l.ring {
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    if map[y as usize][x as usize] != 's' {
                        map[y as usize][x as usize] = '*';
                    }
                }
            }
        }
    }
    for l in &p.layers {
        let layer = layers.get(&l.id).unwrap();
        let hnt = layer
            .content
            .as_ref()
            .and_then(Content::as_hint)
            .unwrap_or_default();
        if let Some(rt) = &l.route {
            let n = rt.steps.len();
            for (k, s) in rt.steps.iter().enumerate() {
                let g = if k + 1 == n {
                    match s.leave {
                        Dir::Up => "^",
                        Dir::Down => "v",
                        Dir::Left => "<",
                        Dir::Right => ">",
                    }
                } else if s.enter != s.leave {
                    "+"
                } else if matches!(s.enter, Dir::Up | Dir::Down) {
                    "|"
                } else {
                    "-"
                };
                put(s.x, s.y, g, '>', &mut text, &mut map);
            }
        }
        if let Some(c) = l.chip {
            let label = match l.anchor.as_ref().and_then(|a| a.off) {
                Some(Off::Above { .. }) => "^ here",
                Some(Off::Below { .. }) => "v here",
                Some(Off::Left { .. }) => "< here",
                _ => "> here",
            };
            for x in c.x..c.right() {
                put(x, c.y, " ", 'c', &mut text, &mut map);
            }
            for (k, ch) in label.chars().enumerate().take(c.w as usize) {
                put(
                    c.x + k as u16,
                    c.y,
                    &ch.to_string(),
                    'c',
                    &mut text,
                    &mut map,
                );
            }
        }
        let Some(r) = l.rect else { continue };
        match l.mode {
            Some(Mode::Strip) => {
                for x in r.x..r.right() {
                    put(x, r.y, " ", '=', &mut text, &mut map);
                }
                let s = format!(" {} {}", hnt.title.clone().unwrap_or_default(), hnt.text);
                for (k, ch) in s.chars().enumerate().take(r.w as usize) {
                    put(
                        r.x + k as u16,
                        r.y,
                        &ch.to_string(),
                        '=',
                        &mut text,
                        &mut map,
                    );
                }
            }
            _ => {
                for y in r.y..r.bottom() {
                    for x in r.x..r.right() {
                        let edge_y = y == r.y || y + 1 == r.bottom();
                        let edge_x = x == r.x || x + 1 == r.right();
                        let g = match (edge_x, edge_y) {
                            (true, true) => "+",
                            (false, true) => "-",
                            (true, false) => "|",
                            _ => " ",
                        };
                        put(x, y, g, '#', &mut text, &mut map);
                    }
                }
                if let Some((jx, jy)) = l.route.as_ref().map(|r| r.junction) {
                    put(jx, jy, "+", '#', &mut text, &mut map);
                }
                let inner = r.w.saturating_sub(4) as usize;
                let mut lines = wrap(&hnt.text, inner);
                if let Some(t) = &hnt.title {
                    lines.insert(0, t.clone());
                }
                for (k, line) in lines
                    .iter()
                    .enumerate()
                    .take(r.h.saturating_sub(2) as usize)
                {
                    let mut x = r.x + 2;
                    for g in line.graphemes(true) {
                        let gw = width(g) as u16;
                        if x + gw > r.right() - 2 {
                            break;
                        }
                        put(x, r.y + 1 + k as u16, g, '#', &mut text, &mut map);
                        if gw == 2 {
                            put(x + 1, r.y + 1 + k as u16, "", '#', &mut text, &mut map);
                        }
                        x += gw;
                    }
                }
            }
        }
    }
    let mut out = String::new();
    for row in &text {
        out.push_str(row.concat().trim_end());
        out.push('\n');
    }
    out.push_str("--- map\n");
    for row in &map {
        out.push_str(row.iter().collect::<String>().trim_end());
        out.push('\n');
    }
    out
}

fn path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name)
}

/// Compares `actual` with the golden file; `LAYERS_GOLDENS=update` rewrites it instead.
pub fn golden(name: &str, actual: &str) {
    let p = path(name);
    if std::env::var("LAYERS_GOLDENS").as_deref() == Ok("update") {
        std::fs::write(&p, actual).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&p).unwrap_or_else(|_| {
        panic!("no golden {name}: run with LAYERS_GOLDENS=update and review it")
    });
    assert!(
        want == actual,
        "golden {name} differs.\n--- want\n{want}\n--- got\n{actual}"
    );
}
