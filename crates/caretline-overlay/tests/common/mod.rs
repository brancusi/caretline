//! Shared helpers: a generic document rendered by caretline, layers placed over it, and the
//! golden files.

#![allow(dead_code)]

use std::collections::BTreeMap;

use caretline::{Msg, State, Viewport, view};
use caretline_overlay::*;

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

pub fn keys() -> BTreeMap<String, String> {
    [
        ("move.word_left", "⌥←"),
        ("move.word_right", "⌥→"),
        ("select.shift", "⇧"),
        ("guide.next", "F2"),
        ("hint.dismiss", "F4"),
        ("search.open", "⌃F"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

/// Lays out and composes `layers` over the state's frame: (the grid drawn, the scene).
pub fn draw(s: &State, layers: &Layers, glyphs: Glyphs) -> (TestGrid, Scene) {
    let frame = view(s);
    let mut cells = TestGrid::from_frame(&frame);
    let res = FrameResolver::new(&frame).with_doc(&s.doc);
    let area = Rect::new(0, 0, frame.width, s.text_rows() as u16);
    let grid = Grid::scan(&cells).with_area(area).with_caret(frame.cursor);
    let k = keys();
    let scene = layout(layers, &res, &grid, &Opts { glyphs, keys: &k });
    compose(&scene, &mut cells);
    (cells, scene)
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

pub fn callout(title: &str, body: &str) -> Item {
    Item::Callout(Callout {
        title: Some(title.into()),
        body: body.into(),
        ..Callout::default()
    })
}

pub fn send(s: &mut State, msgs: Vec<Msg>) {
    for m in msgs {
        caretline::update(s, m);
    }
}

fn path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name)
}

/// Compares `actual` with the golden file; `OVERLAY_GOLDENS=update` rewrites it instead.
pub fn golden(name: &str, actual: &str) {
    let p = path(name);
    if std::env::var("OVERLAY_GOLDENS").as_deref() == Ok("update") {
        std::fs::write(&p, actual).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&p).unwrap_or_else(|_| {
        panic!("no golden {name}: run with OVERLAY_GOLDENS=update and review it")
    });
    assert!(
        want == actual,
        "golden {name} differs.\n--- want\n{want}\n--- got\n{actual}"
    );
}

/// A frame golden: the text, then the role and flag map.
pub fn frame_golden(name: &str, grid: &TestGrid) {
    golden(
        name,
        &format!("{}--- roles\n{}", grid.to_text(), grid.role_map()),
    );
}
