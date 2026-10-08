//! `Document::take_touched`: the blocks a host re-reads after a message.
//!
//! - A caret move that changes nothing touches nothing.
//! - Dropping an empty last block touches only the end, however the drop is sent: a key, an
//!   edit of the host's (as small as it is or as the whole text), a change from elsewhere, a
//!   new whole text.
//! - The same on a 5,000-block page: one block, not the page.
//! - A property: for random documents, edits, caret moves, host edits and changes from
//!   elsewhere, every block whose text, kind, depth, tag, blank row or mark changed is inside
//!   the touched range (it never under-reports).

mod common;

use std::collections::HashMap;
use std::time::Instant;

use caretline::helix::graphemes::ensure_grapheme_boundary_prev;
use caretline::helix::Selection;
use caretline::outline::markdown;
use caretline::{
    update, By, Dir, Edit, ExtChange, Host, Kind, MarkAttrs, MarkId, Msg, NewBlock, OutlineConfig,
    Session, State, Viewport,
};
use common::gen;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

fn page(md: &str) -> State {
    let mut s = markdown::load(
        md,
        Some("t.md".into()),
        Viewport {
            width: 80,
            height: 24,
        },
        OutlineConfig::default(),
    );
    s.doc.set_host(drop_host());
    s
}

/// A host that drops the last block when it is empty: `small` deletes just its line,
/// otherwise the edit rewrites the whole text without it (as a host that serializes its own
/// model would).
fn drop_host() -> Host {
    Host::new().command("test.drop_last", |ctx, args| {
        let text = ctx.text().to_string();
        let o = ctx.blocks().ok_or("an outline")?;
        let b = o.blocks.last().ok_or("no blocks")?;
        if o.blocks.len() < 2 || ctx.text().slice(b.content_start()..b.end).len_chars() > 0 {
            return Ok(Edit::default());
        }
        let from = b.start - 1;
        let len = ctx.text().len_chars();
        let changes = if args["small"].as_bool() == Some(true) {
            vec![(from, len, String::new())]
        } else {
            let kept: String = text.chars().take(from).collect();
            vec![(0, len, kept)]
        };
        Ok(Edit {
            changes,
            ..Edit::default()
        })
    })
}

fn mv(s: &mut State, dir: Dir, by: By) {
    update(
        s,
        Msg::Move {
            dir,
            by,
            extend: false,
        },
    );
}

/// A page of `n` bullets with a fresh empty block last and the caret on it, nothing touched.
fn with_empty_last(n: usize) -> State {
    let md: String = (0..n).map(|i| format!("- item {i}\n")).collect();
    let mut s = page(&md);
    mv(&mut s, Dir::Forward, By::DocEnd);
    update(&mut s, Msg::InsertNewline);
    let o = s.blocks().unwrap();
    assert_eq!(o.blocks.len(), n + 1);
    assert_eq!(s.caret(), s.doc.text.len_chars());
    s.doc.take_touched();
    s
}

/// The blocks a touched range meets (a block ending where it starts counts).
fn touched_blocks(s: &State, r: Option<(usize, usize)>) -> Vec<usize> {
    let Some((from, to)) = r else {
        return Vec::new();
    };
    let o = s.blocks().unwrap();
    (0..o.blocks.len())
        .filter(|&i| o.blocks[i].start <= to && from <= o.blocks[i].end)
        .collect()
}

/// The ways a host drops the empty last block after the caret left it.
fn drops(s: &State) -> Vec<(&'static str, Msg)> {
    let o = s.blocks().unwrap();
    let last = o.blocks.last().unwrap();
    let len = s.doc.text.len_chars();
    let kept: String = s.doc.text.chars().take(last.start - 1).collect();
    vec![
        (
            "a small edit",
            Msg::Command {
                name: "test.drop_last".into(),
                args: serde_json::json!({ "small": true }),
            },
        ),
        (
            "an edit of the whole text",
            Msg::Command {
                name: "test.drop_last".into(),
                args: serde_json::json!({ "small": false }),
            },
        ),
        (
            "Msg::Edit of the whole text",
            Msg::Edit {
                changes: vec![(0, len, kept.clone())],
                join: false,
            },
        ),
        (
            "a block removed from elsewhere",
            Msg::External {
                changes: vec![ExtChange::RemoveBlock { id: last.id }],
            },
        ),
        (
            "the whole text from elsewhere",
            Msg::External {
                changes: vec![ExtChange::Replace {
                    from: 0,
                    to: len,
                    text: kept,
                }],
            },
        ),
    ]
}

