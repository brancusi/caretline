//! Input rules that adjust what the engine does with a key rather than replace it
//! ([`Edit::then_default`]). The rule here is a host's normalization: Enter in the middle of a
//! line drops the spaces after the caret, so the new line starts at a word. Other editors keep
//! the space; the engine's own Enter does too, and these tests check it still does.

use caretline::helix::Selection;
use caretline::outline::markdown;
use caretline::{update, Edit, Host, Kind, MarkOp, Msg, OutlineConfig, State, Viewport};

fn vp() -> Viewport {
    Viewport {
        width: 60,
        height: 10,
    }
}

fn load(md: &str, cfg: OutlineConfig) -> State {
    markdown::load(md, None, vp(), cfg)
}

/// Enter with spaces after the caret: drop them, then let the engine split as it would.
fn trim_rule(ctx: &caretline::Ctx, msg: &Msg) -> Option<Edit> {
    if !matches!(msg, Msg::InsertNewline) || ctx.selection().len() != 1 {
        return None;
    }
    let r = ctx.selection().primary();
    if !r.is_empty() {
        return None;
    }
    let p = r.head;
    let spaces = ctx.text().chars_at(p).take_while(|c| *c == ' ').count();
    (spaces > 0).then(|| Edit {
        changes: vec![(p, p + spaces, String::new())],
        ..Edit::then_default()
    })
}

fn host() -> Host {
    Host::new().input_rule("test.trim_split", trim_rule)
}

/// Each block's id, kind, depth, tag and marker text.
fn shape(s: &State) -> Vec<(u64, Kind, u16, Option<char>, String)> {
    let o = s.doc.blocks().expect("an outline document");
    let text = s.doc.text.slice(..);
    o.blocks
        .iter()
        .map(|b| {
            let marker = text.slice(b.start..b.content_start()).to_string();
            (b.id.0, b.kind, b.depth, b.tag, marker)
        })
        .collect()
}

fn caret_only(s: &State) -> usize {
    assert_eq!(s.view.selection.len(), 1, "{:?}", s.view.selection);
    let r = s.view.selection.primary();
    assert!(r.is_empty(), "{r:?}");
    r.head
}

/// The engine's own Enter at `at`, and the rule's, on the same document.
fn both(md: &str, cfg: OutlineConfig, at: usize) -> (State, State) {
    let mut plain = load(md, cfg.clone());
    let mut ruled = load(md, cfg);
    ruled.doc.set_host(host());
    for s in [&mut plain, &mut ruled] {
        s.view.selection = Selection::point(at);
        update(s, Msg::InsertNewline);
    }
    (plain, ruled)
}

#[test]
fn enter_in_a_bullet_drops_the_space_and_undoes_in_one_step() {
    let md = "- Last line of the plan\n";
    let at = 2 + 9; // after "line"
    let mut s = load(md, OutlineConfig::default());
    s.doc.set_host(host());
    let before = shape(&s);
    s.view.selection = Selection::point(at);

    update(&mut s, Msg::InsertNewline);
    assert_eq!(s.doc.text.to_string(), "- Last line\n- of the plan");
    let o = s.doc.blocks().unwrap();
    assert_eq!(o.blocks.len(), 2);
    let (old, new) = (&o.blocks[0], &o.blocks[1]);
    // The old block keeps its mark; the new one has its own, minted by the engine.
    assert_eq!(old.id.0, before[0].0);
    assert_ne!(new.id, old.id);
    assert!(s.doc.marks.contains(new.id));
    assert_eq!((new.kind, new.depth), (Kind::Bullet, 0));
    // The caret at the start of "of the plan".
    assert_eq!(caret_only(&s), new.content_start());
    assert_eq!(
        s.doc.text.slice(new.content_start()..).to_string(),
        "of the plan"
    );
    let split = shape(&s);

    // One undo: the original text, the caret where it was, one empty range.
    update(&mut s, Msg::Undo);
    assert_eq!(s.doc.text.to_string(), "- Last line of the plan");
    assert_eq!(caret_only(&s), at);
    assert_eq!(shape(&s), before);

    // Redo: the split again, the same ids.
    update(&mut s, Msg::Redo);
    assert_eq!(s.doc.text.to_string(), "- Last line\n- of the plan");
    assert_eq!(shape(&s), split);
}

