#![cfg(feature = "caretline")]
//! Timings at 100x40 (design §3.5, §12):
//! `cargo test -p caretline-layers --release --test bench -- --ignored --nocapture`
//! (`BENCH_N` plans per case, default 5,000; more gives steadier numbers).

mod common;

use std::time::Instant;

use caretline::view;
use caretline_layers::*;
use common::*;

fn time(name: &str, layers: &Layers) {
    let s = state(100, 40);
    let frame = view(&s);
    let res = FrameResolver::new(&frame);
    let grid = Grid::from_frame(&frame);
    let r = renderers();
    let n: usize = std::env::var("BENCH_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5_000);
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(plan(layers, &res, &grid, &r));
    }
    println!(
        "{name}: plan {:.1} µs",
        t.elapsed().as_secs_f64() * 1e6 / n as f64
    );
}

#[test]
#[ignore]
fn bench_100x40() {
    let mut hint_l = Layers::default();
    push(
        &mut hint_l,
        Layer::new(find("word at a time", 0))
            .with_content(hint("Jump by word", "Move one word at a time."))
            .with_arrow()
            .with_ring(),
        None,
    );
    time("hint", &hint_l);
    let mut plain = Layers::default();
    push(
        &mut plain,
        Layer::new(find("word at a time", 0))
            .with_content(hint("Jump by word", "Move one word at a time.")),
        None,
    );
    time("box only", &plain);
    let mut spot = Layers::default();
    push(
        &mut spot,
        Layer::new(find("Hold Shift with any motion", 0))
            .with_content(hint("Select", "Shift with any motion selects."))
            .with_arrow()
            .with_spotlight(),
        None,
    );
    time("spotlight", &spot);
}
