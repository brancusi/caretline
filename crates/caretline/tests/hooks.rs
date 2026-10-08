//! The generic hooks a host builds on: view values (`View::ext`) and their reducers
//! (`Msg::Ext`, `Host::ext`). The test host is made up (a counter and a pin that follows the
//! text): nothing here means anything to the engine.

use std::sync::{Arc, Mutex};

use caretline::trace::{replay_trace_with, TraceLine};
use caretline::view::{
    hit, locate, render_plain, render_skipping, CellFlags, Frame, Hit, Locate, Region, Role,
};
use caretline::{
    update, update_doc_with_changes, update_with_changes, CellPx, ChangeSet, Effect, ExtChange,
    ExtFns, ExtOut, Host, Msg, Session, State, View, Viewport,
};
use serde_json::{json, Value};

mod common;

fn vp() -> Viewport {
    Viewport {
        width: 40,
        height: 8,
    }
}

/// `count`: `{"add": n}` adds to the value (from 0), `{"clear": true}` removes it, `{"fail":
/// true}` refuses, `{"fps": n}` asks for a frame clock and emits `counted`.
fn count(_: &caretline::Ctx, current: Option<&Value>, op: &Value) -> Result<ExtOut, String> {
    if op.get("fail").is_some() {
        return Err("the counter refuses".into());
    }
    if op.get("clear").is_some() {
        return Ok(ExtOut::remove());
    }
    let n = current.and_then(Value::as_i64).unwrap_or(0) + op["add"].as_i64().unwrap_or(1);
    let mut out = ExtOut::value(json!(n)).with_effect("counted", json!(n));
    if let Some(fps) = op.get("fps").and_then(Value::as_u64) {
        out = out.with_frame_clock(fps as u16);
    }
    Ok(out)
}

/// `pin`: `{"at": pos, "until": ms}` pins a char position, which the observer maps through every
/// edit and drops once the clock reaches `until`.
fn pin_host() -> Host {
    Host::new().ext(
        "pin",
        ExtFns::new(|_, _, op| Ok(ExtOut::value(op.clone()))).with_observe(|_, value, seen| {
            let until = value["until"].as_u64().unwrap_or(u64::MAX);
            if let Msg::Tick { now_ms } = seen.msg {
                if *now_ms >= until {
                    return Some(ExtOut::remove().with_status("unpinned"));
                }
            }
            let cs = seen.changes?;
            let at = value["at"].as_u64().unwrap() as usize;
            let mut v = value.clone();
            v["at"] = json!(cs.map_pos(at, caretline::Assoc::Before));
            Some(ExtOut::value(v))
        }),
    )
}

fn host() -> Host {
    Host::new().ext("count", ExtFns::new(count))
}

fn ext(key: &str, op: Value) -> Msg {
    Msg::Ext {
        key: key.into(),
        op,
    }
}

fn with_host(text: &str, host: Host) -> State {
    let mut s = State::new(text, None, vp());
    s.doc.set_host(host);
    s
}

// --------------------------------------------------------------------------------------
// E1: view values

#[test]
fn view_values_are_serialized_and_left_out_when_empty() {
    let mut s = State::new("hello", None, vp());
    let json = s.to_json();
    assert!(!json.contains("\"ext\""), "{json}");
    s.view.ext.insert("count".into(), json!({"n": 3}));
    let json = s.to_json();
    assert!(json.contains("\"ext\""), "{json}");
    let back = State::from_json(&json).unwrap();
    assert_eq!(back.view.ext, s.view.ext);
    assert_eq!(back, s);
    // Without the history too, and a view on its own (view.open, trace lines).
    let v: Value = serde_json::to_value(s.without_history()).unwrap();
    assert_eq!(v["ext"]["count"]["n"], 3);
    let view: View = serde_json::from_value(serde_json::to_value(&s.view).unwrap()).unwrap();
    assert_eq!(view.ext, s.view.ext);
    let plain: Value = serde_json::to_value(View::new(vp())).unwrap();
    assert!(plain.get("ext").is_none());
}

