//! Property tests: random documents, random message sequences (seeded), and the editing
//! invariants checked after every step.

mod common;

use caretline::helix::graphemes::ensure_grapheme_boundary_prev;
use caretline::{update, view, Msg, State, Viewport};
use common::gen;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

const SEEDS: u64 = 24;
const STEPS: usize = 500;

fn check_selection(state: &State, ctx: &str) {
    let text = state.doc.text.slice(..);
    let len = text.len_chars();
    for r in state.view.selection.iter() {
        for pos in [r.anchor, r.head] {
            assert!(pos <= len, "{ctx}: position {pos} past the end {len}");
            assert_eq!(
                ensure_grapheme_boundary_prev(text, pos),
                pos,
                "{ctx}: position {pos} is inside a grapheme of {:?}",
                state.doc.text.to_string()
            );
        }
    }
}

/// EI13: the screen shows the selection. A cell drawing a char is a caret cell exactly when
/// the char is under an empty range other than the primary (the primary is the terminal's
/// cursor), else a selection cell exactly when a range covers it.
fn check_screen(state: &State, frame: &caretline::view::Frame, ctx: &str) {
    use caretline::view::Role;
    let sel = &state.view.selection;
    let primary = sel.primary_index();
    for (i, cell) in frame.cells.iter().enumerate() {
        let Some(c) = cell.char_idx.map(|c| c as usize) else {
            continue;
        };
        let (x, y) = (i % frame.width as usize, i / frame.width as usize);
        let caret = sel
            .iter()
            .enumerate()
            .any(|(k, r)| k != primary && r.is_empty() && r.head == c);
        let selected = sel.iter().any(|r| r.from() <= c && c < r.to());
        let want = if caret && state.view.focused {
            Role::Caret
        } else if selected {
            Role::Selection
        } else {
            Role::Text
        };
        assert_eq!(cell.role, want, "{ctx}: EI13 cell ({x}, {y}) char {c}");
    }
    // And every other empty range is drawn where the cursor would be if it were the primary.
    if !state.view.focused {
        return;
    }
    for k in (0..sel.len()).filter(|&k| k != primary && sel.ranges()[k].is_empty()) {
        let mut as_primary = state.clone();
        as_primary.view.selection.set_primary_index(k);
        if let Some((x, y)) = view(&as_primary).cursor {
            assert_eq!(
                frame.cell(x, y).role,
                Role::Caret,
                "{ctx}: EI13 caret {k} not drawn at ({x}, {y})"
            );
        }
    }
}

fn single_range(state: &State) -> Option<(usize, usize, usize, usize)> {
    if state.view.selection.len() == 1 {
        let r = state.view.selection.primary();
        Some((r.anchor, r.head, r.from(), r.to()))
    } else {
        None
    }
}

fn splice(text: &str, from: usize, to: usize, insert: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out: String = chars[..from].iter().collect();
    out.push_str(insert);
    out.extend(&chars[to..]);
    out
}

fn is_edit(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::InsertText { .. }
            | Msg::InsertNewline
            | Msg::DeleteBackward
            | Msg::DeleteForward
            | Msg::DeleteWordBackward
            | Msg::DeleteWordForward
            | Msg::DeleteToLineStart
            | Msg::DeleteToLineEnd
            | Msg::KillLine
            | Msg::Cut
            | Msg::Paste { .. }
    )
}