#[test]
fn the_new_block_is_shaped_as_the_engines_own_enter_shapes_it() {
    let tasks = OutlineConfig::default()
        .with_tags(" x".into())
        .with_new_tag(Some(' '));
    let cases: [(&str, OutlineConfig, usize); 4] = [
        ("- Last line of the plan\n", OutlineConfig::default(), 11),
        ("* Last line of the plan\n", OutlineConfig::default(), 11),
        ("1. Last line of the plan\n", OutlineConfig::default(), 12),
        ("- top\n  - [x] Last line of the plan\n", tasks, 6 + 8 + 9),
    ];
    for (md, cfg, at) in cases {
        let (plain, ruled) = both(md, cfg, at);
        // The same blocks: ids, kinds, depths, tags and markers.
        assert_eq!(shape(&ruled), shape(&plain), "{md:?}");
        // The same text but the one space after the new marker.
        let o = plain.doc.blocks().unwrap();
        let cs = o
            .block_at(plain.doc.text.slice(..), plain.view.caret())
            .content_start();
        assert_eq!(caret_only(&plain), cs, "{md:?}");
        let mut expect = plain.doc.text.to_string();
        let byte = plain.doc.text.char_to_byte(cs);
        assert_eq!(&expect[byte..byte + 1], " ", "{md:?}");
        expect.remove(byte);
        assert_eq!(ruled.doc.text.to_string(), expect, "{md:?}");
        assert_eq!(caret_only(&ruled), cs, "{md:?}");
    }
}

#[test]
fn enter_in_a_paragraph_drops_the_space_too() {
    let md = "Intro\n\nLast line of the plan\n";
    let at = 6 + 9;
    let (plain, mut ruled) = both(md, OutlineConfig::default(), at);
    // The engine's Enter mid-line breaks the paragraph's line; the rule changes only the space.
    assert_eq!(plain.doc.text.to_string(), "Intro\nLast line\n of the plan");
    assert_eq!(ruled.doc.text.to_string(), "Intro\nLast line\nof the plan");
    assert_eq!(shape(&ruled), shape(&plain));
    assert_eq!(caret_only(&ruled), 16);

    update(&mut ruled, Msg::Undo);
    assert_eq!(ruled.doc.text.to_string(), "Intro\nLast line of the plan");
    assert_eq!(caret_only(&ruled), at);
    update(&mut ruled, Msg::Redo);
    assert_eq!(ruled.doc.text.to_string(), "Intro\nLast line\nof the plan");
}

#[test]
fn typing_after_the_split_is_its_own_undo_step() {
    let mut s = load("- Last line of the plan\n", OutlineConfig::default());
    s.doc.set_host(host());
    s.view.selection = Selection::point(11);
    update(&mut s, Msg::InsertNewline);
    for c in ["n", "o", "t", " "] {
        update(&mut s, Msg::InsertText { text: c.into() });
    }
    assert_eq!(s.doc.text.to_string(), "- Last line\n- not of the plan");
    update(&mut s, Msg::Undo);
    assert_eq!(s.doc.text.to_string(), "- Last line\n- of the plan");
    assert_eq!(caret_only(&s), 14);
    update(&mut s, Msg::Undo);
    assert_eq!(s.doc.text.to_string(), "- Last line of the plan");
    assert_eq!(caret_only(&s), 11);
    // Nothing older to undo.
    update(&mut s, Msg::Undo);
    assert_eq!(s.doc.text.to_string(), "- Last line of the plan");
}

#[test]
fn a_plain_document_groups_like_the_engines_own_enter() {
    // In a plain document Enter is part of the typing run: the rule's edit joins it as well.
    let run = |with_rule: bool| {
        let mut s = State::new("ab  cd", None, vp());
        if with_rule {
            s.doc.set_host(host());
        }
        s.view.selection = Selection::point(2);
        update(&mut s, Msg::InsertNewline);
        let split = s.doc.text.to_string();
        update(&mut s, Msg::InsertText { text: "x".into() });
        update(&mut s, Msg::Undo);
        (split, s.doc.text.to_string(), caret_only(&s))
    };
    assert_eq!(run(false), ("ab\n  cd".into(), "ab  cd".into(), 2));
    assert_eq!(run(true), ("ab\ncd".into(), "ab  cd".into(), 2));
}

#[test]
fn a_rule_that_leaves_the_text_alone_still_lets_the_default_run() {
    // `then_default` with no changes: the engine's Enter, the rule's effect beside it.
    let host = Host::new().input_rule("test.note", |_, msg| {
        matches!(msg, Msg::InsertNewline).then(|| Edit {
            effects: vec![("test.entered".into(), serde_json::json!(null))],
            ..Edit::then_default()
        })
    });
    let mut s = load("- one\n", OutlineConfig::default());
    s.doc.set_host(host);
    s.view.selection = Selection::point(5);
    let fx = update(&mut s, Msg::InsertNewline);
    assert_eq!(s.doc.text.to_string(), "- one\n- ");
    assert!(fx
        .iter()
        .any(|e| matches!(e, caretline::Effect::Host { name, .. } if name == "test.entered")));
    update(&mut s, Msg::Undo);
    assert_eq!(s.doc.text.to_string(), "- one");
    assert_eq!(caret_only(&s), 5);
}

