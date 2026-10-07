#![cfg(feature = "caretline")]
//! Timings at 100x40 (design §3.6): `cargo test -p caretline-overlay --release --test bench -- --ignored --nocapture`.

mod common;

use std::time::Instant;

use caretline::view;
use caretline_overlay::*;
use common::*;

fn time(name: &str, layers: &Layers) {
    let s = state(100, 40);
    let frame = view(&s);
    let base = TestGrid::from_frame(&frame);
    let res = FrameResolver::new(&frame);
    let grid = Grid::scan(&base).with_area(Rect::new(0, 0, 100, 39));
    let opts = Opts::default();
    let n = 2_000;
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(plan(layers, &res, &grid, &TextMeasure(&opts)));
    }
    let plan_us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
    let t = Instant::now();
    for _ in 0..n {
        let mut cells = base.clone();
        let scene = layout(layers, &res, &grid, &opts);
        compose(&scene, &mut cells);
        std::hint::black_box(cells);
    }
    let all_us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(base.clone());
    }
    let clone_us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
    println!("{name}: plan {plan_us:.1} µs, layout + compose {:.1} µs", all_us - clone_us);
}

#[test]
#[ignore]
fn bench_100x40() {
    let mut hint = Layers::default();
    push(
        &mut hint,
        Layer::new(
            find("word at a time", 0),
            vec![callout("Jump by word", "Move one word at a time."), Item::Arrow(Arrow::default()), Item::Ring(Ring::default())],
        ),
        None,
    );
    time("hint", &hint);
    let mut spot = Layers::default();
    push(
        &mut spot,
        Layer::new(
            find("Hold Shift with any motion", 0),
            vec![
                callout("Select", "Shift with any motion selects."),
                Item::Arrow(Arrow::default()),
                Item::Spotlight(Spotlight::default()),
            ],
        ),
        None,
    );
    time("spotlight", &spot);
}