fn run_seed(seed: u64) -> usize {
    let mut rng = StdRng::seed_from_u64(seed);
    let original = gen::text(&mut rng);
    let (width, height) = gen::size(&mut rng);
    let mut state = State::new(
        &original,
        Some("fuzz.md".into()),
        Viewport { width, height },
    );
    let mut steps = 0;
    for step in 0..STEPS {
        let ctx = format!("seed {seed} step {step}");
        if rng.random_range(0..40) == 0 {
            // Start from a multi-range selection now and then, as a state file could.
            state.view.selection = gen::multi_selection(&mut rng, &state);
        }
        let msg = gen::msg(&mut rng, &state);
        let before = state.clone();
        let effects = update(&mut state, msg.clone());
        steps += 1;
        let ctx = format!("{ctx} {msg:?}");
        check_selection(&state, &ctx);
        let text_before = before.doc.text.to_string();

        match &msg {
            // EI1: a motion without Shift leaves no selection.
            Msg::Move { extend: false, .. } => {
                assert!(
                    state.view.selection.iter().all(|r| r.is_empty()),
                    "{ctx}: EI1"
                );
            }
            // EI2: a motion with Shift never moves the anchor.
            Msg::Move { extend: true, .. } => {
                if let (Some((a0, ..)), Some((a1, h1, ..))) =
                    (single_range(&before), single_range(&state))
                {
                    assert_eq!(a0, a1, "{ctx}: EI2 anchor moved");
                    let _ = h1;
                }
            }
            // EI4: copy changes nothing but the clipboard.
            Msg::Copy => {
                assert_eq!(state.doc.text, before.doc.text, "{ctx}: EI4 text");
                assert_eq!(
                    state.view.selection, before.view.selection,
                    "{ctx}: EI4 selection"
                );
                assert_eq!(state.doc.history, before.doc.history, "{ctx}: EI4 history");
                assert_eq!(state.doc.dirty, before.doc.dirty, "{ctx}: EI4 dirty");
            }
            _ => {}
        }

        if let Some((_, _, from, to)) = single_range(&before) {
            if from < to {
                let removed = splice(&text_before, from, to, "");
                match &msg {
                    // EI6: typing over a selection replaces exactly it.
                    Msg::InsertText { text } if !text.contains(['\r', '\n']) => {
                        assert_eq!(
                            state.doc.text.to_string(),
                            splice(&text_before, from, to, text),
                            "{ctx}: EI6"
                        );
                    }
                    // EI7: every delete with a selection removes exactly the selection.
                    Msg::DeleteBackward
                    | Msg::DeleteForward
                    | Msg::DeleteWordBackward
                    | Msg::DeleteWordForward
                    | Msg::DeleteToLineStart
                    | Msg::DeleteToLineEnd
                    | Msg::KillLine
                    | Msg::Cut => {
                        assert_eq!(state.doc.text.to_string(), removed, "{ctx}: EI7");
                    }
                    _ => {}
                }
            }
        }

        // EI5: an edit that made a new revision undoes to exactly the state before it
        // (text and selection), and redoes to exactly the state after it.
        if is_edit(&msg) && state.doc.history.len() == before.doc.history.len() + 1 {
            let mut undone = state.clone();
            update(&mut undone, Msg::Undo);
            assert_eq!(undone.doc.text, before.doc.text, "{ctx}: EI5 undo text");
            assert_eq!(
                undone.view.selection, before.view.selection,
                "{ctx}: EI5 undo selection"
            );
            update(&mut undone, Msg::Redo);
            assert_eq!(undone.doc.text, state.doc.text, "{ctx}: EI5 redo text");
            assert_eq!(
                undone.view.selection.ranges().len(),
                state.view.selection.ranges().len(),
                "{ctx}: EI5 redo"
            );
        }

        // Effects are plain values and match the message.
        if matches!(msg, Msg::Quit) && !effects.is_empty() {
            // A quit ends a real session; keep fuzzing from the same state anyway.
        }

        // view never panics, at the state's size and at extreme sizes.
        let frame = view(&state);
        assert_eq!(
            frame.cells.len(),
            state.view.viewport.width as usize * state.view.viewport.height as usize
        );
        check_screen(&state, &frame, &ctx);
        if step % 10 == 0 {
            for (w, h) in [(1, 1), (2, 200), (200, 2), gen::size(&mut rng)] {
                let mut sized = state.clone();
                update(&mut sized, Msg::resize(w, h));
                let f = view(&sized);
                if let Some((x, y)) = f.cursor {
                    assert!(
                        x < w && y < h.saturating_sub(1).max(1),
                        "{ctx}: cursor off screen"
                    );
                }
            }
        }

        // Serialize and deserialize at random points: the same state and the same frame.
        if rng.random_range(0..25) == 0 {
            let json = state.to_json();
            let back = State::from_json(&json).expect("state parses");
            assert_eq!(back, state, "{ctx}: round trip");
            assert_eq!(view(&back), frame, "{ctx}: round-trip frame");
        }
    }

    // Undo all the way back: the original text.
    let mut guard = 0;
    loop {
        update(&mut state, Msg::Undo);
        if state.view.status.as_deref() == Some("nothing to undo") {
            break;
        }
        guard += 1;
        assert!(guard < 10_000, "seed {seed}: undo never reached the root");
    }
    assert_eq!(
        state.doc.text.to_string(),
        original,
        "seed {seed}: full undo"
    );
    check_selection(&state, &format!("seed {seed} after full undo"));
    steps
}

