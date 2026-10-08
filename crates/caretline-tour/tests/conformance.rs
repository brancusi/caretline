#![cfg(feature = "caretline")]
//! The layers conformance kit over every step of the example walkthrough
//! (examples/tour.toml), planned with `plan_steps` over a generic host at several sizes: a
//! table of rows on the left, a document on the right ("main") and its outline under the
//! table ("outline").

use caretline::view::render;
use caretline::{Document, View, Viewport};
use caretline_layers::conformance::{Scene, check_determinism, check_plan};
use caretline_layers::{
    AnchorKey, AnchorMap, Chain, FrameResolver, Grid, HINT, LayerOp, Layers, Limits, Rect,
    Renderers, Size, apply,
};
use caretline_tour::*;
use serde_json::Value;

const DOC: &str = "# Report\n\nThe quarter in short: what changed, what it cost, what comes next.\n\n## Summary\n\nRevenue rose and costs held. The details follow below, section by section.\n\n## Costs\n\nMostly people and machines.\n";

fn renderers() -> Renderers {
    Renderers::new().register(HINT, |data: &Value, avail: Size| {
        let text = data["text"].as_str().unwrap_or_default();
        let w = (text.chars().count() as u16 + 4).min(avail.w).min(40);
        let lines = (text.chars().count() as u16).div_ceil(w.saturating_sub(4).max(1));
        Size::new(w, (lines + 3).min(avail.h))
    })
}

#[test]
fn every_step_of_the_example_keeps_the_layers_invariants() {
    let mut tour = parse_toml(include_str!("../examples/tour.toml")).unwrap();
    let doc = Document::new(DOC, Some("report.md".into()));
    let missing = tour.resolve_finds(|text, _| find_in(&doc, text));
    assert!(missing.is_empty(), "{missing:?}");
    let r = renderers();
    let mut checked = 0;
    let mut violations = Vec::new();
    let sizes = [(140, 40), (100, 30), (80, 24), (44, 16)];
    let plans = plan_steps(&tour, &sizes, &r, &mut |step, w, h, go| {
        let half = w / 2;
        let rows = (h - 1) / 2;
        // The table: rows on the left.
        let mut grid = Grid::new(w, h).with_area(Rect::new(0, 0, w, h - 1));
        let mut anchors = AnchorMap::new();
        for (i, key) in ["first", "second", "third"].iter().enumerate() {
            let y = 1 + i as u16;
            let text = format!("{key} row");
            grid.mark_text(2, y, &text);
            anchors.put(
                AnchorKey::host("row", key),
                Rect::new(2, y, text.len() as u16, 1),
            );
        }
        // The document on the right, its outline under the table.
        let views = [
            View::new(Viewport {
                width: w - half,
                height: h - 1,
            }),
            View::new(Viewport {
                width: half,
                height: h - 1 - rows,
            }),
        ];
        let main = render(&doc, &views[0]);
        let outline = render(&doc, &views[1]);
        grid.mark_frame(&main, half, 0);
        grid.mark_frame(&outline, 0, rows);
        let m = FrameResolver::new(&main)
            .with_doc(&doc)
            .at(half, 0)
            .id("main")
            .focused();
        let o = FrameResolver::new(&outline)
            .with_doc(&doc)
            .at(0, rows)
            .id("outline")
            .clip(Rect::new(0, rows, half, h - 1 - rows));
        let chain = Chain(vec![&anchors, &m, &o]);
        go(&grid, &chain);
        // The same step's layers, as plan_steps pushes them, checked by the kit.
        let i = tour.steps.iter().position(|s| s.id == step.id).unwrap();
        let mut layers = Layers::default();
        for l in step_layers(&tour, i, false) {
            let _ = apply(&mut layers, LayerOp::Push(l), None, 0, &Limits::default());
        }
        let scene = Scene::new(layers, &chain, grid.clone(), &r);
        let p = scene.plan();
        for v in check_plan(&p, &scene)
            .into_iter()
            .chain(check_determinism(&scene))
        {
            violations.push(format!("{} {w}x{h}: {v}", step.id));
        }
        checked += 1;
    });
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    assert_eq!(checked, tour.steps.len() * sizes.len());
    // Every step found its anchors at every size.
    for p in &plans {
        assert!(
            p.problems.iter().all(|x| x.level != Level::Error),
            "{} {}x{}: {:?}",
            p.step,
            p.width,
            p.height,
            p.problems
        );
    }
}
