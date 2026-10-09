//! `caretline demo` (the default): the welcome, where a new user starts. It is a caretline
//! outline document like any other: the highlight on the chosen line is the editor's own
//! block selection, and the callout beside it is a layer (pixels in Ghostty, cells
//! elsewhere). Enter runs the chosen chapter, one of the other demos, and the welcome
//! comes back with that chapter ticked.
//!
//! Keys: ↑ ↓ (or j k, Tab) choose, Enter or 1–6 start, q quits. The text is read-only here.

use std::cell::Cell;
use std::rc::Rc;

use caretline::{Frame, Key, KeyCode, MarkId, Msg, OutlineLayout, State, Viewport};
use caretline_layers::{Anchor, Content, Layer, LayerOp, Layers, Limits, Spotlight, apply};

use super::canvas::Canvas;
use crate::hub::Hub;
use crate::layers;
use crate::runtime::{Decor, Demo, Gfx, KeyAction, dispatch_demo};

/// One line to choose on the welcome page.
pub(crate) struct Entry {
    /// The line's text, after its bullet.
    pub line: &'static str,
    /// The demo it starts, if it's a chapter (the rest are commands to take away).
    pub demo: Option<&'static str>,
    pub title: &'static str,
    pub text: &'static str,
}

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        line: "Watch · what caretline is, in three minutes",
        demo: Some("showcase"),
        title: "Watch it work",
        text: "Twelve timed slides of real edits, not pictures: many carets, shared views, an agent's guarded writes, overlays, and checks that run as you watch. ←→ steps, Space pauses, q comes back here.",
    },
    Entry {
        line: "Learn by doing · the editor, step by step",
        demo: Some("tour"),
        title: "Hands on",
        text: "Eleven short steps, each with its keys in the status bar: wrapping, word moves, selection, many carets, exact undo, indent, folds, marks, a second view, the state as JSON and a replay. ⌃Q comes back here.",
    },
    Entry {
        line: "Co-edit · an agent types beside you",
        demo: Some("agent"),
        title: "You and an agent",
        text: "A scripted agent connects over the editor's socket and writes in a view of its own while you type. Its stale writes are refused and retried, and ⌃Z undoes only yours.",
    },
    Entry {
        line: "Layers · hints, arrows and spotlights",
        demo: Some("layers"),
        title: "Draw over the text",
        text: "This callout is a layer: anchored to text, it follows it as it scrolls. In Ghostty it draws in pixels, elsewhere in cells; p switches between them, s toggles the spotlight.",
    },
    Entry {
        line: "Scenes · animation streamed into the editor",
        demo: Some("scenes"),
        title: "Frames over the protocol",
        text: "Warp, donut, cube, tunnel, plasma and fire, pushed into a live editor as whole frames over its socket. → or Space for the next scene, ← the one before, q comes back here.",
    },
    Entry {
        line: "caretline notes.md --outline",
        demo: None,
        title: "Edit your own file",
        text: "Lists, headings, folds and marks, read from Markdown and written back as Markdown. F1 shows every key. Quit here with q, then run it in your shell.",
    },
    Entry {
        line: "claude mcp add caretline -- caretline-mcp",
        demo: None,
        title: "Let your agent in",
        text: "The MCP server gives an agent its own caret in the editor you're typing in, with guarded writes, so it never overwrites what you just typed. Needs caretline-mcp: cargo install caretline-mcp.",
    },
    Entry {
        line: "caretline notes.md --keys 'hi<cr>' --snapshot 80x24",
        demo: None,
        title: "Drive it headless",
        text: "Keys, messages and snapshots without a terminal: the same engine, as a command. Every session is a trace you can replay to the identical state, which is how the goldens test it.",
    },
    Entry {
        line: "cargo add caretline",
        demo: None,
        title: "Embed the engine",
        text: "A pure Rust library: one serializable State, changed only by messages through update. Bring your own terminal, keys, commands and decorations. Guides at caretline.app/docs.",
    },
];

