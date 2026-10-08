#![cfg(feature = "caretline")]
//! One document in several views: a main editor and a side panel on one host screen, each
//! drawn at its own offset and clipped to its own rect. Scoped anchors resolve only in the
//! view they name; unscoped ones in the focused view first, then the first that shows them,
//! then which way they lie from the focused view. An edit through either view maps the
//! anchors in both.

mod common;

use caretline::view::{Cell, CellFlags, Frame, RowInfo, render};
use caretline::{Document, Msg, View, Viewport, update_doc, update_doc_with_changes};
use caretline_layers::*;
use common::*;

const W: u16 = 100;
const H: u16 = 24;
/// The main editor: the screen's left 60 columns, all rows.
const MAIN: Rect = Rect::new(0, 0, 60, 24);
/// The panel: drawn at (62, 2), its frame 36x14, clipped to the top 9 rows.
const PANEL_AT: (u16, u16) = (62, 2);
const PANEL_CLIP: Rect = Rect::new(62, 2, 36, 9);

/// The document and its two views: the main one at the top, the panel scrolled to "## Undo".
fn setup() -> (Document, Vec<View>) {
    let mut doc = Document::new(DOC, Some("guide.md".into()));
    let mut views = vec![
        View::new(Viewport {
            width: MAIN.w,
            height: MAIN.h,
        }),
        View::new(Viewport {
            width: 36,
            height: 14,
        }),
    ];
    update_doc(&mut doc, &mut views, 1, Msg::ScrollView { rows: 22 });
    (doc, views)
}

/// The two views as the host draws them, and the screen as placement sees it (the panel's
/// cells only inside its clip).
fn frames(doc: &Document, views: &[View]) -> (Frame, Frame, Grid) {
    let (main, panel) = (render(doc, &views[0]), render(doc, &views[1]));
    let mut grid = Grid::new(W, H).with_area(Rect::new(0, 0, W, H - 1));
    grid.mark_frame(&main, MAIN.x, MAIN.y);
    grid.mark_frame(&panel, PANEL_AT.0, PANEL_AT.1);
    for y in PANEL_AT.1..PANEL_AT.1 + panel.height {
        for x in PANEL_AT.0..PANEL_AT.0 + panel.width {
            if !PANEL_CLIP.contains(x, y) {
                grid.set_kind(x, y, CellKind::Blank);
            }
        }
    }
    (main, panel, grid)
}

fn resolvers<'a>(
    doc: &'a Document,
    main: &'a Frame,
    panel: &'a Frame,
    focus_panel: bool,
) -> (FrameResolver<'a>, FrameResolver<'a>) {
    let m = FrameResolver::new(main).with_doc(doc).id("main");
    let p = FrameResolver::new(panel)
        .with_doc(doc)
        .at(PANEL_AT.0, PANEL_AT.1)
        .id("panel:2")
        .clip(PANEL_CLIP);
    if focus_panel {
        (m, p.focused())
    } else {
        (m.focused(), p)
    }
}

/// The host's screen: both views' cells (the panel's inside its clip), for the picture.
fn screen(main: &Frame, panel: &Frame) -> Frame {
    let blank = Cell {
        symbol: " ".into(),
        role: main.cell(0, 0).role,
        char_idx: None,
        flags: CellFlags::NONE,
    };
    let mut f = Frame {
        width: W,
        height: H,
        cells: vec![blank; W as usize * H as usize],
        cursor: None,
        rows: (0..H)
            .map(|y| {
                if y + 1 == H {
                    RowInfo::Status
                } else {
                    RowInfo::Past
                }
            })
            .collect(),
        roles: Vec::new(),
        cell_px: None,
        regions: Vec::new(),
    };
    for y in 0..main.height {
        for x in 0..main.width {
            f.cells[y as usize * W as usize + x as usize] = main.cell(x, y).clone();
        }
    }
    for y in 0..panel.height {
        for x in 0..panel.width {
            let (sx, sy) = (PANEL_AT.0 + x, PANEL_AT.1 + y);
            if PANEL_CLIP.contains(sx, sy) {
                f.cells[sy as usize * W as usize + sx as usize] = panel.cell(x, y).clone();
            }
        }
    }
    f
}

