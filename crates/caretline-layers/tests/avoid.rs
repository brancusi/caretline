#![cfg(feature = "caretline")]
//! Soft avoid areas: cells a box and its arrow keep off when anything else fits. A host marks
//! them on the grid (`Grid::avoid`: a highlighted band, a table) or a layer names them as
//! anchors (`Layer::avoid`: the text its step talks about), resolved every frame. A box goes
//! further from its anchor, within the grid's reach, to keep clear, and says when it couldn't
//! (`Planned::covers_avoid`).

mod common;

use caretline::{State, Viewport, view};
use caretline_layers::*;
use common::*;
use serde_json::{Value, json};

/// A review of a change: the diff's changed lines are the band the step talks about.
const DIFF: &str = "# Review: parser

@@ -12,7 +12,9 @@ fn parse(input: &str) -> Result<Ast> {
     let mut tokens = lex(input)?;
     let mut ast = Ast::new();
-    while let Some(t) = tokens.next() {
-        ast.push(t)?;
+    for t in tokens {
+        let node = t.into_node()?;
+        ast.push(node)?;
     }
     Ok(ast)
 }

The loop now converts each token into a node before it is pushed, so a
token that can't become a node stops the parse with an error there.

Notes on the rest of the change follow below, with the tests that cover it
and the cases they leave out for a later change.
";

/// A table of releases with a line of notes under it.
const TABLE: &str = "# Releases

| Version | Date       | Notes           |
|---------|------------|-----------------|
| 0.3.0   | 2026-09-01 | Views and folds |
| 0.2.0   | 2026-07-14 | The protocol    |
| 0.1.0   | 2026-05-02 | First release   |

Each row links to its notes. Older releases are in the archive, with
the notes for every version before these three.
";

/// The char range of `needle` in `doc`.
fn range(doc: &str, needle: &str) -> Anchor {
    let b = doc.find(needle).unwrap_or_else(|| panic!("{needle:?}"));
    let from = doc[..b].chars().count();
    Anchor::Text {
        from,
        to: from + needle.chars().count(),
    }
}

fn state_of(doc: &str, w: u16, h: u16) -> State {
    State::new(
        doc,
        Some("page.md".into()),
        Viewport {
            width: w,
            height: h,
        },
    )
}

/// Plans `layers` over `doc`, with `setup` marking avoid cells on the grid.
fn plan_doc(
    doc: &str,
    layers: &Layers,
    setup: impl Fn(&mut Grid, &FrameResolver),
) -> (caretline::view::Frame, Grid, Plan) {
    let s = state_of(doc, 80, 24);
    let frame = view(&s);
    let res = FrameResolver::new(&frame).with_doc(&s.doc);
    let mut grid = Grid::from_frame(&frame);
    setup(&mut grid, &res);
    let p = plan(layers, &res, &grid, &renderers());
    (frame, grid, p)
}

/// The rows a text anchor's cells take, as full-width rects.
fn rows_of(res: &FrameResolver, a: &Anchor, w: u16) -> Vec<Rect> {
    res.resolve(a)
        .unwrap()
        .rects
        .iter()
        .map(|r| Rect::new(0, r.y, w, r.h))
        .collect()
}

fn goldens(name: &str, frame: &caretline::view::Frame, p: &Plan, layers: &Layers) {
    golden(&format!("{name}.txt"), &picture(frame, p, layers));
    golden(
        &format!("{name}.json"),
        &(serde_json::to_string_pretty(p).unwrap() + "\n"),
    );
}

#[test]
fn a_hint_keeps_off_the_band_its_step_explains() {
    let band = {
        let Anchor::Text { from, .. } = range(DIFF, "-    while") else {
            unreachable!()
        };
        let Anchor::Text { to, .. } = range(DIFF, "+        ast.push(node)?;") else {
            unreachable!()
        };
        Anchor::Text { from, to }
    };
    // The step talks about the paragraph under the diff too: the layer keeps off it.
    let notes = range(
        DIFF,
        "The loop now converts each token into a node before it is pushed, so a\ntoken that can't become a node stops the parse with an error there.",
    );
    let layer = Layer::new(range(DIFF, "tokens.next()"))
        .with_content(hint("Into a node", "Each token becomes a node first."))
        .with_arrow()
        .with_ring();
    let mut plain = Layers::default();
    push(&mut plain, layer.clone(), None);
    let mut avoiding = Layers::default();
    push(&mut avoiding, layer.with_avoid(vec![notes.clone()]), None);
    let s = state_of(DIFF, 80, 24);
    let frame = view(&s);
    let res = FrameResolver::new(&frame).with_doc(&s.doc);
    // The host highlights the changed lines across the screen's width.
    let rows = rows_of(&res, &band, 80);
    let notes_cells = res.resolve(&notes).unwrap().rects;
    let on = |cells: &[Rect], r: Rect| cells.iter().any(|c| c.intersects(&r));

    // Without avoiding them, the box sits on the band's last lines.
    let (_, _, before) = plan_doc(DIFF, &plain, |_, _| {});
    assert!(on(&rows, before.layers[0].rect.unwrap()), "{before:?}");

    // Avoiding the band (the host's) and the notes (the layer's): beside them both.
    let (frame, _, p) = plan_doc(DIFF, &avoiding, |g, _| {
        for r in &rows {
            g.avoid(*r, AVOID);
        }
    });
    let l = &p.layers[0];
    let r = l.rect.unwrap();
    assert!(!on(&rows, r), "{r:?} covers the band");
    assert!(!on(&notes_cells, r), "{r:?} covers the notes");
    assert_eq!(l.covers_avoid, 0);
    assert!(l.route.is_some(), "{l:?}");
    goldens("avoid.diff", &frame, &p, &avoiding);
}

#[test]
fn a_hint_at_a_table_row_keeps_its_arrow_off_the_other_rows() {
    let table = {
        let Anchor::Text { from, .. } = range(TABLE, "| Version") else {
            unreachable!()
        };
        let Anchor::Text { to, .. } = range(TABLE, "First release   |") else {
            unreachable!()
        };
        Anchor::Text { from, to }
    };
    let row = range(TABLE, "| 0.2.0   | 2026-07-14 | The protocol    |");
    let mut layers = Layers::default();
    push(
        &mut layers,
        Layer::new(row)
            .with_content(hint("0.2.0", "The release that added the protocol."))
            .with_arrow(),
        None,
    );
    // The host marks the whole table, full width, as a band to keep off.
    let (frame, grid, p) = plan_doc(TABLE, &layers, |g, res| {
        for r in rows_of(res, &table, 80) {
            g.avoid(r, AVOID);
        }
    });
    let l = &p.layers[0];
    let r = l.rect.unwrap();
    assert_eq!(l.covers_avoid, 0, "{l:?}");
    let route = l.route.as_ref().expect("an arrow");
    // The arrow keeps to blank cells: the margin past the rows' ends, or a gap.
    for s in &route.steps {
        assert_eq!(grid.kind(s.x, s.y), CellKind::Blank, "{s:?} on text");
    }
    // And the box is clear of the table.
    assert!(rows_of_plan(&grid, r));
    goldens("avoid.table", &frame, &p, &layers);
}

/// Whether a box covers no avoid cell of the grid.
fn rows_of_plan(grid: &Grid, r: Rect) -> bool {
    (r.y..r.bottom()).all(|y| (r.x..r.right()).all(|x| grid.avoid_weight(x, y) == 0))
}

/// A host whose `card` is a fixed size.
fn card_size(w: u16, h: u16) -> Renderers {
    Renderers::new().register("card", move |_: &Value, avail: Size| {
        Size::new(w.min(avail.w), h)
    })
}

#[test]
fn a_box_goes_further_out_past_protected_and_avoided_rows() {
    // Under the anchor, a band to keep off (rows 6 to 12), part of it protected too (a
    // selection, right of column 20): the box goes on down to the blank rows past the band,
    // its arrow bridging the gap.
    let grid = Grid::new(80, 30)
        .with_area(Rect::new(0, 0, 80, 29))
        .with_protect(vec![Rect::new(20, 6, 60, 3)])
        .with_avoid(Rect::new(0, 6, 80, 7), AVOID);
    let mut m = AnchorMap::new();
    m.put(AnchorKey::host("row", "a"), Rect::new(10, 5, 6, 1));
    let a = Anchor::Host {
        kind: "row".into(),
        key: "a".into(),
    };
    let mut layer = Layer::new(a)
        .with_content(Content::new("card", json!({})))
        .with_arrow();
    layer.place = vec![Side::Below];
    let mut l = Layers::default();
    push(&mut l, layer.clone(), None);
    let p = plan(&l, &m, &grid, &card_size(20, 3));
    let pl = &p.layers[0];
    let r = pl.rect.expect("a box, not a strip");
    assert_eq!(pl.mode, Some(Mode::Box));
    assert_eq!(r.y, 13, "the first clear rows past the band");
    assert_eq!(pl.covers_avoid, 0);
    assert!(pl.route.is_some());
    // Out of reach, it covers the band rather than give up, and says so.
    let near = grid.clone().with_reach(Reach { rows: 4, cols: 4 });
    let p = plan(&l, &m, &near, &card_size(20, 3));
    assert!(p.layers[0].covers_avoid > 0, "{:?}", p.layers[0]);
    // Deterministic: the same inputs, the same plan.
    let again = plan(&l, &m, &grid, &card_size(20, 3));
    assert_eq!(again, plan(&l, &m, &grid, &card_size(20, 3)));
}

/// xorshift64*: small, seeded, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

#[test]
fn a_box_clear_of_avoid_cells_wins_whenever_the_search_has_one() {
    // Blank screens with full-width avoid bands (so where along a row a box sits doesn't
    // matter) and a box above or below an anchor: if any row the search tries, within reach,
    // is clear of the bands, the chosen box covers no avoid cell.
    let (bw, bh) = (20u16, 3u16);
    for seed in 1..=400u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (w, h) = (60 + rng.below(40) as u16, 20 + rng.below(20) as u16);
        let area = Rect::new(0, 0, w, h - 1);
        let mut grid = Grid::new(w, h).with_area(area);
        let reach = Reach {
            rows: rng.below(14) as u16,
            cols: 40,
        };
        grid = grid.with_reach(reach);
        let ay = 2 + rng.below(h as u64 - 5) as u16;
        let anchor = Rect::new(5 + rng.below(20) as u16, ay, 4 + rng.below(8) as u16, 1);
        let mut banned = vec![false; h as usize];
        for _ in 0..rng.below(5) {
            let y = rng.below(h as u64) as u16;
            let n = 1 + rng.below(5) as u16;
            let weight = 1 + rng.below(40) as u16;
            grid.avoid(Rect::new(0, y, w, n), weight);
            for b in banned.iter_mut().skip(y as usize).take(n as usize) {
                *b = true;
            }
        }
        let clear = |y: i32| {
            y >= area.y as i32
                && y + bh as i32 <= area.bottom() as i32
                && (y..y + bh as i32).all(|r| !banned[r as usize])
        };
        // The rows the search tries: the nearest four, then out to the reach.
        let last = 1 + 3.max(reach.rows) as i32;
        let exists = (1..=last)
            .any(|k| clear(anchor.bottom() as i32 + k) || clear(anchor.y as i32 - k - bh as i32));
        let mut m = AnchorMap::new();
        m.put(AnchorKey::host("row", "a"), anchor);
        let mut layer = Layer::new(Anchor::Host {
            kind: "row".into(),
            key: "a".into(),
        })
        .with_content(Content::new("card", json!({})));
        if rng.below(2) == 0 {
            layer = layer.with_arrow();
        }
        layer.place = vec![Side::Below, Side::Above];
        let mut l = Layers::default();
        push(&mut l, layer, None);
        let p = plan(&l, &m, &grid, &card_size(bw, bh));
        let pl = &p.layers[0];
        if exists {
            assert_eq!(pl.covers_avoid, 0, "seed {seed}: {pl:?}");
            assert_eq!(pl.mode, Some(Mode::Box), "seed {seed}");
        }
        // And the count is right.
        if let Some(r) = pl.rect.filter(|_| pl.mode == Some(Mode::Box)) {
            let n = (r.y..r.bottom()).filter(|&y| banned[y as usize]).count() as u16 * r.w;
            assert_eq!(pl.covers_avoid, n, "seed {seed}");
        }
        assert_eq!(p, plan(&l, &m, &grid, &card_size(bw, bh)), "seed {seed}");
    }
}