#[test]
fn a_rule_without_the_default_must_rebuild_the_split_itself() {
    // Without `then_default` a rule writes the line break and the marker itself: it works for
    // a plain bullet, but the next number, a tag and a paragraph's rules are the host's to
    // reimplement. Undo is one step either way.
    let host = Host::new().input_rule("test.by_hand", |ctx, msg| {
        let mut edit = trim_rule(ctx, msg)?;
        let o = ctx.blocks()?;
        let b = o.block_at(ctx.text(), ctx.caret());
        let marker = ctx.text().slice(b.start..b.content_start()).to_string();
        let (p, end, _) = edit.changes[0].clone();
        edit.changes = vec![(p, end, format!("{}{marker}", ctx.line_ending()))];
        edit.selection = Some(Selection::point(p + 1 + marker.chars().count()));
        edit.then_default = false;
        Some(edit)
    });
    let mut s = load("- Last line of the plan\n", OutlineConfig::default());
    s.doc.set_host(host);
    s.view.selection = Selection::point(11);
    update(&mut s, Msg::InsertNewline);
    assert_eq!(s.doc.text.to_string(), "- Last line\n- of the plan");
    assert_eq!(caret_only(&s), 14);
    assert_eq!(s.doc.blocks().unwrap().blocks.len(), 2);
    update(&mut s, Msg::Undo);
    assert_eq!(s.doc.text.to_string(), "- Last line of the plan");
    assert_eq!(caret_only(&s), 11);
}

// ---------------------------------------------------------------------------------------
// Blank rows a rule sets: explicit host intent wins over the engine's gap pinning

/// Block `k`'s depth and blank row.
fn depth_gap(s: &State, k: usize) -> (u16, bool) {
    let o = s.doc.blocks().unwrap();
    (o.blocks[k].depth, o.blocks[k].gap)
}

/// Tab: close the blank row above the caret's block, then nest it as the engine would.
fn join_rule(ctx: &caretline::Ctx, msg: &Msg) -> Option<Edit> {
    if !matches!(msg, Msg::Indent) {
        return None;
    }
    let o = ctx.blocks()?;
    let id = o.block_at(ctx.text(), ctx.caret()).id;
    Some(Edit {
        marks: vec![MarkOp::SetGap {
            id,
            gap: Some(false),
        }],
        ..Edit::then_default()
    })
}

#[test]
fn a_gap_a_rule_sets_survives_the_default_and_undoes_with_it() {
    let mut s = load("a\n\n- b\n", OutlineConfig::default());
    s.doc
        .set_host(Host::new().input_rule("test.join", join_rule));
    s.view.selection = Selection::point(s.doc.text.len_chars());
    assert_eq!(depth_gap(&s, 1), (0, true));
    let rev = s.doc.history.current_revision();
    update(&mut s, Msg::Indent);
    assert_eq!(
        depth_gap(&s, 1),
        (1, false),
        "the rule's gap is not pinned back"
    );
    assert_eq!(s.doc.history.current_revision(), rev + 1, "one undo step");
    update(&mut s, Msg::Undo);
    assert_eq!(depth_gap(&s, 1), (0, true));
    update(&mut s, Msg::Redo);
    assert_eq!(depth_gap(&s, 1), (1, false));
    // Shift-Tab pins it where it is.
    update(&mut s, Msg::Outdent);
    assert_eq!(depth_gap(&s, 1), (0, false));
}

#[test]
fn a_gap_a_rule_sets_without_the_default_survives_too() {
    // The rule replaces Tab: it indents and closes the row itself.
    let host = Host::new().input_rule("test.join_by_hand", |ctx, msg| {
        let mut edit = join_rule(ctx, msg)?;
        let o = ctx.blocks()?;
        let b = o.block_at(ctx.text(), ctx.caret());
        edit.changes = vec![(b.start, b.start, "  ".into())];
        edit.then_default = false;
        Some(edit)
    });
    let mut s = load("a\n\n- b\n", OutlineConfig::default());
    s.doc.set_host(host);
    s.view.selection = Selection::point(s.doc.text.len_chars());
    update(&mut s, Msg::Indent);
    assert_eq!(depth_gap(&s, 1), (1, false));
}

#[test]
fn a_gap_a_keep_gaps_command_sets_is_not_pinned_back() {
    let host = Host::new().command("test.join", |ctx, _| {
        let o = ctx.blocks().ok_or("no outline")?;
        let b = o.block_at(ctx.text(), ctx.caret());
        Ok(Edit {
            changes: vec![(b.start, b.start, "  ".into())],
            marks: vec![MarkOp::SetGap {
                id: b.id,
                gap: Some(false),
            }],
            keep_gaps: true,
            ..Edit::default()
        })
    });
    let mut s = load("a\n\n- b\n\n- c\n", OutlineConfig::default());
    s.doc.set_host(host);
    s.view.selection = Selection::point(5);
    update(
        &mut s,
        Msg::Command {
            name: "test.join".into(),
            args: serde_json::Value::Null,
        },
    );
    assert_eq!(depth_gap(&s, 1), (1, false), "the command's own gap");
    assert_eq!(depth_gap(&s, 2), (0, true), "every other row kept");
}