/// The page: a title, what caretline is, the chapters, and the commands to take away. Seen
/// chapters get a tick.
pub(crate) fn text(seen: &[bool]) -> String {
    let mut s = String::from(
        "# caretline\n\n\
         A text editor engine for the terminal.\n\
         The whole editor is one value: text, carets, undo, folds, views.\n\
         Save it, send it, replay it exactly. This page is one too.\n\n\
         ## Start here\n\n",
    );
    let line = |s: &mut String, i: usize, e: &Entry| {
        let tick = if seen.get(i).copied().unwrap_or(false) {
            "  ✓"
        } else {
            ""
        };
        s.push_str(&format!("- {}{tick}\n", e.line));
    };
    let chapters = ENTRIES.iter().take_while(|e| e.demo.is_some()).count();
    for (i, e) in ENTRIES[..chapters].iter().enumerate() {
        line(&mut s, i, e);
    }
    s.push_str("\n## Then make it yours\n\n");
    for (i, e) in ENTRIES.iter().enumerate().skip(chapters) {
        line(&mut s, i, e);
    }
    s.push_str("\nEvery chapter comes back here when it ends. q quits.\n");
    s
}

/// The welcome's state: `text(seen)` as an outline, `chosen` selected.
pub(crate) fn state(seen: &[bool], chosen: usize, viewport: Viewport) -> State {
    let mut state = crate::new_state(&text(seen), Some("welcome.md".into()), viewport, true);
    state.view.layout = Some(OutlineLayout::default().with_hang_glyphs(true));
    let mut hub = Hub::new(caretline::Session::new(state), None);
    select(&mut hub, chosen);
    hub.session.state().clone()
}

/// The marks of the entries' blocks, in `ENTRIES` order.
fn items(state: &State) -> Vec<MarkId> {
    state
        .blocks()
        .map(|o| {
            o.blocks
                .iter()
                .filter(|b| b.is_item())
                .map(|b| b.id)
                .collect()
        })
        .unwrap_or_default()
}

/// Selects entry `i`'s line: the editor's own block selection is the highlight.
fn select(hub: &mut Hub, i: usize) {
    if let Some(&id) = items(hub.session.state()).get(i) {
        dispatch_demo(hub, vec![Msg::SelectBlock { id }]);
    }
}

/// The entry the caret is in (a click can put it anywhere), or the nearest one above it.
fn chosen(state: &State) -> usize {
    let Some(o) = state.blocks() else {
        return 0;
    };
    let at = o.index_at(state.doc.text.slice(..), state.view.caret());
    let ids = items(state);
    o.blocks[..=at.min(o.blocks.len().saturating_sub(1))]
        .iter()
        .rev()
        .find_map(|b| ids.iter().position(|&id| id == b.id))
        .unwrap_or(0)
}

pub(crate) struct WelcomeDemo {
    /// Where the chosen chapter goes when Enter quits the page.
    pub start: Rc<Cell<Option<usize>>>,
    canvas: Canvas,
    generation: u64,
}

impl WelcomeDemo {
    pub(crate) fn new(start: Rc<Cell<Option<usize>>>) -> WelcomeDemo {
        WelcomeDemo {
            start,
            canvas: Canvas::new(),
            generation: 0,
        }
    }

    /// The callout on the chosen line, with a ring and the spotlight.
    pub(crate) fn layers(state: &State) -> Layers {
        let mut layers = Layers::default();
        let i = chosen(state);
        let (Some(e), Some(o)) = (ENTRIES.get(i), state.blocks()) else {
            return layers;
        };
        let Some(b) = items(state).get(i).and_then(|&id| o.get(id)) else {
            return layers;
        };
        let mut l = Layer::new(Anchor::Text {
            from: b.content_start(),
            to: b.end,
        })
        .with_content(Content::hint(Some(e.title), e.text))
        .with_arrow();
        l.id = "welcome".into();
        l.spotlight = Some(Spotlight::default());
        let _ = apply(&mut layers, LayerOp::Push(l), None, 0, &Limits::default());
        layers
    }

    /// The page's status line, over the editor's.
    fn status(&self, state: &State, frame: &mut Frame, gfx: &Gfx) {
        let Some(y) = frame.height.checked_sub(1) else {
            return;
        };
        let what = match ENTRIES.get(chosen(state)) {
            Some(Entry { demo: Some(_), .. }) => "Enter starts",
            _ => "to try in your shell",
        };
        let text = format!(
            " caretline · ↑↓ choose · {what} · q quit · {}",
            self.canvas.mode(gfx)
        );
        layers::status(frame, y, &text);
    }
}

