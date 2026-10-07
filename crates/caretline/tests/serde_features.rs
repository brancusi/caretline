//! The engine's JSON under any serde_json features a host turns on.
//!
//! Cargo unifies features across a build, so a host that enables serde_json's
//! `preserve_order` or `arbitrary_precision` enables it for the engine too. The engine must
//! not enable either itself (it would change `serde_json::Map` or number handling for the
//! whole host), and its wire format must not depend on them: responses are structs, whose
//! field order is fixed by declaration, never maps. CI runs this suite with each feature on
//! (docs/testing.md).

use caretline::marks::{ClipMark, Clipboard};
use caretline::{ExtChange, MarkAttrs, MarkId, MarkOp, Msg, Session, State, Viewport};
use serde_json::{json, Value};

fn session() -> Session {
    Session::new(State::new("one\ntwo\n", Some("doc.md".into()), Viewport { width: 30, height: 5 }))
}

/// The response line, as bytes.
fn line(s: &mut Session, req: Value) -> String {
    s.handle(&req.to_string(), None).response
}

fn round_trip<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
    let text = serde_json::to_string(v).unwrap();
    let back: T = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
    assert_eq!(&back, v, "{text}");
}

/// A payload with the numbers `arbitrary_precision` changes the handling of.
fn payload() -> Value {
    json!({"n": 3, "neg": -7, "big": 18446744073709551615u64, "f": 2.5, "list": [1, 2.25], "s": "x"})
}

#[test]
fn the_engine_manifest_enables_no_map_or_number_feature_of_serde_json() {
    // The engine's own dependency, and the workspace one it may inherit.
    let root = env!("CARGO_MANIFEST_DIR");
    for path in [format!("{root}/Cargo.toml"), format!("{root}/../../Cargo.toml")] {
        let Ok(manifest) = std::fs::read_to_string(&path) else { continue };
        for l in manifest.lines().filter(|l| l.trim_start().starts_with("serde_json")) {
            for feature in ["preserve_order", "arbitrary_precision"] {
                assert!(!l.contains(feature), "{path}: `{l}` enables serde_json's {feature} for every host");
            }
        }
    }
}

