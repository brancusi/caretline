//! The generic hooks a host builds on: view values (`View::ext`) and their reducers
//! (`Msg::Ext`, `Host::ext`). The test host is made up (a counter and a pin that follows the
//! text): nothing here means anything to the engine.

use std::sync::{Arc, Mutex};

use caretline::trace::{replay_trace_with, TraceLine};
use caretline::{
    update, update_doc_with_changes, update_with_changes, ChangeSet, Effect, ExtChange, ExtFns,
    ExtOut, Host, Msg, Session, State, View, Viewport,
};
use serde_json::{json, Value};

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