#[test]
fn avoid_anchors_go_on_the_wire_and_in_the_schema() {
    let req = json!({
        "op": "hint.show",
        "anchor": {"text": {"from": 4, "to": 9}},
        "avoid": [{"text": {"from": 0, "to": 40}}, {"block": 3, "in": "main"}],
        "text": "Not over the band."
    });
    let (ops::Request::Apply(LayerOp::Push(l)), _) = ops::parse("hint.show", &req).unwrap() else {
        panic!()
    };
    assert_eq!(l.avoid.len(), 2);
    let wire = serde_json::to_value(&l).unwrap();
    assert_eq!(wire["avoid"][1], json!({"block": 3, "in": "main"}));
    let back: Layer = serde_json::from_value(wire).unwrap();
    assert_eq!(back, l);
    // A malformed avoid anchor is refused like a malformed anchor.
    let mut bad = Layer::new(Anchor::Caret).with_content(hint("t", "b"));
    bad.avoid = vec![Anchor::In {
        view: String::new(),
        anchor: Box::new(Anchor::Caret),
    }];
    let r = apply(
        &mut Layers::default(),
        LayerOp::Push(bad),
        None,
        0,
        &Limits::default(),
    );
    assert_eq!(r.unwrap_err().reason, Reason::Invalid);
    let schema = ops::schema();
    assert!(schema["$defs"]["layer"]["properties"]["avoid"].is_object());
    assert!(schema["$defs"]["hint.show"].to_string().contains("avoid"));
}