#[test]
fn responses_keep_their_key_order() {
    let mut s = session();
    assert_eq!(line(&mut s, json!({"id": 1, "op": "view.open"})), r#"{"id":1,"result":{"rev":1,"view":1}}"#);
    assert_eq!(
        line(&mut s, json!({"id": 2, "op": "view.list"})),
        r#"{"id":2,"result":{"rev":1,"views":[{"view":0,"w":30,"h":5,"caret":0,"read_only":false},{"view":1,"w":30,"h":5,"caret":0,"read_only":false}]}}"#
    );
    assert_eq!(line(&mut s, json!({"id": 3, "op": "view.close", "view": 1})), r#"{"id":3,"result":{"rev":2,"closed":1}}"#);
    assert_eq!(line(&mut s, json!({"id": 4, "op": "subscribe"})), r#"{"id":4,"result":{"rev":2,"subscribed":true}}"#);
    assert_eq!(line(&mut s, json!({"id": 5, "op": "unsubscribe"})), r#"{"id":5,"result":{"rev":2,"subscribed":false}}"#);
    let set = line(&mut s, json!({"id": 6, "op": "text.set", "text": "one\n"}));
    assert!(set.starts_with(r#"{"id":6,"result":{"rev":3,"changed":true,"view":0,"msgs":[{"msg":"#), "{set}");
    let hello = line(&mut s, json!({"id": 7, "op": "hello"}));
    assert!(hello.starts_with(r#"{"id":7,"result":{"proto":1,"version":""#), "{hello}");
    assert!(hello.contains(r#","rev":3,"ops":["#) && hello.ends_with(r#""commands":[]}}"#), "{hello}");
    let keymap = line(&mut s, json!({"id": 8, "op": "keymap.get"}));
    assert!(keymap.starts_with(r#"{"id":8,"result":{"outline":false,"bindings":["#), "{keymap}");
    let commands = line(&mut s, json!({"id": 9, "op": "commands.list"}));
    assert!(commands.starts_with(r#"{"id":9,"result":{"commands":["#) && commands.ends_with(r#""host_commands":[]}}"#));
    assert_eq!(line(&mut s, json!({"id": 10, "op": "trace.checkpoint"})), r#"{"id":10,"result":{"rev":3}}"#);
}

#[test]
fn clipboards_with_marks_and_payloads_round_trip() {
    round_trip(&Clipboard::from("plain".to_string()));
    let mark = |offset, id| ClipMark { offset, id: MarkId(id), attrs: MarkAttrs { gap: Some(true), data: Some(payload()) } };
    round_trip(&Clipboard { text: "a\nb\n".into(), external: Some("- a\n- b\n".into()), marks: vec![mark(0, 4), mark(2, 9)], blocks: true });
}

#[test]
fn tagged_messages_and_changes_round_trip() {
    for msg in [
        Msg::Tick { now_ms: 1_700_000_000_123 },
        Msg::Scroll { rows: -3 },
        Msg::Resize { width: 80, height: 24 },
        Msg::Click { col: 7, row: 2, extend: true },
        Msg::Paste { text: Some("x".into()) },
    ] {
        round_trip(&msg);
    }
    for op in [
        MarkOp::Mint { pos: 12, attrs: MarkAttrs { gap: None, data: Some(payload()) } },
        MarkOp::Remove { id: MarkId(5) },
        MarkOp::SetGap { id: MarkId(6), gap: Some(false) },
        MarkOp::SetData { id: MarkId(7), data: Some(payload()) },
    ] {
        round_trip(&op);
    }
    round_trip(&ExtChange::SetData { id: MarkId(3), data: Some(payload()) });
    round_trip(&ExtChange::SetGap { id: MarkId(3), gap: Some(true) });
}

#[test]
fn a_state_with_marks_payloads_and_a_clipboard_round_trips_through_the_protocol() {
    let mut s = session();
    let mut state = s.state().clone();
    let id = state.doc.marks.mint(0);
    state.doc.marks.set_data(id, Some(payload()));
    state.doc.clipboard = Clipboard {
        text: "one\n".into(),
        external: None,
        marks: vec![ClipMark { offset: 0, id: MarkId(40), attrs: MarkAttrs { gap: None, data: Some(payload()) } }],
        blocks: false,
    };
    let r: Value = serde_json::from_str(&line(&mut s, json!({"id": 1, "op": "state.set", "state": state}))).unwrap();
    assert!(r.get("error").is_none(), "{r}");
    let got: Value = serde_json::from_str(&line(&mut s, json!({"id": 2, "op": "state.get"}))).unwrap();
    let back: State = serde_json::from_value(got["result"]["state"].clone()).unwrap();
    assert_eq!(serde_json::to_value(&back).unwrap(), serde_json::to_value(&state).unwrap());
}

#[test]
fn requests_with_numbers_and_traces_replay() {
    let mut s = session();
    let msgs = json!([
        {"msg": "tick", "now_ms": 1000},
        {"msg": "resize", "width": 40, "height": 6},
        {"msg": "scroll", "rows": 1},
        {"msg": "paste", "text": "a\nb\nc\n"},
        {"msg": "click", "col": 1, "row": 0, "extend": false}
    ]);
    let r: Value = serde_json::from_str(&line(&mut s, json!({"id": 1, "op": "msgs", "msgs": msgs, "if_rev": 0}))).unwrap();
    assert!(r.get("error").is_none(), "{r}");
    let (replayed, _) = caretline::trace::replay_trace(&s.trace_jsonl()).unwrap();
    assert_eq!(replayed.doc.text.to_string(), s.state().doc.text.to_string());
    assert_eq!(replayed.view.selection, s.state().view.selection);
    assert_eq!(replayed.view.viewport, Viewport { width: 40, height: 6 });
}
