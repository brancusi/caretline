#![cfg(feature = "caretline")]
//! Golden frames: overlays composed over a caretline frame, as text and as a role and flag map
//! (`#` callout, `>` arrow, `k` key badge, `o` dots, `=` strip, `c` chip, `@` an agent's
//! callout, `*` ringed, `.` dimmed, `s` status). Each at 80x24 and 44x16, in rounded and ASCII
//! glyphs. `OVERLAY_GOLDENS=update cargo test -p caretline-overlay` rewrites them; review the
//! diff by eye.

mod common;

use caretline::Msg;
use caretline_overlay::*;
use common::*;

const SIZES: [(u16, u16); 2] = [(80, 24), (44, 16)];
const GLYPHS: [(Glyphs, &str); 2] = [(Glyphs::Rounded, "rounded"), (Glyphs::Ascii, "ascii")];

fn each(name: &str, setup: impl Fn(&mut caretline::State) -> Layers) {
    for (w, h) in SIZES {
        for (g, gn) in GLYPHS {
            let mut s = state(w, h);
            let layers = setup(&mut s);
            let (grid, _) = draw(&s, &layers, g);
            frame_golden(&format!("{name}.{w}x{h}.{gn}.txt"), &grid);
        }
    }
}

fn word_hint() -> Layers {
    let mut l = Layers::default();
    push(
        &mut l,
        Layer::new(
            find("word at a time", 0),
            vec![
                callout(
                    "Jump by word",
                    "{{key:move.word_left}} and {{key:move.word_right}} move one word at a time. Hold {{key:select.shift}} to select as you go.",
                ),
                Item::Arrow(Arrow::default()),
                Item::Ring(Ring::default()),
            ],
        ),
        None,
    );
    l
}

fn spotlight() -> Layers {
    let mut l = Layers::default();
    let mut layer = Layer::new(
        find("Hold Shift with any motion to select as you move.", 0),
        vec![
            Item::Spotlight(Spotlight::default()),
            callout(
                "Select as you move",
                "Shift with any motion selects. Try it on the lit line.",
            ),
            Item::Arrow(Arrow::default()),
            Item::Ring(Ring::default()),
        ],
    );
    layer.owner = Owner::Guide;
    push(&mut l, layer, None);
    l
}

fn agent_hint() -> Layers {
    let mut l = Layers::default();
    push(
        &mut l,
        Layer::new(
            find("Every change can be undone.", 0),
            vec![
                callout("hint", "Each undo step is one word while you type."),
                Item::Arrow(Arrow::default()),
            ],
        ),
        Some("helper"),
    );
    l
}

fn step() -> Layers {
    let mut l = Layers::default();
    let mut layer = Layer::new(
        find("jump a word", 0),
        vec![
            Item::Callout(Callout {
                title: Some("Moving by words".into()),
                body: "{{key:move.word_left}} {{key:move.word_right}} jump by word.".into(),
                chips: vec![Chip {
                    id: "next".into(),
                    text: "{{key:guide.next}} next".into(),
                }],
                ..Callout::default()
            }),
            Item::Steps { of: 6, at: 2 },
            Item::Arrow(Arrow::default()),
            Item::Ring(Ring::default()),
        ],
    );
    layer.owner = Owner::Guide;
    push(&mut l, layer, None);
    l
}

fn off_screen() -> Layers {
    let mut l = Layers::default();
    let mut layer = Layer::new(
        find("Control S", 0),
        vec![
            callout(
                "Save",
                "Press {{key:search.open}} to search, and Control S to save.",
            ),
            Item::Steps { of: 6, at: 6 },
            Item::Ring(Ring::default()),
        ],
    );
    layer.owner = Owner::Guide;
    push(&mut l, layer, None);
    l
}

#[test]
fn callout_with_an_arrow_at_a_word() {
    each("word", |_| word_hint());
}

#[test]
fn spotlight_dims_all_but_the_anchor_and_callout() {
    each("spotlight", |_| spotlight());
}

#[test]
fn agent_hint_is_attributed_and_never_dims() {
    each("agent", |_| agent_hint());
}

#[test]
fn walkthrough_step_with_dots_and_a_chip() {
    each("step", |_| step());
}

#[test]
fn off_screen_anchor_gets_an_edge_chip() {
    each("offscreen", |_| off_screen());
}

#[test]
fn scrolled_into_view_the_chip_gives_way_to_the_ring() {
    each("offscreen.scrolled", |s| {
        let rows = s.text_rows() as u16;
        caretline::update(s, Msg::ScrollView { rows: rows as i32 });
        off_screen()
    });
}

#[test]
fn scene_json() {
    let s = state(80, 24);
    let (_, scene) = draw(&s, &word_hint(), Glyphs::Rounded);
    golden(
        "word.80x24.scene.json",
        &(serde_json::to_string_pretty(&scene).unwrap() + "\n"),
    );
    let (_, scene) = draw(&s, &spotlight(), Glyphs::Rounded);
    golden(
        "spotlight.80x24.scene.json",
        &(serde_json::to_string_pretty(&scene).unwrap() + "\n"),
    );
    let s = state(44, 16);
    let (_, scene) = draw(&s, &off_screen(), Glyphs::Rounded);
    golden(
        "offscreen.44x16.scene.json",
        &(serde_json::to_string_pretty(&scene).unwrap() + "\n"),
    );
}