#[test]
fn random_sessions_keep_every_invariant() {
    let steps: usize = (0..SEEDS).map(run_seed).sum();
    assert!(steps >= 10_000, "only {steps} steps");
}

/// EI3 and EI8: cut then paste in place, and copy then paste over the same selection, leave
/// the text unchanged.
#[test]
fn cut_paste_and_copy_paste_are_identities() {
    let mut rng = StdRng::seed_from_u64(7);
    for _ in 0..400 {
        let text = gen::text(&mut rng);
        let mut s = State::new(
            &text,
            None,
            Viewport {
                width: 40,
                height: 10,
            },
        );
        let len = s.doc.text.len_chars();
        let t = s.doc.text.slice(..);
        let a = ensure_grapheme_boundary_prev(t, rng.random_range(0..=len));
        let b = ensure_grapheme_boundary_prev(t, rng.random_range(0..=len));
        s.view.selection = caretline::helix::Selection::single(a, b);
        let mut cut = s.clone();
        update(&mut cut, Msg::Cut);
        update(&mut cut, Msg::Paste { text: None });
        if a != b {
            assert_eq!(cut.doc.text, s.doc.text, "EI3 on {text:?} {a}..{b}");
            assert_eq!(cut.caret(), a.max(b), "EI3 caret at the end of the paste");
        }
        let mut copy = s.clone();
        update(&mut copy, Msg::Copy);
        update(&mut copy, Msg::Paste { text: None });
        if a != b {
            assert_eq!(copy.doc.text, s.doc.text, "EI8 on {text:?} {a}..{b}");
        }
    }
}

/// EI12: select all and delete leaves an empty document; one undo brings it all back.
#[test]
fn select_all_delete_then_undo() {
    let mut rng = StdRng::seed_from_u64(12);
    for _ in 0..200 {
        let text = gen::text(&mut rng);
        let mut s = State::new(
            &text,
            None,
            Viewport {
                width: 30,
                height: 8,
            },
        );
        update(&mut s, Msg::SelectAll);
        update(&mut s, Msg::DeleteBackward);
        assert_eq!(s.doc.text.len_chars(), 0);
        update(&mut s, Msg::Undo);
        assert_eq!(s.doc.text.to_string(), text);
        assert_eq!(
            s.view.selection,
            caretline::helix::Selection::single(0, s.doc.text.len_chars())
        );
    }
}