#[test]
fn leaving_an_empty_last_block_touches_nothing() {
    let mut s = with_empty_last(3);
    mv(&mut s, Dir::Backward, By::DocStart);
    assert_eq!(s.caret(), 2);
    assert_eq!(s.doc.take_touched(), None);
}

#[test]
fn dropping_an_empty_last_block_touches_only_the_end() {
    let base = with_empty_last(3);
    for (how, msg) in drops(&base) {
        let mut s = base.clone();
        mv(&mut s, Dir::Backward, By::DocStart);
        let ids: Vec<MarkId> = s.blocks().unwrap().blocks.iter().map(|b| b.id).collect();
        update(&mut s, msg);
        let o = s.blocks().unwrap();
        assert_eq!(o.blocks.len(), 3, "{how}: {:?}", s.doc.text.to_string());
        let now: Vec<MarkId> = o.blocks.iter().map(|b| b.id).collect();
        assert_eq!(now, ids[..3], "{how}: the other blocks keep their ids");
        let t = s.doc.take_touched();
        assert_eq!(touched_blocks(&s, t), vec![2], "{how}: touched {t:?}");
    }
    // A whole new text through the session: only its end.
    let mut session = Session::new(base.clone());
    session.apply(Msg::Move {
        dir: Dir::Backward,
        by: By::DocStart,
        extend: false,
    });
    let text = session.state().doc.text.to_string();
    session.set_text(text.trim_end_matches("\n- "));
    let mut s = session.state().clone();
    s.doc.take_touched();
    assert_eq!(s.blocks().unwrap().blocks.len(), 3);
}

/// The host's case: a 5,000-block page, the caret on a fresh empty last block, then to the
/// start of the document, then the block dropped. Each reads one block, not the page.
#[test]
fn a_large_page_touches_one_block() {
    let n = 5_000;
    let base = with_empty_last(n);
    let mut s = base.clone();
    let t = Instant::now();
    mv(&mut s, Dir::Backward, By::DocStart);
    let touched = s.doc.take_touched();
    eprintln!("move.doc_start: {touched:?} in {:?}", t.elapsed());
    assert_eq!(touched, None);
    for (how, msg) in drops(&base) {
        let mut s = base.clone();
        mv(&mut s, Dir::Backward, By::DocStart);
        let t = Instant::now();
        update(&mut s, msg);
        let touched = s.doc.take_touched();
        let blocks = touched_blocks(&s, touched);
        eprintln!(
            "{how}: {} block(s) touched in {:?}",
            blocks.len(),
            t.elapsed()
        );
        assert_eq!(blocks, vec![n - 1], "{how}: touched {touched:?}");
    }
}

/// Changing only a block's mark (its blank row, its payload) touches that block.
#[test]
fn a_mark_change_touches_its_block() {
    let mut s = page("- a\n- b\n- c\n");
    s.doc.take_touched();
    let b = s.blocks().unwrap().blocks[1].clone();
    update(
        &mut s,
        Msg::External {
            changes: vec![ExtChange::SetGap {
                id: b.id,
                gap: Some(true),
            }],
        },
    );
    assert_eq!(touched_blocks(&s, s.clone().doc.take_touched()), vec![1]);
    s.doc.take_touched();
    update(
        &mut s,
        Msg::External {
            changes: vec![ExtChange::SetData {
                id: b.id,
                data: Some(serde_json::json!({ "row": 7 })),
            }],
        },
    );
    assert_eq!(touched_blocks(&s, s.clone().doc.take_touched()), vec![1]);
    s.doc.take_touched();
    // The same payload again changes nothing.
    update(
        &mut s,
        Msg::External {
            changes: vec![ExtChange::SetData {
                id: b.id,
                data: Some(serde_json::json!({ "row": 7 })),
            }],
        },
    );
    assert_eq!(s.doc.take_touched(), None);
}

