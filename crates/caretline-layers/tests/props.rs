#![cfg(feature = "caretline")]
//! Properties over random sizes, anchors and layers (a seeded PRNG, so a failure names its
//! seed and replays): a box never covers its anchor, an earlier layer's box or hole, or the
//! status row; an agent's box never covers the caret; no box edge, chip or arrow splits a
//! wide grapheme; the plan is the same after a resize there and back.

mod common;

use caretline::{Msg, State, Viewport, update, view};
use caretline_layers::*;

const SEEDS: u64 = 300;

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
    fn chance(&mut self, k: u64) -> bool {
        self.below(k) == 0
    }
}

/// Text with wide graphemes (CJK, emoji) and plenty of words and blank lines.
const DOC: &str = "# 見出し and a heading\n\nSome words, then 漢字 in the middle, and an emoji 👍 here.\nA line of plain words that goes on for quite a while before it ends.\n\n## Another 🧭 section\n\n全角の文字が並ぶ行です。そして続きます。\nShort.\n\nMore text after a blank line, long enough to wrap on narrow screens.\n";

fn random_layers(rng: &mut Rng, chars: usize) -> Layers {
    let mut l = Layers::default();
    for k in 0..1 + rng.below(3) {
        let from = rng.below(chars as u64) as usize;
        let to = (from + 1 + rng.below(12) as usize).min(chars);
        let title = rng.chance(2).then_some("Title 見");
        let mut layer = Layer::new(Anchor::Text { from, to }).with_content(Content::hint(
            title,
            "A body with some words and 漢字 that wraps across lines.",
        ));
        if rng.chance(2) {
            layer = layer.with_arrow();
        }
        if rng.chance(2) {
            layer = layer.with_ring();
        }
        let agent = rng.chance(3);
        if !agent && rng.chance(3) {
            layer = layer.with_spotlight();
        }
        if rng.chance(4) {
            layer.place =
                vec![[Side::Below, Side::Above, Side::Right, Side::Left][rng.below(4) as usize]];
        }
        apply(
            &mut l,
            LayerOp::Push(layer),
            agent.then_some("helper"),
            k * 1_000,
            &Limits::default(),
        )
        .unwrap();
    }
    l
}

fn plan_for(s: &State, layers: &Layers) -> (Plan, Grid) {
    let frame = view(s);
    let res = FrameResolver::new(&frame).with_doc(&s.doc);
    let grid = Grid::from_frame(&frame);
    let r = common::renderers();
    let p = plan(layers, &res, &grid, &r);
    common::conform(layers, &res, &grid, &r, &p);
    (p, grid)
}

#[test]
fn random_plans_keep_every_invariant() {
    let chars = DOC.chars().count();
    for seed in 1..=SEEDS {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let w = 20 + rng.below(140) as u16;
        let h = 6 + rng.below(44) as u16;
        let mut s = State::new(
            DOC,
            Some("p.md".into()),
            Viewport {
                width: w,
                height: h,
            },
        );
        if rng.chance(3) {
            update(
                &mut s,
                Msg::ScrollView {
                    rows: rng.below(6) as i32,
                },
            );
        }
        let layers = random_layers(&mut rng, chars);
        let (p, grid) = plan_for(&s, &layers);
        let area = grid.area;
        assert_eq!(
            area.bottom(),
            h - 1,
            "seed {seed}: the status row is outside the area"
        );
        let mut before: Vec<Rect> = Vec::new();
        for l in &p.layers {
            let anchor = l
                .anchor
                .as_ref()
                .map(|a| a.rects.clone())
                .unwrap_or_default();
            if let Some(r) = l.rect {
                assert!(
                    r.bottom() <= area.bottom() && r.right() <= area.right(),
                    "seed {seed}: {r:?} leaves {area:?}"
                );
                assert!(
                    !grid.splits(&r),
                    "seed {seed}: {r:?} splits a wide grapheme"
                );
                if l.mode == Some(Mode::Box) {
                    for a in &anchor {
                        assert!(
                            !r.intersects(a),
                            "seed {seed}: box {r:?} covers its anchor {a:?}"
                        );
                    }
                    for b in &before {
                        assert!(
                            !r.intersects(b),
                            "seed {seed}: box {r:?} covers an earlier box or hole {b:?}"
                        );
                    }
                    if let (true, Some((cx, cy))) = (l.agent, grid.caret) {
                        assert!(
                            !r.contains(cx, cy),
                            "seed {seed}: an agent's box covers the caret"
                        );
                    }
                }
            }
            if let Some(c) = l.chip {
                assert!(!grid.splits(&c), "seed {seed}: chip splits a wide grapheme");
            }
            if let Some(rt) = &l.route {
                for s in &rt.steps {
                    assert!(
                        area.contains(s.x, s.y),
                        "seed {seed}: arrow outside the area"
                    );
                    assert!(
                        !matches!(grid.kind(s.x, s.y), CellKind::Wide | CellKind::WideTail),
                        "seed {seed}: arrow on a wide grapheme"
                    );
                }
            }
            before.extend(l.rect.filter(|_| l.mode == Some(Mode::Box)));
            before.extend(
                p.spots
                    .iter()
                    .filter(|s| s.layer == l.id)
                    .flat_map(|s| s.holes.iter().copied()),
            );
        }

        // Resize there and back: the same frame gives the same plan.
        let (w2, h2) = (20 + rng.below(140) as u16, 6 + rng.below(44) as u16);
        let mut s2 = s.clone();
        update(&mut s2, Msg::resize(w2, h2));
        plan_for(&s2, &layers);
        update(&mut s2, Msg::resize(w, h));
        if view(&s2) == view(&s) {
            assert_eq!(
                plan_for(&s2, &layers).0,
                p,
                "seed {seed}: the plan changed after a resize there and back"
            );
        }
    }
}