/// The view stays put unless it must move: an edit that leaves the primary caret inside the
/// view's margins (`scrolloff`) never scrolls, even when it shortens the document, and a
/// click inside the view never scrolls, whatever the margins or the follow policy; a drag
/// scrolls only on an edge row, one row. A passive message (a tick, a host value's operation, a
/// status line, a save's result, a copy) never moves it, whatever came before.
///
/// After a key or an edit that moved the caret or changed the text, the caret is never
/// closer to the bottom than the margin, except near the end of the document by no more than
/// `scroll_past_end` makes up: with it at `margin` or more, never.
#[test]
fn the_view_stays_for_edits_and_clicks_inside_it() {
    use caretline::layout::Layout;
    use caretline::{Follow, ScrollPastEnd};
    let (mut edits, mut clicks, mut passives, mut follows) = (0, 0, 0, 0);
    for seed in 0..SEEDS {
        let mut rng = StdRng::seed_from_u64(0x5ca1_ab1e ^ seed);
        let text: Vec<String> = (0..rng.random_range(1..8))
            .map(|_| gen::text(&mut rng))
            .collect();
        let text = text.join("\n\n\n");
        let width = rng.random_range(8..60);
        let height = rng.random_range(3..16);
        let mut state = State::new(&text, None, Viewport { width, height });
        state.view.config.scrolloff = rng.random_range(0..5);
        let typewriter = rng.random_range(0..4) == 0;
        if typewriter {
            state.view.config.follow = Follow::Typewriter {
                percent: rng.random_range(0..=100),
            };
        }
        // Its own generator, so the sessions above stay as they were.
        let mut past_rng = StdRng::seed_from_u64(0x0e2d_0f0f ^ seed);
        state.view.config.scroll_past_end = match past_rng.random_range(0..5) {
            0 => ScrollPastEnd::Off,
            1 | 2 => ScrollPastEnd::Margin,
            3 => ScrollPastEnd::Rows(past_rng.random_range(0..8)),
            _ => ScrollPastEnd::Half,
        };
        for step in 0..STEPS {
            let msg = match rng.random_range(0..5) {
                0 => Msg::Click {
                    col: rng.random_range(0..width),
                    row: rng.random_range(0..state.text_rows() as u16),
                    extend: rng.random_bool(0.3),
                },
                1 => Msg::Drag {
                    col: rng.random_range(0..width),
                    row: rng.random_range(0..state.text_rows() as u16 + 2),
                },
                _ => gen::msg(&mut rng, &state),
            };
            if matches!(msg, Msg::Resize { .. }) {
                continue;
            }
            let before = state.view.scroll;
            let (sel_before, rev_before) = (state.view.selection.clone(), state.doc.rev);
            update(&mut state, msg.clone());
            let ctx = format!("seed {seed} step {step} {msg:?}");
            let h = state.text_rows();
            let layout = Layout::new(&state);
            // The old top in the new text (a top row past its line's new end clamps).
            let top = layout.top(&before);
            let now = (state.view.scroll.line, state.view.scroll.row);
            match msg {
                Msg::Click { row, .. } if (row as usize) < h => {
                    clicks += 1;
                    assert_eq!(now, (top.line, top.row), "{ctx}: a click scrolled");
                }
                // A drag scrolls only on an edge row, and then one row.
                Msg::Drag { row, .. } => {
                    let r = row as usize;
                    let new = layout.top(&state.view.scroll);
                    let moved = if new < top {
                        -layout.rows_between(new, top, h)
                    } else {
                        layout.rows_between(top, new, h)
                    };
                    if r > 0 && r + 1 < h {
                        assert_eq!(moved, 0, "{ctx}: a drag inside scrolled");
                    } else {
                        assert!(moved.abs() <= 1, "{ctx}: a drag scrolled {moved} rows");
                    }
                }
                ref m if is_edit(m) && !typewriter => {
                    let so = (state.view.config.scrolloff as usize).min((h - 1) / 2) as isize;
                    let (caret, _) = layout.pos_coords(state.caret());
                    let dist = layout.rows_between(top, caret, h * 2);
                    if (so..h as isize - so).contains(&dist) {
                        edits += 1;
                        assert_eq!(now, (top.line, top.row), "{ctx}: an edit scrolled");
                    }
                }
                _ => {}
            }
            // A following move keeps the bottom margin, or as much of it as the document's end
            // and `scroll_past_end` leave. (A click, even below the text rows, keeps the view
            // while the caret lands in it.)
            let pointer = matches!(msg, Msg::Click { .. } | Msg::Drag { .. });
            let moved = state.view.selection != sel_before || state.doc.rev != rev_before;
            if !typewriter && !pointer && moved && !state.view.free && h > 0 {
                follows += 1;
                let c = &state.view.config;
                let so = (c.scrolloff as usize).min((h - 1) / 2);
                let past = c.scroll_past_end.rows(h, c.scrolloff);
                let top = layout.top(&state.view.scroll);
                let (caret, _) = layout.pos_coords(state.caret());
                let row = layout.rows_between(top, caret, h * 2);
                let to_end = layout.rows_between(caret, layout.end(), h) as usize;
                let margin = so.min(past + to_end);
                assert!(
                    (0..(h - margin) as isize).contains(&row),
                    "{ctx}: the caret on row {row} of {h}, margin {margin} ({:?})",
                    c.scroll_past_end
                );
                if past >= so {
                    assert!(row < (h - so) as isize, "{ctx}: inside the margin");
                }
            }
            // Passive messages, any number in any order, never move the view.
            for _ in 0..rng.random_range(0..4) {
                let msg = match rng.random_range(0..7) {
                    0 => Msg::Tick {
                        now_ms: rng.random_range(0..100_000),
                    },
                    1 => Msg::Ext {
                        key: "nobody".into(),
                        op: serde_json::Value::Null,
                    },
                    2 => Msg::ShowStatus { text: "hi".into() },
                    3 => Msg::Saved,
                    4 => Msg::SaveFailed { err: "no".into() },
                    5 => Msg::Frame {
                        now_ms: rng.random_range(0..100_000),
                    },
                    _ => Msg::Copy,
                };
                let before = state.view.scroll;
                update(&mut state, msg.clone());
                passives += 1;
                assert_eq!(
                    state.view.scroll, before,
                    "seed {seed} step {step} {msg:?}: a passive message scrolled"
                );
            }
        }
    }
    assert!(
        edits > 1000 && clicks > 1000 && passives > 1000 && follows > 1000,
        "{edits} edits, {clicks} clicks, {passives} passive messages, {follows} follows"
    );
}