/// A change that reshapes blocks past its own chars takes them in: a code fence opened
/// before them.
#[test]
fn a_fence_takes_in_the_blocks_it_reshapes() {
    let mut s = page("intro\n\n- a\n- b\n\nlast");
    s.doc.take_touched();
    s.view.selection = Selection::point(5);
    update(
        &mut s,
        Msg::InsertText {
            text: "\n```".into(),
        },
    );
    let t = s.doc.take_touched();
    let o = s.blocks().unwrap();
    let all: Vec<usize> = (0..o.blocks.len()).collect();
    assert_eq!(touched_blocks(&s, t), all, "{:?}", s.doc.text.to_string());
}

// ---------------------------------------------------------------------------------------
// The property

const WORDS: &[&str] = &["one", "two", "日本", "é", "x.", "a"];

fn random_doc(rng: &mut StdRng) -> State {
    let mut md = String::new();
    for i in 0..rng.random_range(1..10) {
        if i > 0 && rng.random_bool(0.3) {
            md.push('\n');
        }
        let text = (0..rng.random_range(0..3))
            .map(|_| WORDS[rng.random_range(0..WORDS.len())])
            .collect::<Vec<_>>()
            .join(" ");
        let pad = "  ".repeat(rng.random_range(0..2));
        let line = match rng.random_range(0..9) {
            0..=2 => text,
            3 | 4 => format!("{pad}- {text}"),
            5 => format!("{pad}- [{}] {text}", ['a', 'b'][rng.random_range(0..2)]),
            6 => format!("{pad}1. {text}"),
            7 => format!("## {text}"),
            _ => format!("```\n{text}\n- in a fence\n```"),
        };
        md.push_str(&line);
        md.push('\n');
    }
    let mut s = markdown::load(
        &md,
        Some("p.md".into()),
        Viewport {
            width: 40,
            height: 10,
        },
        common::tagged_cfg(),
    );
    s.doc.set_host(common::retag_host());
    let len = s.doc.text.len_chars();
    let p = ensure_grapheme_boundary_prev(s.doc.text.slice(..), rng.random_range(0..=len));
    s.view.selection = Selection::point(p);
    update(&mut s, Msg::resize(40, 10));
    s
}