/// The text under a resolved anchor's cells on the host's screen.
fn under(f: &Frame, r: &Resolved) -> String {
    r.rects
        .iter()
        .map(|r| {
            (r.x..r.right())
                .map(|x| f.cell(x, r.y).symbol.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn hint_at(anchor: Anchor, text: &str) -> Layer {
    Layer::new(anchor).with_content(hint("Note", text))
}

#[test]
fn a_scoped_anchor_resolves_only_in_its_view() {
    let (doc, views) = setup();
    let (main, panel, _) = frames(&doc, &views);
    let screen = screen(&main, &panel);
    let (m, p) = resolvers(&doc, &main, &panel, false);
    let both = Chain(vec![&m, &p]);
    // "## Undo" shows in both views.
    let undo = find("## Undo", 0);
    let in_main = both.resolve(&undo).unwrap();
    assert_eq!(in_main.view.as_deref(), Some("main"), "focused first");
    assert_eq!(under(&screen, &in_main), "## Undo");
    assert!(in_main.rects[0].x < MAIN.right());
    let scoped = Anchor::scoped("panel:2", undo.clone());
    let in_panel = both.resolve(&scoped).unwrap();
    assert_eq!(in_panel.view.as_deref(), Some("panel:2"));
    assert_eq!(under(&screen, &in_panel), "## Undo");
    assert!(PANEL_CLIP.contains(in_panel.rects[0].x, in_panel.rects[0].y));
    // A view no resolver has: nothing.
    assert_eq!(both.resolve(&Anchor::scoped("panel:9", undo.clone())), None);
    // The resolver alone: only anchors scoped to it, or unscoped.
    assert_eq!(m.resolve(&scoped), None);
    assert!(p.resolve(&undo).is_some());
    // Each view's own caret: main's at the top; the panel's is scrolled out of it, and a
    // scoped anchor never falls back to another view.
    let caret = both
        .resolve(&Anchor::scoped("main", Anchor::Caret))
        .unwrap();
    assert_eq!(caret.view.as_deref(), Some("main"));
    assert_eq!(caret.rects, vec![Rect::new(0, 0, 1, 1)]);
    assert_eq!(
        both.resolve(&Anchor::scoped("panel:2", Anchor::Caret)),
        None
    );
}

#[test]
fn unscoped_anchors_go_focused_first_then_first_shown_then_focused_direction() {
    let (doc, views) = setup();
    let (main, panel, _) = frames(&doc, &views);
    let screen = screen(&main, &panel);
    // Focus on the panel: "## Undo" (in both) resolves there now.
    let (m, p) = resolvers(&doc, &main, &panel, true);
    let both = Chain(vec![&m, &p]);
    let r = both.resolve(&find("## Undo", 0)).unwrap();
    assert_eq!(r.view.as_deref(), Some("panel:2"));
    // "Getting around" shows only in main: the first view that shows it.
    let r = both.resolve(&find("Getting around", 0)).unwrap();
    assert_eq!(r.view.as_deref(), Some("main"));
    assert_eq!(under(&screen, &r), "Getting around");
    // "Control S" shows in neither (below main's last row, clipped off the panel): which way
    // it lies from the focused view, the panel, pulled inside its clip.
    let r = both.resolve(&find("Control S", 0)).unwrap();
    assert!(r.rects.is_empty());
    assert_eq!(r.view.as_deref(), Some("panel:2"));
    let Some(Off::Below { x: Some(x) }) = r.off else {
        panic!("{r:?}")
    };
    assert!((PANEL_CLIP.x..PANEL_CLIP.right()).contains(&x));
    // Focus on main: the same anchor lies below main.
    let (m, p) = resolvers(&doc, &main, &panel, false);
    let r = Chain(vec![&m, &p]).resolve(&find("Control S", 0)).unwrap();
    assert_eq!(r.view.as_deref(), Some("main"));
    assert!(matches!(r.off, Some(Off::Below { .. })));
}

#[test]
fn a_clipped_anchor_lies_off_its_view_the_way_its_cells_are() {
    let (doc, views) = setup();
    let (main, panel, _) = frames(&doc, &views);
    let p = FrameResolver::new(&panel)
        .with_doc(&doc)
        .at(PANEL_AT.0, PANEL_AT.1)
        .id("panel:2");
    // Unclipped, the panel's frame shows "## Search" on a row past the clip.
    let full = p.resolve(&find("## Search", 0)).unwrap();
    assert_eq!(full.rects.len(), 1);
    assert!(full.rects[0].y >= PANEL_CLIP.bottom(), "{full:?}");
    let clipped = p.clip(PANEL_CLIP).resolve(&find("## Search", 0)).unwrap();
    assert!(clipped.rects.is_empty());
    assert_eq!(
        clipped.off,
        Some(Off::Below {
            x: Some(full.rects[0].x)
        })
    );
    // Clipped from the left: the cells lie left of the view.
    let narrow = Rect::new(PANEL_CLIP.x + 12, PANEL_CLIP.y, 20, PANEL_CLIP.h);
    let p = FrameResolver::new(&panel)
        .with_doc(&doc)
        .at(PANEL_AT.0, PANEL_AT.1)
        .clip(narrow);
    let r = p.resolve(&find("## Undo", 0)).unwrap();
    assert!(r.rects.is_empty(), "{r:?}");
    assert!(matches!(r.off, Some(Off::Left { .. })), "{r:?}");
    // Partly clipped: only the cells inside count.
    let r = p.resolve(&find("Every change can be undone.", 0)).unwrap();
    assert!(r.rects.iter().all(|c| narrow.contains(c.x, c.y)), "{r:?}");
    assert!(!r.rects.is_empty());
    let _ = main;
}

#[test]
fn an_edit_through_one_view_maps_anchors_in_both() {
    let (mut doc, mut views) = setup();
    let mut layers = Layers::default();
    let words = [
        ("main", "Getting around"),
        ("panel:2", "Every change can be undone."),
        ("panel:2", "## Undo"),
    ];
    for (view, needle) in words {
        push(
            &mut layers,
            hint_at(Anchor::scoped(view, find(needle, 0)), needle),
            None,
        );
    }
    // Type at the top of the document through the main view.
    let (_, cs) = update_doc_with_changes(
        &mut doc,
        &mut views,
        0,
        Msg::InsertText {
            text: "Hello. ".into(),
        },
    );
    // One document in both views: the edit maps anchors scoped to either, and unscoped ones.
    let both_views = Edited::Views {
        views: &["main", "panel:2"],
        unscoped: true,
    };
    assert!(observe(&mut layers, both_views, cs.as_ref(), 2_000));
    let (main, panel, _) = frames(&doc, &views);
    let screen = screen(&main, &panel);
    let (m, p) = resolvers(&doc, &main, &panel, false);
    let both = Chain(vec![&m, &p]);
    for ((view, needle), layer) in words.iter().zip(&layers.layers) {
        assert_eq!(layer.anchor[0].view(), Some(*view));
        let r = both.resolve(&layer.anchor[0]).unwrap();
        assert_eq!(r.view.as_deref(), Some(*view));
        assert_eq!(under(&screen, &r), *needle, "{view}");
    }
    // And through the panel: a word deleted there drops the anchor on it in both.
    let Anchor::In { anchor, .. } = &layers.layers[1].anchor[0] else {
        panic!()
    };
    let Anchor::Text { from, to } = **anchor else {
        panic!()
    };
    let (_, cs) = update_doc_with_changes(
        &mut doc,
        &mut views,
        1,
        Msg::External {
            changes: vec![caretline::ExtChange::Replace {
                from,
                to,
                text: String::new(),
            }],
        },
    );
    observe(&mut layers, both_views, cs.as_ref(), 3_000);
    assert_eq!(layers.layers.len(), 2);
}

/// The text of a text anchor's chars in a document.
fn text_of(doc: &Document, a: &Anchor) -> String {
    let Anchor::Text { from, to } = *a.unscoped() else {
        panic!("{a:?}")
    };
    doc.text.slice(from..to).to_string()
}

#[test]
fn an_edit_moves_only_the_anchors_in_its_own_document() {
    // Two documents: page A in `main` (focused), page B in `panel:1`. A `ChangeSet` belongs
    // to one of them.
    let vp = |width, height| View::new(Viewport { width, height });
    let mut a = Document::new("alpha bravo charlie delta", Some("a.md".into()));
    let mut b = Document::new("abc hello world, and more text", Some("b.md".into()));
    let mut a_views = vec![vp(60, 20)];
    let mut b_views = vec![vp(36, 9)];
    let mut layers = Layers::default();
    // A hint scoped to the panel, at "hello" of page B; one scoped to main and an unscoped
    // one, both at "bravo" of page A.
    let in_b: Anchor =
        serde_json::from_str(r#"{"text":{"from":4,"to":9},"in":"panel:1"}"#).unwrap();
    let bravo = Anchor::Text { from: 6, to: 11 };
    for anchor in [in_b, Anchor::scoped("main", bravo.clone()), bravo] {
        push(&mut layers, hint_at(anchor, "x"), None);
    }
    let texts = |layers: &Layers, a: &Document, b: &Document| -> Vec<String> {
        layers
            .layers
            .iter()
            .zip([b, a, a])
            .map(|(l, d)| text_of(d, &l.anchor[0]))
            .collect()
    };
    assert_eq!(texts(&layers, &a, &b), ["hello", "bravo", "bravo"]);

    // Type three chars at the top of page A, in main: the panel's anchor stays where it was.
    let (_, cs) = update_doc_with_changes(
        &mut a,
        &mut a_views,
        0,
        Msg::InsertText { text: "XYZ".into() },
    );
    let main = Edited::Views {
        views: &["main"],
        unscoped: true,
    };
    assert!(observe(&mut layers, main, cs.as_ref(), 0));
    assert_eq!(texts(&layers, &a, &b), ["hello", "bravo", "bravo"]);
    assert_eq!(
        layers.layers[0].anchor[0],
        Anchor::scoped("panel:1", Anchor::Text { from: 4, to: 9 })
    );

    // Type at the top of page B, in the panel (not the focused view's document): only the
    // panel's anchor moves; the unscoped one belongs to the focused view's page A.
    let (_, cs) = update_doc_with_changes(
        &mut b,
        &mut b_views,
        0,
        Msg::InsertText { text: "> ".into() },
    );
    let panel = Edited::Views {
        views: &["panel:1"],
        unscoped: false,
    };
    assert!(observe(&mut layers, panel, cs.as_ref(), 0));
    assert_eq!(texts(&layers, &a, &b), ["hello", "bravo", "bravo"]);
    assert_eq!(
        layers.layers[0].anchor[0],
        Anchor::scoped("panel:1", Anchor::Text { from: 6, to: 11 })
    );

    // Deleting "hello" from page B drops only the anchor on it.
    let (_, cs) = update_doc_with_changes(
        &mut b,
        &mut b_views,
        0,
        Msg::External {
            changes: vec![caretline::ExtChange::Replace {
                from: 6,
                to: 11,
                text: String::new(),
            }],
        },
    );
    assert!(observe(&mut layers, panel, cs.as_ref(), 0));
    assert_eq!(layers.layers.len(), 2);
    assert!(
        layers
            .layers
            .iter()
            .all(|l| text_of(&a, &l.anchor[0]) == "bravo")
    );

    // `Edited::All` is for a host with one document: here, page B's edit would move page
    // A's anchors.
    let (_, cs) = update_doc_with_changes(
        &mut b,
        &mut b_views,
        0,
        Msg::InsertText { text: "new".into() },
    );
    observe(&mut layers, Edited::All, cs.as_ref(), 0);
    assert_eq!(layers.layers.len(), 2);
    assert!(
        layers
            .layers
            .iter()
            .all(|l| text_of(&a, &l.anchor[0]) != "bravo")
    );
}

#[test]
fn anchors_scope_on_the_wire_strictly() {
    let a: Anchor = serde_json::from_str(r#"{"text":{"from":4,"to":9},"in":"panel:2"}"#).unwrap();
    assert_eq!(
        a,
        Anchor::scoped("panel:2", Anchor::Text { from: 4, to: 9 })
    );
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        r#"{"text":{"from":4,"to":9},"in":"panel:2"}"#
    );
    for ok in [
        r#"{"block":7,"in":"main"}"#,
        r#"{"caret":true,"in":"main"}"#,
        r#"{"text":{"block":7,"from":1,"to":2},"in":"main"}"#,
    ] {
        let a: Anchor = serde_json::from_str(ok).unwrap();
        assert_eq!(serde_json::to_string(&a).unwrap(), ok);
    }
    for bad in [
        r#"{"in":"main"}"#,
        r#"{"screen":"center","in":"main"}"#,
        r#"{"host":{"kind":"row","key":"a"},"in":"main"}"#,
        r#"{"caret":true,"in":""}"#,
        r#"{"caret":true,"in":2}"#,
        r#"{"caret":true,"view":"main"}"#,
        r#"{"caret":true,"block":3}"#,
    ] {
        assert!(serde_json::from_str::<Anchor>(bad).is_err(), "{bad}");
    }
    // Built in Rust, a malformed scope is refused, not written.
    let nested = Anchor::In {
        view: "a".into(),
        anchor: Box::new(Anchor::scoped("b", Anchor::Caret)),
    };
    assert!(serde_json::to_string(&nested).is_err());
    let mut l = Layers::default();
    let r = apply(
        &mut l,
        LayerOp::Push(hint_at(nested, "x")),
        None,
        0,
        &Limits::default(),
    );
    assert_eq!(r.unwrap_err().reason, Reason::Invalid);
    // Scoping again replaces the view.
    assert_eq!(
        Anchor::scoped("b", Anchor::scoped("a", Anchor::Caret)),
        Anchor::scoped("b", Anchor::Caret)
    );
}

/// The plan of layers in both views, and the host's picture of it.
fn golden_views(name: &str, focus_panel: bool) {
    let (doc, views) = setup();
    let (main, panel, grid) = frames(&doc, &views);
    let (m, p) = resolvers(&doc, &main, &panel, focus_panel);
    let mut layers = Layers::default();
    for (anchor, text, arrow) in [
        (
            Anchor::scoped("panel:2", find("Every change can be undone.", 0)),
            "Scoped to the panel.",
            true,
        ),
        (
            Anchor::scoped("main", find("word at a time", 0)),
            "Scoped to main.",
            true,
        ),
        (find("## Undo", 0), "In both: the focused view's.", true),
        (find("Control S", 0), "In neither.", false),
    ] {
        let mut l = hint_at(anchor, text);
        l.arrow = arrow;
        push(&mut layers, l, None);
    }
    let chain = Chain(vec![&m, &p]);
    let r = renderers();
    let p = plan(&layers, &chain, &grid, &r);
    conform(&layers, &chain, &grid, &r, &p);
    golden(
        &format!("{name}.txt"),
        &picture(&screen(&main, &panel), &p, &layers),
    );
    golden(
        &format!("{name}.json"),
        &(serde_json::to_string_pretty(&p).unwrap() + "\n"),
    );
}

#[test]
fn layers_over_two_views_focused_on_main() {
    golden_views("views.main", false);
}

#[test]
fn layers_over_two_views_focused_on_the_panel() {
    golden_views("views.panel", true);
}