#[test]
fn the_engine_never_reads_view_values() {
    // Any value under any key: editing and drawing are the same with or without it.
    let mut a = State::new("one two three", None, vp());
    let mut b = a.clone();
    b.view
        .ext
        .insert("anything".into(), json!([1, {"x": null}]));
    for msg in [
        Msg::InsertText { text: "x".into() },
        Msg::Move {
            dir: caretline::Dir::Forward,
            by: caretline::By::Word,
            extend: true,
        },
        Msg::Cut,
        Msg::Undo,
    ] {
        assert_eq!(update(&mut a, msg.clone()), update(&mut b, msg));
    }
    assert_eq!(caretline::view(&a), caretline::view(&b));
    assert_eq!(a.doc, b.doc);
}

#[test]
fn view_values_go_through_the_protocol_and_view_open() {
    let mut s = State::new("hello", None, vp());
    s.view.ext.insert("count".into(), json!(2));
    let mut session = Session::new(s);
    let r: Value = serde_json::from_str(
        &session
            .handle(r#"{"id":1,"op":"state.get"}"#, None)
            .response,
    )
    .unwrap();
    assert_eq!(r["result"]["state"]["ext"]["count"], 2);
    let r: Value = serde_json::from_str(
        &session
            .handle(
                r#"{"id":2,"op":"view.open","open":{"ext":{"count":5}}}"#,
                None,
            )
            .response,
    )
    .unwrap();
    let id = r["result"]["view"].as_u64().unwrap() as u32;
    assert_eq!(session.view(id).unwrap().ext["count"], 5);
    let (state, views, _) = replay_trace_with(&session.trace_jsonl(), &Host::new()).unwrap();
    assert_eq!(state.view.ext["count"], 2);
    assert_eq!(views[0].1.ext["count"], 5);
    // state.set keeps what it is given.
    let r = session.handle(
        r#"{"id":3,"op":"state.set","state":{"text":"x","ext":{"k":[1]}}}"#,
        None,
    );
    assert!(r.response.contains("\"rev\""), "{}", r.response);
    assert_eq!(session.state().view.ext["k"], json!([1]));
}

// --------------------------------------------------------------------------------------
// E2: Msg::Ext and ext reducers

#[test]
fn ext_applies_the_reducer_on_the_acting_view() {
    let mut s = with_host("hello", host());
    let fx = update(&mut s, ext("count", json!({"add": 2})));
    assert_eq!(s.view.ext["count"], 2);
    assert_eq!(
        fx,
        vec![Effect::Host {
            name: "counted".into(),
            data: json!(2)
        }]
    );
    update(&mut s, ext("count", json!({"add": 3, "fps": 30})));
    assert_eq!(s.view.ext["count"], 5);
    assert_eq!(s.view.frame_rate(), Some(30));
    update(&mut s, ext("count", json!({"clear": true})));
    assert!(s.view.ext.is_empty());
    // No text, no history, no rev.
    assert_eq!(s.doc.rev, 0);
    assert!(!s.doc.dirty);
}

#[test]
fn ext_without_a_reducer_or_refused_says_so() {
    let mut s = with_host("hello", host());
    update(&mut s, ext("nobody", json!(1)));
    assert_eq!(s.view.status.as_deref(), Some("no ext 'nobody'"));
    assert!(s.view.ext.is_empty());
    update(&mut s, ext("count", json!({"fail": true})));
    assert_eq!(s.view.status.as_deref(), Some("the counter refuses"));
    assert!(s.view.ext.is_empty());
}

#[test]
fn ext_is_passive() {
    let mut s = with_host("", host());
    update(&mut s, Msg::Tick { now_ms: 1000 });
    update(&mut s, Msg::InsertText { text: "a".into() });
    update(
        &mut s,
        Msg::ShowStatus {
            text: "keep me".into(),
        },
    );
    update(&mut s, ext("count", json!({"add": 1})));
    // The status stays, and the typing run goes on: one undo takes both letters.
    assert_eq!(s.view.status.as_deref(), Some("keep me"));
    update(&mut s, Msg::InsertText { text: "b".into() });
    assert_eq!(s.doc.text.to_string(), "ab");
    update(&mut s, Msg::Undo);
    assert_eq!(s.doc.text.to_string(), "");
}

#[test]
fn ext_is_accepted_on_a_read_only_view() {
    let mut s = with_host("hello", host());
    s.view.read_only = true;
    assert_eq!(
        update(&mut s, Msg::InsertText { text: "x".into() }),
        vec![Effect::Refused]
    );
    let fx = update(&mut s, ext("count", json!({"add": 4})));
    assert_eq!(s.view.ext["count"], 4);
    assert!(!fx.contains(&Effect::Refused));
}

#[test]
fn ext_round_trips_as_json() {
    let m = ext("count", json!({"add": 2}));
    let line = serde_json::to_string(&m).unwrap();
    assert_eq!(line, r#"{"msg":"ext","key":"count","op":{"add":2}}"#);
    assert_eq!(serde_json::from_str::<Msg>(&line).unwrap(), m);
    let bare: Msg = serde_json::from_str(r#"{"msg":"ext","key":"k"}"#).unwrap();
    assert_eq!(
        bare,
        Msg::Ext {
            key: "k".into(),
            op: Value::Null
        }
    );
}

#[test]
fn a_trace_with_ext_replays_identically_with_the_host() {
    let host = host().ext("pin", ExtFns::new(|_, _, op| Ok(ExtOut::value(op.clone()))));
    let mut s = State::new("hello world", None, vp());
    s.doc.set_host(host.clone());
    let mut session = Session::new(s);
    let msgs = vec![
        ext("count", json!({"add": 2})),
        Msg::InsertText {
            text: "say ".into(),
        },
        ext("count", json!({"add": 5})),
        ext("pin", json!({"at": 3})),
        Msg::Undo,
    ];
    let mut effects = Vec::new();
    for m in msgs {
        effects.extend(session.apply(m));
    }
    let opened = session.open_view(View::new(vp()));
    session.apply_on(opened, ext("count", json!({"add": 1})));
    let trace = session.trace_jsonl();
    assert!(trace.contains(r#""msg":"ext""#), "{trace}");
    let (state, views, n) = replay_trace_with(&trace, &host).unwrap();
    assert_eq!(n, 6);
    assert_eq!(state.to_json(), session.state().to_json());
    assert_eq!(state.view.ext["count"], 7);
    assert_eq!(views[0].1.ext["count"], 1);
    // Without the host, the values stay as they were and the status says why.
    let (bare, _, _) = replay_trace_with(&trace, &Host::new()).unwrap();
    assert!(bare.view.ext.is_empty());
    // A checkpoint's state line carries the values at its start.
    session.checkpoint();
    let TraceLine::State(seg) = &session.segment_trace()[0] else {
        panic!("a state line");
    };
    assert_eq!(seg.view.ext["count"], 7);
}

#[test]
fn observe_maps_and_expires_its_value() {
    let mut s = with_host("hello world", pin_host());
    update(&mut s, ext("pin", json!({"at": 6, "until": 5000})));
    s.view.selection = caretline::helix::Selection::point(0);
    update(&mut s, Msg::InsertText { text: ">> ".into() });
    assert_eq!(s.view.ext["pin"]["at"], 9);
    update(&mut s, Msg::Undo);
    assert_eq!(s.view.ext["pin"]["at"], 6);
    update(
        &mut s,
        Msg::External {
            changes: vec![ExtChange::Replace {
                from: 0,
                to: 0,
                text: "abc".into(),
            }],
        },
    );
    assert_eq!(s.view.ext["pin"]["at"], 9);
    update(&mut s, Msg::Tick { now_ms: 4999 });
    assert!(s.view.ext.contains_key("pin"));
    update(&mut s, Msg::Tick { now_ms: 5000 });
    assert!(!s.view.ext.contains_key("pin"));
    assert_eq!(s.view.status.as_deref(), Some("unpinned"));
}

/// Records what the observer saw (a test's way to look inside; real observers are pure).
type Seen = Arc<Mutex<Vec<(Option<ChangeSet>, bool, usize)>>>;

fn spy() -> (Host, Seen) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let host = Host::new().ext(
        "spy",
        ExtFns::new(|_, _, _| Ok(ExtOut::value(json!(true)))).with_observe(move |_, _, o| {
            log.lock()
                .unwrap()
                .push((o.changes.cloned(), o.acting, o.effects.len()));
            None
        }),
    );
    (host, seen)
}

#[test]
fn observe_gets_the_changes_update_with_changes_returns() {
    let (host, seen) = spy();
    let mut s = with_host("one two\nthree", host);
    update(&mut s, ext("spy", Value::Null));
    seen.lock().unwrap().clear();
    let msgs = vec![
        Msg::InsertText { text: "x".into() },
        Msg::InsertNewline,
        Msg::SelectAll,
        Msg::Cut,
        Msg::Undo,
        Msg::Redo,
        Msg::Paste {
            text: Some("a\nb".into()),
        },
        Msg::External {
            changes: vec![ExtChange::Replace {
                from: 0,
                to: 1,
                text: "Z".into(),
            }],
        },
        Msg::Tick { now_ms: 10 },
        Msg::Copy,
    ];
    let mut returned = Vec::new();
    for m in msgs {
        let (fx, cs) = update_with_changes(&mut s, m);
        returned.push((cs, fx.len()));
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), returned.len());
    for (i, ((cs, acting, n), (want, fx))) in seen.iter().zip(&returned).enumerate() {
        assert_eq!(cs, want, "message {i}");
        assert_eq!(n, fx, "message {i}");
        // A change from elsewhere goes through no view.
        assert_eq!(*acting, i != 7, "message {i}");
    }
    // `update` (no changes asked for) still hands observers the changes.
    let (host, seen) = spy();
    let mut s = with_host("abc", host);
    update(&mut s, ext("spy", Value::Null));
    update(&mut s, Msg::InsertText { text: "x".into() });
    assert!(seen.lock().unwrap().last().unwrap().0.is_some());
}

#[test]
fn observe_runs_on_every_view_holding_the_key() {
    let (host, seen) = spy();
    let mut doc = caretline::Document::new("hello", None);
    doc.set_host(host);
    let mut views = vec![View::new(vp()), View::new(vp()), View::new(vp())];
    views[1].ext.insert("spy".into(), json!(true));
    let (_, cs) = update_doc_with_changes(
        &mut doc,
        &mut views,
        0,
        Msg::InsertText { text: "x".into() },
    );
    let seen = seen.lock().unwrap();
    // Only view 1 holds the key: it sees view 0's edit, not as its own.
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, cs);
    assert!(!seen[0].1);
}

// --------------------------------------------------------------------------------------
// E6: the cell pixel size is a message

#[test]
fn resize_carries_the_cell_size_into_the_state_and_the_frame() {
    let mut s = State::new("hello", None, vp());
    assert_eq!(caretline::view(&s).cell_px, None);
    let px = CellPx::new(8, 16);
    update(
        &mut s,
        Msg::Resize {
            width: 30,
            height: 6,
            cell_px: Some(px),
        },
    );
    assert_eq!(s.view.cell_px, Some(px));
    assert_eq!(caretline::view(&s).cell_px, Some(px));
    // A resize without pixels keeps them.
    update(&mut s, Msg::resize(20, 5));
    assert_eq!((s.view.viewport.width, s.view.cell_px), (20, Some(px)));
    // In the state's JSON, and back.
    let json = s.to_json();
    assert!(json.contains(r#""cell_px""#), "{json}");
    assert_eq!(State::from_json(&json).unwrap().view.cell_px, Some(px));
    assert!(!State::new("", None, vp()).to_json().contains("cell_px"));
    // Rendering at another size (the protocol's `render`) keeps them too.
    let session = Session::new(s);
    assert_eq!(session.render(50, 9).cell_px, Some(px));
}

#[test]
fn resize_json_without_cell_px_is_unchanged() {
    let line = r#"{"msg":"resize","width":30,"height":6}"#;
    let m: Msg = serde_json::from_str(line).unwrap();
    assert_eq!(m, Msg::resize(30, 6));
    assert_eq!(serde_json::to_string(&m).unwrap(), line);
    let with = Msg::Resize {
        width: 30,
        height: 6,
        cell_px: Some(CellPx::new(9, 18)),
    };
    let json = serde_json::to_string(&with).unwrap();
    assert_eq!(
        json,
        r#"{"msg":"resize","width":30,"height":6,"cell_px":{"w":9,"h":18}}"#
    );
    assert_eq!(serde_json::from_str::<Msg>(&json).unwrap(), with);
}

#[test]
fn an_old_trace_with_resize_still_replays() {
    // As written before the cell size was a message.
    let trace = concat!(
        r#"{"state":{"text":"hello world","viewport":{"width":80,"height":24}}}"#,
        "\n",
        r#"{"msg":{"msg":"resize","width":12,"height":4}}"#,
        "\n",
        r#"{"msg":{"msg":"move","dir":"forward","by":"doc_end"}}"#,
        "\n",
    );
    let (s, _, n) = replay_trace_with(trace, &Host::new()).unwrap();
    assert_eq!(n, 2);
    assert_eq!((s.view.viewport.width, s.view.viewport.height), (12, 4));
    assert_eq!(s.view.cell_px, None);
    assert_eq!(s.caret(), 11);
    // A new trace with the cell size replays it.
    let mut session = Session::new(State::new("x", None, vp()));
    session.apply(Msg::Resize {
        width: 40,
        height: 8,
        cell_px: Some(CellPx::new(16, 34)),
    });
    let (s, _, _) = replay_trace_with(&session.trace_jsonl(), &Host::new()).unwrap();
    assert_eq!(s.view.cell_px, Some(CellPx::new(16, 34)));
    assert_eq!(caretline::view(&s), session.frame());
}

// --------------------------------------------------------------------------------------
// E3: frame passes, cell writers, flags and regions

/// Draws `[n]` at the top right, where `n` is the view's `count` value, and rings the caret's
/// cell.
fn badge_host() -> Host {
    host()
        .frame_pass("badge", |ctx, frame| {
            let Some(n) = ctx.view.ext.get("count") else {
                return;
            };
            let role = frame.role("badge");
            let text = format!("[{n}]");
            let mut x = frame.width - text.len() as u16;
            for g in text.chars() {
                x += frame.set(x, 0, &g.to_string(), role);
            }
            frame.regions.push(Region {
                x: frame.width - text.len() as u16,
                y: 0,
                w: text.len() as u16,
                h: 1,
                id: "badge".into(),
            });
        })
        .frame_pass("ring", |_, frame| {
            if let Some((x, y)) = frame.cursor {
                frame.flag(x, y, CellFlags::RING);
            }
        })
}

#[test]
fn a_frame_pass_shows_in_render_but_not_render_plain() {
    let mut s = with_host("hello", badge_host());
    let plain = caretline::view(&s);
    update(&mut s, ext("count", json!({"add": 7})));
    let drawn = caretline::view(&s);
    assert!(drawn.to_text().lines().next().unwrap().ends_with("[7]"));
    let bare = render_plain(&s.doc, &s.view);
    assert!(!bare.to_text().contains("[7]"));
    assert_eq!(bare.to_text(), plain.to_text());
    assert!(bare.regions.is_empty());
    // The pass's role has its name; the region is there to hit.
    let x = drawn.width - 2;
    assert_eq!(drawn.role_name(drawn.cell(x, 0).role), "badge");
    assert_eq!(drawn.region_at(x, 0).map(|r| r.id.as_str()), Some("badge"));
    assert_eq!(drawn.region_at(0, 0), None);
    // The caret's cell is ringed; skipping that pass leaves it alone.
    let (cx, cy) = drawn.cursor.unwrap();
    assert!(drawn.cell(cx, cy).flags.contains(CellFlags::RING));
    let skipped = render_skipping(&s.doc, &s.view, &["ring"]);
    assert!(skipped.cell(cx, cy).flags.is_empty());
    assert!(skipped.to_text().contains("[7]"));
    // A session's frames and the protocol's go through the passes, with flags in `cells`.
    let mut session = Session::new(s);
    assert_eq!(session.frame(), drawn);
    let r: Value = serde_json::from_str(
        &session
            .handle(r#"{"id":1,"op":"render","format":"cells"}"#, None)
            .response,
    )
    .unwrap();
    let row = &r["result"]["rows"][cy as usize];
    assert_eq!(row["flags"], json!([[cx, 1, "ring"]]));
    assert!(r["result"]["rows"][1].get("flags").is_none());
    let ansi = session.frame().to_ansi();
    assert!(ansi.contains("\x1b[4m"), "{ansi:?}");
}

#[test]
fn frame_passes_run_in_order_and_replace_by_name() {
    let host = Host::new()
        .frame_pass("a", |_, f| {
            let r = f.role("a");
            f.set(0, 0, "A", r);
        })
        .frame_pass("b", |_, f| {
            let r = f.role("b");
            f.set(0, 0, "B", r);
        });
    assert_eq!(host.frame_pass_names(), vec!["a", "b"]);
    let s = with_host("", host.clone());
    assert_eq!(caretline::view(&s).cell(0, 0).symbol.as_str(), "B");
    // Replaced in its place: still before "b".
    let host = host.frame_pass("a", |_, f| {
        let r = f.role("a");
        f.set(1, 0, "Z", r);
    });
    assert_eq!(host.frame_pass_names(), vec!["a", "b"]);
    let s = with_host("", host);
    let f = caretline::view(&s);
    assert_eq!(
        (f.cell(0, 0).symbol.as_str(), f.cell(1, 0).symbol.as_str()),
        ("B", "Z")
    );
}

#[test]
fn frame_set_restyle_and_flag_respect_wide_graphemes() {
    let mut f = Frame::new(6, 1);
    let r = f.role("x");
    assert_eq!(f.set(0, 0, "語", r), 2);
    assert_eq!(f.set(2, 0, "語", r), 2);
    assert_eq!(f.to_text(), "語語\n");
    // Over the second half of the first: its lead goes blank.
    assert_eq!(f.set(1, 0, "a", r), 1);
    assert_eq!(f.to_text(), " a語\n");
    // Over the lead of the second: its second half goes blank.
    assert_eq!(f.set(2, 0, "b", r), 1);
    assert_eq!(f.to_text(), " ab\n");
    assert_eq!(f.cell(3, 0).symbol.as_str(), " ");
    // A wide grapheme that doesn't fit is a space.
    assert_eq!(f.set(5, 0, "語", r), 1);
    assert_eq!(f.cell(5, 0).symbol.as_str(), " ");
    // Off the frame, or nothing: no cells.
    assert_eq!(f.set(6, 0, "a", r), 0);
    assert_eq!(f.set(0, 1, "a", r), 0);
    assert_eq!(f.set(0, 0, "", r), 0);
    // A control char draws as a placeholder.
    assert_eq!(f.set(0, 0, "\u{7}", r), 1);
    assert_eq!(f.cell(0, 0).symbol.as_str(), "\u{FFFD}");
    // Restyling or flagging either half covers the whole grapheme.
    f.set(3, 0, "語", Role::Text);
    let y = f.role("y");
    f.restyle(4, 0, y);
    assert_eq!((f.cell(3, 0).role, f.cell(4, 0).role), (y, y));
    f.flag(4, 0, CellFlags::DIM);
    f.flag(3, 0, CellFlags::DIM | CellFlags::RING);
    assert!(f
        .cell(3, 0)
        .flags
        .contains(CellFlags::DIM | CellFlags::RING));
    assert_eq!(f.cell(4, 0).flags.names(), "dim ring");
    assert!(f.cell(2, 0).flags.is_empty());
}

#[test]
fn frame_set_never_splits_a_wide_grapheme() {
    use rand::{Rng, SeedableRng};
    const GS: &[&str] = &["a", "語", "😀", "é", " ", "x", "ｗ", "\t"];
    for seed in 0..300u64 {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        let w = rng.random_range(1..12u16);
        let mut f = Frame::new(w, 2);
        let r = f.role("r");
        for _ in 0..rng.random_range(1..40) {
            let g = GS[rng.random_range(0..GS.len())];
            let x = rng.random_range(0..w + 1);
            let y = rng.random_range(0..2);
            let n = f.set(x, y, g, r);
            if x < w {
                assert!(n >= 1 && x + n <= w, "seed {seed}: {g:?} at {x} wrote {n}");
            }
            // Every row: a lead of width k is followed by exactly k - 1 empty cells.
            for y in 0..2 {
                let mut x = 0;
                while x < w {
                    let c = f.cell(x, y);
                    assert!(
                        !c.symbol.is_empty(),
                        "seed {seed}: a lone second half at {x}"
                    );
                    let k = caretline::view::display_width(&c.symbol) as u16;
                    for dx in 1..k {
                        assert!(x + dx < w, "seed {seed}: a grapheme past the edge");
                        assert!(f.cell(x + dx, y).symbol.is_empty(), "seed {seed}");
                    }
                    x += k.max(1);
                }
            }
        }
    }
}

// --------------------------------------------------------------------------------------
// E4: view::locate, the inverse of view::hit

/// Every position a caret can stop at: the grapheme boundaries.
fn stops(s: &State) -> Vec<usize> {
    let text = s.doc.text.slice(..);
    let mut out = vec![0];
    let mut p = 0;
    while p < text.len_chars() {
        p = caretline::helix::graphemes::next_grapheme_boundary(text, p);
        out.push(p);
    }
    out
}

/// `hit(locate(p)) == p` for every position on screen, and the caret is where the frame draws
/// it.
fn check_inverse(s: &State, what: &str) -> usize {
    let layout = caretline::layout::Layout::of(&s.doc, &s.view);
    let mut on_screen = 0;
    for p in stops(s) {
        let line = s.doc.text.char_to_line(p);
        if p < layout.content_start(line) {
            continue; // inside a block's marker: placed at its content, as a caret is
        }
        match locate(&s.doc, &s.view, p) {
            Locate::At { x, y } => {
                on_screen += 1;
                assert_eq!(
                    hit(&s.doc, &s.view, x, y),
                    Hit::Text { pos: p },
                    "{what}: {p} located at ({x}, {y})"
                );
            }
            Locate::Folded { block } => assert!(s.view.folds.contains(&block), "{what}"),
            _ => {}
        }
    }
    let f = caretline::view(s);
    if let Some((x, y)) = f.cursor {
        assert_eq!(
            locate(&s.doc, &s.view, s.caret()),
            Locate::At { x, y },
            "{what}: the caret"
        );
    }
    on_screen
}

#[test]
fn locate_finds_cells_and_directions() {
    // 10 columns, no wrap, 3 text rows.
    let mut s = State::new("abcdefghijklmno\nline two\nthree\nfour\nfive\n", None, vp());
    s.doc.config.soft_wrap = false;
    update(&mut s, Msg::resize(10, 4));
    let at = |s: &State, p| locate(&s.doc, &s.view, p);
    assert_eq!(at(&s, 0), Locate::At { x: 0, y: 0 });
    assert_eq!(at(&s, 3), Locate::At { x: 3, y: 0 });
    assert_eq!(at(&s, 12), Locate::Right);
    assert_eq!(at(&s, 17), Locate::At { x: 1, y: 1 });
    assert_eq!(at(&s, 38), Locate::Below);
    // Scrolled sideways and down.
    s.view.scroll.col = 5;
    assert_eq!(at(&s, 2), Locate::Left);
    assert_eq!(at(&s, 12), Locate::At { x: 7, y: 0 });
    update(&mut s, Msg::ScrollView { rows: 2 });
    assert_eq!(at(&s, 0), Locate::Above);
    // Serialized for hosts and agents.
    assert_eq!(
        serde_json::to_value(Locate::At { x: 1, y: 2 }).unwrap(),
        json!({"kind": "at", "x": 1, "y": 2})
    );
    assert_eq!(
        serde_json::to_value(Locate::Folded {
            block: caretline::MarkId(4)
        })
        .unwrap(),
        json!({"kind": "folded", "block": 4})
    );
}

#[test]
fn locate_is_the_inverse_of_hit_in_plain_text() {
    use rand::{Rng, SeedableRng};
    let mut shown = 0;
    for seed in 0..120u64 {
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        let text = common::gen::text(&mut rng);
        let w = rng.random_range(4..40);
        let h = rng.random_range(2..10);
        let mut s = State::new(&text, None, vp());
        s.doc.config.soft_wrap = rng.random_bool(0.6);
        s.view.config.status_bar = rng.random_bool(0.5);
        update(&mut s, Msg::resize(w, h));
        for _ in 0..rng.random_range(0..6) {
            let m = common::gen::msg(&mut rng, &s);
            update(&mut s, m);
            if rng.random_bool(0.3) {
                update(
                    &mut s,
                    Msg::ScrollView {
                        rows: rng.random_range(-3..4),
                    },
                );
            }
        }
        shown += check_inverse(&s, &format!("seed {seed}"));
    }
    assert!(shown > 1000, "only {shown} positions were on screen");
}

#[test]
fn locate_is_the_inverse_of_hit_in_an_outline_with_folds() {
    use caretline::outline::markdown;
    use caretline::{OutlineConfig, OutlineLayout};
    const MD: &str = "# Trip\n\nBooked the flat, which faces the river and the old tram line.\n\n- Pay the deposit before the end of the month\n  - ask about the desk\n  - and the lamp\n- Book flights\n\n```\na fenced line that is much longer than any column here\n```\n\n1. Pack\n2. Leave\n";
    for w in [12u16, 20, 33, 60] {
        for h in [3u16, 6, 12] {
            for layout in [false, true] {
                let mut s = markdown::load(MD, None, vp(), OutlineConfig::default());
                if layout {
                    s.view.layout = Some(OutlineLayout::default().with_hang_glyphs(true));
                }
                update(&mut s, Msg::resize(w, h));
                let what = format!("{w}x{h} layout {layout}");
                check_inverse(&s, &what);
                // Fold the first list item: its children are Folded under it.
                let item = s.doc.text.to_string().find("- Pay").unwrap();
                let block = s.doc.marks.at(item).unwrap();
                update(&mut s, Msg::Fold { id: block });
                let child = s.doc.text.to_string().find("ask").unwrap();
                assert_eq!(
                    locate(&s.doc, &s.view, child),
                    Locate::Folded { block },
                    "{what}"
                );
                check_inverse(&s, &what);
                for _ in 0..4 {
                    update(&mut s, Msg::ScrollView { rows: 2 });
                    check_inverse(&s, &what);
                }
            }
        }
    }
}