fn random_msg(rng: &mut StdRng, s: &State) -> Msg {
    let o = s.blocks().unwrap();
    let b = o.blocks[rng.random_range(0..o.blocks.len())].clone();
    let len = s.doc.text.len_chars();
    let text = s.doc.text.to_string();
    let at = |rng: &mut StdRng| {
        ensure_grapheme_boundary_prev(s.doc.text.slice(..), rng.random_range(0..=len))
    };
    let dir = if rng.random_bool(0.5) {
        Dir::Forward
    } else {
        Dir::Backward
    };
    match rng.random_range(0..30) {
        0..=9 => gen::msg(rng, s),
        10 => Msg::Indent,
        11 => Msg::Outdent,
        12 => Msg::MoveBlock { dir },
        13 => common::retag(),
        14 => Msg::InsertBlocks {
            after: rng.random_bool(0.8).then_some(b.id),
            blocks: vec![NewBlock {
                depth: 0,
                kind: Kind::Bullet,
                tag: None,
                text: "new".into(),
                gap: None,
                mark: None,
            }],
        },
        15 => Msg::Paste {
            text: Some("- a\n  - [a] b\n\npara".into()),
        },
        16 => Msg::Move {
            dir,
            by: if rng.random_bool(0.5) {
                By::Block
            } else {
                By::DocStart
            },
            extend: false,
        },
        // A host's edit of the whole text with one small change in it.
        17 | 18 => {
            let p = at(rng);
            let mut new: String = text.chars().take(p).collect();
            new.push_str(["x", "\n", "- ", "```\n", ""][rng.random_range(0..5)]);
            let skip = rng.random_range(0..3).min(len - p);
            new.extend(text.chars().skip(p + skip));
            Msg::Edit {
                changes: vec![(0, len, new)],
                join: false,
            }
        }
        19 => Msg::External {
            changes: vec![ExtChange::SetGap {
                id: b.id,
                gap: [None, Some(true), Some(false)][rng.random_range(0..3)],
            }],
        },
        20 => Msg::External {
            changes: vec![ExtChange::SetData {
                id: b.id,
                data: rng
                    .random_bool(0.7)
                    .then(|| serde_json::json!(rng.random_range(0..3))),
            }],
        },
        21 => Msg::External {
            changes: vec![ExtChange::RemoveBlock { id: b.id }],
        },
        22 => Msg::External {
            changes: vec![ExtChange::InsertBlock {
                after: Some(b.id),
                block: NewBlock {
                    depth: 1,
                    kind: Kind::Bullet,
                    tag: Some('a'),
                    text: "far".into(),
                    gap: Some(true),
                    mark: None,
                },
            }],
        },
        23 => Msg::External {
            changes: vec![ExtChange::SetShape {
                id: b.id,
                depth: rng.random_range(0..2),
                kind: Kind::Bullet,
                tag: None,
            }],
        },
        24 => {
            let (a, c) = (at(rng), at(rng));
            Msg::External {
                changes: vec![ExtChange::Replace {
                    from: a.min(c),
                    to: a.max(c),
                    text: ["", "y", "\n\n", "- z\n"][rng.random_range(0..4)].into(),
                }],
            }
        }
        25 => Msg::Command {
            name: "test.set_tag".into(),
            args: serde_json::json!({ "id": b.id.0, "tag": "b" }),
        },
        _ => Msg::Undo,
    }
}

/// What a host mirrors of a block: its text, shape, blank row and mark.
type Seen = (String, Kind, u16, Option<char>, bool, MarkAttrs);

fn seen(s: &State) -> HashMap<MarkId, Seen> {
    let o = s.blocks().unwrap();
    let text = s.doc.text.slice(..);
    o.blocks
        .iter()
        .map(|b| {
            (
                b.id,
                (
                    text.slice(b.start..b.end).to_string(),
                    b.kind,
                    b.depth,
                    b.tag,
                    b.gap,
                    b.attrs.clone(),
                ),
            )
        })
        .collect()
}

#[test]
fn touched_covers_every_block_that_changed() {
    let seeds: u64 = std::env::var("CARETLINE_TOUCHED_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let mut checked = 0;
    for seed in 0..seeds {
        let mut rng = StdRng::seed_from_u64(0x70c4 ^ seed.wrapping_mul(0x9e37_79b9));
        let mut s = random_doc(&mut rng);
        s.doc.take_touched();
        for step in 0..150 {
            let msg = random_msg(&mut rng, &s);
            if matches!(msg, Msg::Quit | Msg::Save) {
                continue;
            }
            let was = seen(&s);
            update(&mut s, msg.clone());
            let touched = s.doc.take_touched();
            let hit = touched_blocks(&s, touched);
            let o = s.blocks().unwrap();
            let is = seen(&s);
            for (i, b) in o.blocks.iter().enumerate() {
                let now = &is[&b.id];
                if was.get(&b.id) != Some(now) {
                    assert!(
                        hit.contains(&i),
                        "seed {seed} step {step} {msg:?}: block {i} {now:?} changed (was {:?}) \
                         outside {touched:?} in {:?}",
                        was.get(&b.id),
                        s.doc.text.to_string()
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 1000, "only {checked} changed blocks checked");
}
