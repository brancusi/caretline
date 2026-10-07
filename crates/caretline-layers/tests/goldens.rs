#![cfg(feature = "caretline")]
//! Golden plans over a caretline frame at 80x24 and 44x16: the plan as JSON (what a host
//! draws from), and a picture a plain-text test host draws from it (`#` box, `=` strip, `c`
//! chip, `>` arrow, `*` ring, `.` dimmed, `s` status). `LAYERS_GOLDENS=update cargo test -p
//! caretline-layers` rewrites them; review the diff by eye.

mod common;

use caretline::Msg;
use caretline_layers::*;
use common::*;

const SIZES: [(u16, u16); 2] = [(80, 24), (44, 16)];

fn each(name: &str, setup: impl Fn(&mut caretline::State) -> Layers) {
    for (w, h) in SIZES {
        let mut s = state(w, h);
        let layers = setup(&mut s);
        let (frame, p) = plan_over(&s, &layers);
        golden(
            &format!("{name}.{w}x{h}.txt"),
            &picture(&frame, &p, &layers),
        );
        golden(
            &format!("{name}.{w}x{h}.json"),
            &(serde_json::to_string_pretty(&p).unwrap() + "\n"),
        );
    }
}

fn word_hint() -> Layers {
    let mut l = Layers::default();
    let layer = Layer::new(find("word at a time", 0))
        .with_content(hint(
            "Jump by word",
            "Option and an arrow key move one word at a time.",
        ))
        .with_arrow()
        .with_ring();
    push(&mut l, layer, None);
    l
}

fn spotlight() -> Layers {
    let mut l = Layers::default();
    let mut layer = Layer::new(find("Hold Shift with any motion to select as you move.", 0))
        .with_content(hint(
            "Select as you move",
            "Shift with any motion selects. Try it on the lit line.",
        ))
        .with_arrow()
        .with_ring()
        .with_spotlight();
    layer.owner = Owner::Guide;
    push(&mut l, layer, None);
    l
}

fn agent_hint() -> Layers {
    let mut l = Layers::default();
    let layer = Layer::new(find("Every change can be undone.", 0))
        .with_content(hint("tip", "Each undo step is one word while you type."))
        .with_arrow();
    push(&mut l, layer, Some("helper"));
    l
}

fn off_screen() -> Layers {
    let mut l = Layers::default();
    let mut layer = Layer::new(find("Control S", 0))
        .with_content(hint("Save", "Control S saves."))
        .with_ring();
    layer.owner = Owner::Guide;
    push(&mut l, layer, None);
    l
}

#[test]
fn a_box_and_an_arrow_at_a_word() {
    each("word", |_| word_hint());
}

#[test]
fn a_spotlight_leaves_the_anchor_and_box_lit() {
    each("spotlight", |_| spotlight());
}

#[test]
fn an_agent_hint_is_attributed_and_never_dims() {
    each("agent", |_| agent_hint());
}

#[test]
fn an_off_screen_anchor_gets_an_edge_chip() {
    each("offscreen", |_| off_screen());
}

#[test]
fn scrolled_into_view_the_chip_gives_way_to_the_ring() {
    each("offscreen.scrolled", |s| {
        let rows = s.text_rows() as i32;
        caretline::update(s, Msg::ScrollView { rows });
        off_screen()
    });
}

#[test]
fn a_plan_from_a_state_round_tripped_through_serde_is_identical() {
    for layers in [word_hint(), spotlight(), agent_hint(), off_screen()] {
        let s = state(80, 24);
        let (_, a) = plan_over(&s, &layers);
        let back: Layers = serde_json::from_str(&serde_json::to_string(&layers).unwrap()).unwrap();
        let (_, b) = plan_over(&s, &back);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        // And the plan itself survives JSON.
        let p: Plan = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(p, a);
    }
}
