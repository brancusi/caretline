#![cfg(feature = "caretline")]
//! Properties over random sizes, anchors and layers (a seeded PRNG, so a failure names its
//! seed and replays): a box never covers its anchor, another layer's hole or the status row;
//! an agent's box never covers the caret; nothing splits a wide grapheme; layout is the same
//! after a resize there and back.

use caretline::{Msg, State, Viewport, update, view};
use caretline_overlay::*;

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
    let n = 1 + rng.below(3);
    for k in 0..n {
        let from = rng.below(chars as u64) as usize;
        let to = (from + 1 + rng.below(12) as usize).min(chars);
        let mut items = vec![Item::Callout(Callout {
            title: rng.chance(2).then(|| "Title 見".to_string()),
            body: "A body with some words and 漢字 that wraps across lines.".into(),
            ..Callout::default()
        })];
        if rng.chance(2) {
            items.push(Item::Arrow(Arrow::default()));
        }
        if rng.chance(2) {
            items.push(Item::Ring(Ring::default()));
        }
        let agent = rng.chance(3);
        if !agent && rng.chance(3) {
            items.push(Item::Spotlight(Spotlight::default()));
        }
        let mut layer = Layer::new(Anchor::Text { from, to }, items);
        if rng.chance(4) {
            layer.place =
                vec![[Side::Below, Side::Above, Side::Right, Side::Left][rng.below(4) as usize]];
        }
        let actor = agent.then_some("helper");
        apply(
            &mut l,
            LayerOp::Push(layer),
            actor,
            k * 1_000,
            &Limits::default(),
        )
        .unwrap();
    }
    l
}

fn render(s: &State, layers: &Layers, glyphs: Glyphs) -> (TestGrid, Scene, Grid) {
    let frame = view(s);
    let mut cells = TestGrid::from_frame(&frame);
    let res = FrameResolver::new(&frame).with_doc(&s.doc);
    let grid = Grid::scan(&cells)
        .with_area(Rect::new(0, 0, frame.width, s.text_rows() as u16))
        .with_caret(frame.cursor);
    let scene = layout(layers, &res, &grid, &Opts::new(glyphs));
    compose(&scene, &mut cells);
    (cells, scene, grid)
}

fn check_wide(seed: u64, g: &TestGrid) {
    for y in 0..g.height {
        let mut x = 0;
        while x < g.width {
            let s = &g.cell(x, y).symbol;
            if width(s) >= 2 {
                assert!(
                    x + 1 < g.width && g.cell(x + 1, y).symbol.is_empty(),
                    "seed {seed}: wide {s:?} at {x},{y} lost its second half"
                );
                x += 2;
                continue;
            }
            assert!(
                !s.is_empty(),
                "seed {seed}: a second half with no first at {x},{y}"
            );
            x += 1;
        }
    }
}

#[test]
fn random_layouts_keep_every_invariant() {
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
        let glyphs = if rng.chance(2) {
            Glyphs::Rounded
        } else {
            Glyphs::Ascii
        };
        let (cells, scene, grid) = render(&s, &layers, glyphs);
        let area = grid.area;
        let mut holes_before: Vec<Rect> = Vec::new();
        for p in &scene.layers {
            let anchor = p
                .anchor
                .as_ref()
                .map(|a| a.rects.clone())
                .unwrap_or_default();
            for panel in &p.panels {
                let r = panel.rect;
                assert!(
                    r.bottom() <= area.bottom() && r.right() <= area.right(),
                    "seed {seed}: {r:?} leaves the area {area:?}"
                );
                if panel.kind == PanelKind::Callout {
                    for a in &anchor {
                        assert!(
                            !r.intersects(a),
                            "seed {seed}: callout {r:?} covers its anchor {a:?}"
                        );
                    }
                    for hl in &holes_before {
                        assert!(
                            !r.intersects(hl),
                            "seed {seed}: callout {r:?} covers a hole {hl:?}"
                        );
                    }
                    if p.agent
                        && let Some((cx, cy)) = grid.caret
                    {
                        assert!(
                            !r.contains(cx, cy),
                            "seed {seed}: an agent's callout covers the caret"
                        );
                    }
                }
            }
            if let Some(a) = &p.arrow {
                for c in &a.cells {
                    assert!(
                        area.contains(c.x, c.y),
                        "seed {seed}: arrow outside the area"
                    );
                    assert!(
                        grid.kind(c.x, c.y) != CellKind::Wide,
                        "seed {seed}: arrow on a wide grapheme"
                    );
                }
            }
            holes_before.extend(
                scene
                    .spots
                    .iter()
                    .filter(|s| s.layer == p.id)
                    .flat_map(|s| s.holes.iter().copied()),
            );
        }
        // The status row is untouched.
        let status = h - 1;
        for x in 0..w {
            let c = cells.cell(x, status);
            assert!(
                !c.role.starts_with("overlay."),
                "seed {seed}: overlay on the status row"
            );
        }
        check_wide(seed, &cells);

        // Resize there and back: the same frame gives the same scene.
        let (w2, h2) = (20 + rng.below(140) as u16, 6 + rng.below(44) as u16);
        let mut s2 = s.clone();
        update(
            &mut s2,
            Msg::Resize {
                width: w2,
                height: h2,
            },
        );
        let (other, _, _) = render(&s2, &layers, glyphs);
        check_wide(seed, &other);
        update(
            &mut s2,
            Msg::Resize {
                width: w,
                height: h,
            },
        );
        if view(&s2) == view(&s) {
            let (_, again, _) = render(&s2, &layers, glyphs);
            assert_eq!(
                again, scene,
                "seed {seed}: layout changed after a resize there and back"
            );
        }
    }
}

#[test]
fn writing_over_either_half_of_a_wide_grapheme_blanks_the_other() {
    let mut g = TestGrid::from_text(10, 1, "a漢b");
    put(&mut g, 2, 0, "x", "overlay.callout");
    assert_eq!(g.to_text(), "a x\n".replace(" x", " xb"));
    let mut g = TestGrid::from_text(10, 1, "a漢b");
    put(&mut g, 1, 0, "x", "overlay.callout");
    assert_eq!(g.to_text(), "ax b\n");
    // A wide grapheme that doesn't fit is a space.
    let mut g = TestGrid::new(3, 1);
    assert_eq!(put(&mut g, 2, 0, "漢", "overlay.callout"), 1);
    assert_eq!(g.to_text(), "\n");
}