impl Demo for WelcomeDemo {
    fn key(&mut self, hub: &mut Hub, key: &Key) -> KeyAction {
        let plain = !key.mods.ctrl && !key.mods.alt && !key.mods.cmd;
        let at = chosen(hub.session.state());
        let n = ENTRIES.len();
        let mut go = |hub: &mut Hub, i: usize| {
            select(hub, i);
            self.generation += 1;
            KeyAction::Consumed
        };
        match key.code {
            KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') if plain => {
                go(hub, (at + n - 1) % n)
            }
            KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') if plain => go(hub, (at + 1) % n),
            KeyCode::Home | KeyCode::PageUp => go(hub, 0),
            KeyCode::End | KeyCode::PageDown => go(hub, n - 1),
            KeyCode::Enter if ENTRIES[at].demo.is_some() => {
                self.start.set(Some(at));
                KeyAction::Quit
            }
            KeyCode::Char(c @ '1'..='9') if plain => {
                let i = c as usize - '1' as usize;
                if ENTRIES.get(i).is_some_and(|e| e.demo.is_some()) {
                    self.start.set(Some(i));
                    KeyAction::Quit
                } else {
                    KeyAction::Consumed
                }
            }
            KeyCode::Char('p') if plain => {
                self.canvas.pixels = !self.canvas.pixels;
                self.generation += 1;
                KeyAction::Consumed
            }
            KeyCode::Char('q') | KeyCode::Esc if plain => KeyAction::Quit,
            KeyCode::Char('c' | 'q') if key.mods.ctrl => KeyAction::Quit,
            // Read-only: nothing else edits.
            _ => KeyAction::Consumed,
        }
    }

    fn paste(&mut self, _hub: &mut Hub, _text: &str) -> KeyAction {
        KeyAction::Consumed
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn wants_pixels(&self) -> bool {
        true
    }

    fn decorate(&mut self, hub: &Hub, frame: &mut Frame, gfx: &Gfx) -> Decor {
        let state = hub.session.state();
        let decor = self
            .canvas
            .paint(&WelcomeDemo::layers(state), frame, gfx, None);
        self.status(state, frame, gfx);
        decor
    }

    fn hide(&mut self) -> Vec<u8> {
        self.canvas.hide()
    }
}

/// The page at `w`×`h`, headless, after `keys` (the page's own keys), in cells.
pub(crate) fn snapshot(w: u16, h: u16, keys: Option<&str>) -> Result<Frame, String> {
    let start = Rc::new(Cell::new(None));
    let mut demo = WelcomeDemo::new(start);
    let viewport = Viewport {
        width: w,
        height: h,
    };
    let mut hub = Hub::new(caretline::Session::new(state(&[], 0, viewport)), None);
    for item in caretline::parse_keys(keys.unwrap_or(""))? {
        if let caretline::keymap::ScriptItem::Key(key) = item
            && matches!(demo.key(&mut hub, &key), KeyAction::Quit)
        {
            break;
        }
    }
    let mut frame = crate::runtime::compose(&hub, 0);
    let gfx = Gfx {
        why: "snapshot".into(),
        ..Default::default()
    };
    demo.decorate(&hub, &mut frame, &gfx);
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_a_line_of_the_page() {
        let s = state(
            &[],
            0,
            Viewport {
                width: 80,
                height: 24,
            },
        );
        assert_eq!(items(&s).len(), ENTRIES.len());
        for (i, e) in ENTRIES.iter().enumerate() {
            let s = state(
                &[],
                i,
                Viewport {
                    width: 80,
                    height: 24,
                },
            );
            assert_eq!(chosen(&s), i, "{}", e.line);
        }
    }

    #[test]
    fn chapters_come_first_and_name_real_demos() {
        let chapters = ENTRIES.iter().take_while(|e| e.demo.is_some()).count();
        assert!(ENTRIES[chapters..].iter().all(|e| e.demo.is_none()));
        for e in &ENTRIES[..chapters] {
            assert!(["showcase", "tour", "agent", "layers", "scenes"].contains(&e.demo.unwrap()));
        }
    }

    #[test]
    fn seen_chapters_are_ticked() {
        assert!(text(&[true]).contains(&format!("- {}  ✓", ENTRIES[0].line)));
        assert!(!text(&[]).contains('✓'));
    }
}
