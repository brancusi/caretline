//! A timed, interactive presentation driven by real editor and protocol operations.
//! No background agent: the clock supplies cues, the deck supplies their data.

mod deck;
mod graphics;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use caretline::helix::{Range, Selection};
use caretline::{Dir, ExtChange, Frame, Key, KeyCode, MarkId, Msg, Session, State, Viewport};
use caretline_layers::{
    Anchor, Content, FrameResolver, Grid, HINT, Layer, LayerOp, Layers, Limits, Rect, Renderers,
    apply, plan,
};
use serde_json::{Value, json};

use super::scenes::Animation;
use super::{DemoArgs, canvas::Canvas};
use crate::hub::Hub;
use crate::layers::{self, HintRenderer};
use crate::runtime::{self, Decor, Demo, Gfx, KeyAction, dispatch_demo};
use deck::{Art, SLIDES};

#[derive(Clone, Copy)]
enum Action {
    Caret(&'static str),
    Carets(&'static [&'static str]),
    Highlights(&'static [&'static str]),
    Type(&'static str),
    CheckStream,
    CheckMulti,
    UndoMulti,
    RedoMulti,
    CheckHighlights,
    OpenView,
    RemoteAppend(&'static str, &'static str),
    CheckViews,
    RememberRev,
    GuardedRemote,
    UndoLocal,
    RedoLocal,
    RememberMark,
    Fold,
    Unfold,
    MoveBlock,
    CheckMark,
    RoundTrip,
    ReplayCheck,
    Summary,
}

pub(super) struct Showcase {
    slide: usize,
    elapsed: Duration,
    play_time: Duration,
    motion: bool,
    animation: Option<(&'static str, Animation)>,
    last_frame: Option<(u64, u16, u16, usize)>,
    duration: Duration,
    last: Instant,
    entered: bool,
    next_cue: usize,
    paused: bool,
    autoplay: bool,
    spotlight: bool,
    generation: u64,
    canvas: Canvas,
    remembered_rev: u64,
    remembered_mark: Option<MarkId>,
    checks: BTreeMap<&'static str, bool>,
}

fn locate(text: &str, needle: &str) -> Option<(usize, usize)> {
    let byte = text.find(needle)?;
    let from = text[..byte].chars().count();
    Some((from, from + needle.chars().count()))
}

fn line_anchor(text: &str, needle: &str) -> Option<Anchor> {
    let byte = text.find(needle)?;
    let start = text[..byte].rfind('\n').map_or(0, |i| i + 1);
    let end = text[byte..].find('\n').map_or(text.len(), |i| byte + i);
    Some(Anchor::Text {
        from: text[..start].chars().count(),
        to: text[..end].chars().count(),
    })
}

fn state(slide: usize, viewport: Viewport) -> State {
    let mut state = crate::new_state(SLIDES[slide].text, None, viewport, SLIDES[slide].outline);
    state.view.layout = Some(caretline::OutlineLayout::default().with_hang_glyphs(true));
    state
}

impl Showcase {
    fn new(seconds: f64, now: Instant) -> Self {
        Self {
            slide: 0,
            elapsed: Duration::ZERO,
            play_time: Duration::ZERO,
            motion: true,
            animation: None,
            last_frame: None,
            duration: Duration::from_secs_f64(seconds),
            last: now,
            entered: false,
            next_cue: 0,
            paused: false,
            autoplay: true,
            spotlight: true,
            generation: 0,
            canvas: Canvas::new(),
            remembered_rev: 0,
            remembered_mark: None,
            checks: BTreeMap::new(),
        }
    }

    /// The same protocol route a socket client uses, with notifications to subscribers.
    fn request(hub: &mut Hub, request: Value) -> Value {
        let handled = hub.session.handle(&request.to_string(), None);
        if let Some(change) = &handled.change {
            hub.changed(change, "showcase");
        }
        serde_json::from_str(&handled.response).expect("protocol replies are JSON")
    }

    fn enter(&mut self, hub: &mut Hub, slide: usize) {
        // A slide is a new document state. Recover the full viewport before closing its pane.
        let old = &hub.session.state().view;
        let width = old.viewport.width;
        let height = old.viewport.height.saturating_add(
            hub.session
                .views()
                .first()
                .map_or(0, |(_, v)| v.viewport.height),
        );
        let cell_px = old.cell_px;
        let ids: Vec<_> = hub.session.views().iter().map(|(id, _)| *id).collect();
        for id in ids {
            Self::request(hub, json!({"op":"view.close", "view":id}));
        }
        let mut next = state(slide, Viewport { width, height });
        next.view.cell_px = cell_px;
        Self::request(hub, json!({"op":"state.set", "state":next}));
        self.slide = slide;
        self.elapsed = Duration::ZERO;
        self.play_time = Duration::ZERO;
        self.animation = None;
        self.last_frame = None;
        self.next_cue = 0;
        self.remembered_mark = None;
        self.entered = true;
        self.generation += 1;
    }

    fn select(&self, hub: &mut Hub, needles: &[&str], extend: bool, at_end: bool) {
        let mut s = hub.session.state().clone();
        let text = s.doc.text.to_string();
        let ranges: caretline::helix::SmallVec<[Range; 1]> = needles
            .iter()
            .filter_map(|needle| {
                let (from, to) = locate(&text, needle)?;
                Some(if extend {
                    Range::new(from, to)
                } else {
                    Range::point(if at_end { to } else { from })
                })
            })
            .collect();
        if !ranges.is_empty() {
            s.view.selection = Selection::new(ranges, 0);
            Self::request(hub, json!({"op":"state.set", "state":s}));
        }
    }

    fn replace(hub: &mut Hub, needle: &str, replacement: &str) {
        if let Some((from, to)) = locate(&hub.session.state().doc.text.to_string(), needle) {
            dispatch_demo(
                hub,
                vec![Msg::External {
                    changes: vec![ExtChange::Replace {
                        from,
                        to,
                        text: replacement.into(),
                    }],
                }],
            );
        }
    }

    fn action(&mut self, hub: &mut Hub, action: Action) {
        match action {
            Action::Caret(needle) => self.select(hub, &[needle], false, true),
            Action::Carets(needles) => self.select(hub, needles, false, false),
            Action::Highlights(needles) => self.select(hub, needles, true, false),
            Action::Type(text) => {
                dispatch_demo(hub, vec![Msg::InsertText { text: text.into() }]);
            }
            Action::CheckStream => {
                let text = hub.session.state().doc.text.to_string();
                self.checks.insert(
                    "unicode_stream",
                    text.contains("Hello, Caretline.") && text.contains("東京, 🦀"),
                );
            }
            Action::CheckMulti | Action::RedoMulti => {
                if matches!(action, Action::RedoMulti) {
                    dispatch_demo(hub, vec![Msg::Redo]);
                }
                let s = hub.session.state();
                self.checks.insert(
                    if matches!(action, Action::RedoMulti) {
                        "multi_redo"
                    } else {
                        "multi_edit"
                    },
                    s.doc.text.to_string().matches("ready ").count() == 3
                        && s.view.selection.ranges().len() == 3,
                );
            }
            Action::UndoMulti => {
                dispatch_demo(hub, vec![Msg::Undo]);
                let s = hub.session.state();
                self.checks.insert(
                    "multi_undo",
                    !s.doc.text.to_string().contains("ready ")
                        && s.view.selection.ranges().len() == 3,
                );
            }
            Action::CheckHighlights => {
                self.checks.insert(
                    "multiple_highlights",
                    hub.session.state().view.selection.ranges().len() == 3,
                );
            }
            Action::OpenView => {
                let mut view = hub.session.state().view.clone();
                let full = view.viewport;
                let rows = super::pane_rows(full.height);
                view.viewport.height = rows;
                view.status = Some("VIEW B · same document, independent caret".into());
                dispatch_demo(
                    hub,
                    vec![Msg::resize(full.width, full.height.saturating_sub(rows))],
                );
                Self::request(hub, json!({"op":"view.open", "open":view}));
            }
            Action::RemoteAppend(needle, text) => {
                let doc = &hub.session.state().doc.text;
                if let Some((from, _)) = locate(&doc.to_string(), needle) {
                    let line = doc.char_to_line(from);
                    let end = doc.line_to_char(line) + doc.line(line).len_chars().saturating_sub(1);
                    dispatch_demo(
                        hub,
                        vec![Msg::External {
                            changes: vec![ExtChange::Replace {
                                from: end,
                                to: end,
                                text: text.into(),
                            }],
                        }],
                    );
                }
            }
            Action::CheckViews => {
                let s = hub.session.state();
                let same = hub
                    .session
                    .views()
                    .first()
                    .and_then(|(id, _)| hub.session.state_of(*id))
                    .is_some_and(|other| {
                        other.doc.text == s.doc.text && other.view.caret() != s.view.caret()
                    });
                self.checks.insert(
                    "shared_views",
                    same && s.doc.text.to_string().contains("external update"),
                );
            }
            Action::RememberRev => self.remembered_rev = hub.session.rev(),
            Action::GuardedRemote => {
                let text = hub.session.state().doc.text.to_string();
                if let Some((_, at)) = locate(&text, "Agent: ") {
                    let msg = Msg::External {
                        changes: vec![ExtChange::Replace {
                            from: at,
                            to: at,
                            text: "remote text survives undo".into(),
                        }],
                    };
                    let stale = Self::request(
                        hub,
                        json!({"op":"msgs", "view":0, "if_rev":self.remembered_rev, "msgs":[msg]}),
                    );
                    let refused = stale["error"]["kind"] == "stale";
                    let retry = Self::request(
                        hub,
                        json!({"op":"msgs", "view":0, "if_rev":hub.session.rev(), "msgs":[msg]}),
                    );
                    self.checks
                        .insert("stale_guard", refused && retry.get("error").is_none());
                    Self::replace(
                        hub,
                        "Guard: waiting",
                        if refused {
                            "Guard: stale write refused; fresh revision accepted"
                        } else {
                            "Guard: FAILED"
                        },
                    );
                    Self::replace(
                        hub,
                        "Undo: waiting",
                        "Undo: next, remove only the person's edit",
                    );
                }
            }
            Action::UndoLocal => {
                dispatch_demo(hub, vec![Msg::Undo]);
                let text = hub.session.state().doc.text.to_string();
                let kept =
                    !text.contains("my local edit") && text.contains("remote text survives undo");
                self.checks.insert("remote_undo", kept);
                dispatch_demo(
                    hub,
                    vec![Msg::ShowStatus {
                        text: if kept {
                            "Undo verified: local edit removed; remote text kept"
                        } else {
                            "Undo: FAILED"
                        }
                        .into(),
                    }],
                );
            }
            Action::RedoLocal => {
                dispatch_demo(hub, vec![Msg::Redo]);
                let restored = hub
                    .session
                    .state()
                    .doc
                    .text
                    .to_string()
                    .contains("my local edit");
                self.checks.insert("remote_redo", restored);
                Self::replace(
                    hub,
                    "Undo: next, remove only the person's edit",
                    if restored && self.checks.get("remote_undo") == Some(&true) {
                        "Undo: VERIFIED, local only; redo restored the local text"
                    } else {
                        "Undo: FAILED"
                    },
                );
            }
            Action::RememberMark => {
                self.select(hub, &["Explore the engine"], false, false);
                let s = hub.session.state();
                self.remembered_mark = s
                    .blocks()
                    .map(|b| b.blocks[b.index_at(s.doc.text.slice(..), s.view.caret())].id);
            }
            Action::Fold | Action::Unfold => {
                if let Some(id) = self.remembered_mark {
                    dispatch_demo(
                        hub,
                        vec![if matches!(action, Action::Fold) {
                            Msg::Fold { id }
                        } else {
                            Msg::Unfold { id }
                        }],
                    );
                    if matches!(action, Action::Fold) {
                        self.checks
                            .insert("fold", hub.session.state().view.folds.contains(&id));
                    }
                }
            }
            Action::MoveBlock => {
                dispatch_demo(hub, vec![Msg::MoveBlock { dir: Dir::Forward }]);
            }
            Action::CheckMark => {
                let s = hub.session.state();
                let text = s.doc.text.to_string();
                let same = locate(&text, "Explore the engine").and_then(|(pos, _)| {
                    s.blocks()
                        .map(|b| b.blocks[b.index_at(s.doc.text.slice(..), pos)].id)
                }) == self.remembered_mark;
                let moved = text.find("Embed it in your host") < text.find("Explore the engine");
                self.checks.insert(
                    "stable_mark",
                    self.remembered_mark.is_some() && same && moved,
                );
                let verdict = format!(
                    "Mark #{}: {} across fold, unfold and subtree move.",
                    self.remembered_mark.map_or(0, |id| id.0),
                    if same && moved { "VERIFIED" } else { "FAILED" }
                );
                Self::replace(
                    hub,
                    "The block's mark remains the same across each operation.",
                    &verdict,
                );
            }
            Action::RoundTrip => {
                let json = hub.session.state().to_json();
                let exact = State::from_json(&json).is_ok_and(|loaded| loaded.to_json() == json);
                self.checks.insert("state_round_trip", exact);
                Self::replace(
                    hub,
                    "State round-trip: pending",
                    if exact {
                        "State round-trip: VERIFIED, undo history included"
                    } else {
                        "State round-trip: FAILED"
                    },
                );
            }
            Action::ReplayCheck => {
                let trace = hub.session.trace_jsonl();
                let target = hub.session.state().to_json();
                let exact = caretline::trace::replay_trace(&trace)
                    .is_ok_and(|(replayed, _)| replayed.to_json() == target);
                self.checks.insert("trace_replay", exact);
                Self::replace(
                    hub,
                    "Trace replay: pending",
                    if exact {
                        "Trace replay: VERIFIED, identical editor state"
                    } else {
                        "Trace replay: FAILED"
                    },
                );
            }
            Action::Summary => {
                let passed = self.checks.values().filter(|v| **v).count();
                Self::replace(
                    hub,
                    "Home restarts the showcase. Left revisits any slide.",
                    &format!(
                        "{passed}/{} live invariants verified. Home restarts; Left revisits.",
                        self.checks.len()
                    ),
                );
            }
        }
    }

    fn update_art(&mut self, hub: &mut Hub) {
        let art = SLIDES[self.slide].art;
        if !matches!(art, Art::Ascii | Art::Finale | Art::Motion) {
            return;
        }
        let phase = if self.motion {
            (self.play_time.as_secs_f64() * 24.0).floor() as u64
        } else {
            0
        };
        let viewport = hub.session.state().view.viewport;
        let scene_index = if art == Art::Ascii {
            (self.progress() as usize / 25).min(3)
        } else {
            0
        };
        let key = (phase, viewport.width, viewport.height, scene_index);
        if self.last_frame == Some(key) {
            return;
        }
        self.last_frame = Some(key);
        dispatch_demo(
            hub,
            vec![Msg::Frame {
                now_ms: (phase * 1000).div_ceil(24),
            }],
        );
        if matches!(art, Art::Ascii | Art::Finale) {
            let name = if art == Art::Finale {
                "warp"
            } else {
                ["donut", "cube", "tunnel", "plasma"][scene_index]
            };
            if self.animation.as_ref().is_none_or(|(old, _)| *old != name) {
                self.animation = Animation::new(name).map(|a| (name, a));
            }
            // Same newline margin as demo scenes: a full-width line would soft-wrap
            // its newline into an extra blank row and hide the score/caption.
            let width = viewport.width.saturating_sub(1).max(1) as usize;
            let height = viewport.height.saturating_sub(3).max(1) as usize;
            let (body, ranges) =
                self.animation
                    .as_mut()
                    .unwrap()
                    .1
                    .frame(phase as f64 / 24.0, width, height);
            let prefix = "\n";
            let passed = self.checks.values().filter(|v| **v).count();
            let caption = if art == Art::Finale {
                format!(
                    "{passed}/{} checks verified / the selection is the logo / q quit",
                    self.checks.len()
                )
            } else {
                format!("{name} / live frame + highlights / Space pauses / pixels not required")
            };
            let text = format!("{prefix}{body}\n{caption}");
            let offset = prefix.chars().count();
            let highlights: Vec<_> = ranges
                .into_iter()
                .map(|(from, to)| (from + offset, to + offset))
                .collect();
            Self::request(
                hub,
                json!({"op":"frame", "text":text, "highlights":highlights, "caret":0}),
            );
        }
        self.generation += 1;
    }

    fn progress(&self) -> u8 {
        ((self.elapsed.as_secs_f64() / self.duration.as_secs_f64()) * 100.0).min(100.0) as u8
    }

    fn layers(&self, text: &str) -> Layers {
        let mut result = Layers::default();
        let slide = &SLIDES[self.slide];
        let anchors: Vec<_> = slide
            .notes
            .iter()
            .filter_map(|n| line_anchor(text, n.anchor))
            .collect();
        for (i, note) in slide
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.at <= self.progress())
        {
            if let Some(anchor) = line_anchor(text, note.anchor) {
                let mut layer = Layer::new(anchor)
                    .with_content(Content::hint(Some(note.title), note.text))
                    .with_arrow()
                    .with_ring()
                    .with_avoid(anchors.clone());
                layer.id = format!("note-{i}");
                layer.max_width = Some(36);
                if i == 0 && self.spotlight {
                    layer = layer.with_spotlight();
                }
                let _ = apply(
                    &mut result,
                    LayerOp::Push(layer),
                    None,
                    0,
                    &Limits::default(),
                );
            }
        }
        for (i, needle) in slide.rings.iter().enumerate() {
            if let Some((from, to)) = locate(text, needle) {
                let mut ring = Layer::new(Anchor::Text { from, to }).with_ring();
                ring.id = format!("ring-{i}");
                let _ = apply(
                    &mut result,
                    LayerOp::Push(ring),
                    None,
                    0,
                    &Limits::default(),
                );
            }
        }
        result
    }

    fn press(&mut self, hub: &mut Hub, key: &Key, now: Instant) -> KeyAction {
        if key.mods.ctrl && matches!(key.code, KeyCode::Char('c' | 'q')) {
            return KeyAction::Quit;
        }
        if key.mods.ctrl || key.mods.alt || key.mods.cmd {
            return KeyAction::Consumed;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return KeyAction::Quit,
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Char('0'..='9') => {
                let slide = match key.code {
                    KeyCode::Left => self.slide.saturating_sub(1),
                    KeyCode::Right => (self.slide + 1).min(SLIDES.len() - 1),
                    KeyCode::End | KeyCode::Char('0') => SLIDES.len() - 1,
                    KeyCode::Char(c) => (c as usize - '1' as usize).min(SLIDES.len() - 1),
                    _ => 0,
                };
                if key.code == KeyCode::Home {
                    self.checks.clear();
                }
                self.enter(hub, slide);
                self.autoplay = key.code == KeyCode::Home;
                self.paused = false;
                self.last = now;
            }
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                self.last = now;
            }
            KeyCode::Char('a') => {
                self.autoplay = !self.autoplay;
                self.paused = false;
                self.last = now;
            }
            KeyCode::Char('r') => {
                self.enter(hub, self.slide);
                self.paused = false;
                self.last = now;
            }
            KeyCode::Char('s') => self.spotlight = !self.spotlight,
            KeyCode::Char('p') => self.canvas.pixels = !self.canvas.pixels,
            KeyCode::Char('t') => self.canvas.toggle_transport(),
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                let by = if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
                    8
                } else {
                    1
                };
                let rows = if matches!(key.code, KeyCode::Up | KeyCode::PageUp) {
                    -by
                } else {
                    by
                };
                dispatch_demo(hub, vec![Msg::ScrollView { rows }]);
            }
            _ => return KeyAction::Consumed,
        }
        self.generation += 1;
        KeyAction::Consumed
    }
}

impl Demo for Showcase {
    fn key(&mut self, hub: &mut Hub, key: &Key) -> KeyAction {
        self.press(hub, key, Instant::now())
    }

    fn paste(&mut self, _hub: &mut Hub, _text: &str) -> KeyAction {
        KeyAction::Consumed
    }

    fn after(&mut self, hub: &mut Hub) {
        if self.entered {
            self.update_art(hub);
        }
    }

    fn poll(&mut self, hub: &mut Hub, now: Instant) -> Option<Instant> {
        if !self.entered {
            self.enter(hub, 0);
            self.last = now;
        }
        let delta = now.saturating_duration_since(self.last);
        self.last = now;
        if self.paused {
            return None;
        }
        let before = self.progress();
        self.elapsed += delta;
        self.play_time += delta;
        loop {
            while let Some(&(percent, action)) = SLIDES[self.slide].cues.get(self.next_cue) {
                if self.elapsed < self.duration.mul_f64(percent as f64 / 100.0) {
                    break;
                }
                // Give the engine a deterministic clock, independent of paint frequency.
                dispatch_demo(
                    hub,
                    vec![Msg::Tick {
                        now_ms: hub.session.state().doc.now_ms.saturating_add(2000),
                    }],
                );
                self.action(hub, action);
                self.next_cue += 1;
                self.generation += 1;
            }
            if self.elapsed < self.duration {
                break;
            }
            if !self.autoplay || self.slide + 1 == SLIDES.len() {
                // A held slide has no accumulated overtime. Resuming automatic advance
                // must start the next slide, not race through the rest of the deck.
                self.elapsed = self.duration;
                break;
            }
            let remaining = self.elapsed.saturating_sub(self.duration);
            self.enter(hub, self.slide + 1);
            self.elapsed = remaining;
            self.play_time = remaining;
        }
        self.update_art(hub);
        if self.progress() != before {
            self.generation += 1;
        }
        let animated = self.motion
            && matches!(
                SLIDES[self.slide].art,
                Art::Ascii | Art::Motion | Art::Finale
            );
        if animated {
            Some(now + Duration::from_millis(42))
        } else if self.elapsed >= self.duration {
            None
        } else {
            Some(now + Duration::from_millis(100))
        }
    }

    fn pane_rows(&self, hub: &Hub, height: u16) -> u16 {
        if hub.session.views().is_empty() {
            0
        } else {
            super::pane_rows(height)
        }
    }

    fn generation(&self) -> u64 {
        self.generation
    }
    fn wants_pixels(&self) -> bool {
        true
    }
    fn hide(&mut self) -> Vec<u8> {
        self.canvas.hide()
    }

    fn decorate(&mut self, hub: &Hub, frame: &mut Frame, gfx: &Gfx) -> Decor {
        let art = SLIDES[self.slide].art;
        let brand_text = layers::role(frame, "brand.text");
        let brand_selection = layers::role(frame, "brand.selection");
        let brand_chrome = layers::role(frame, "brand.chrome");
        for cell in &mut frame.cells {
            cell.role = match cell.role {
                caretline::view::Role::Selection => brand_selection,
                caretline::view::Role::Status | caretline::view::Role::StatusAccent => brand_chrome,
                caretline::view::Role::Text => brand_text,
                other => other,
            };
        }
        let layers = self.layers(&hub.session.state().doc.text.to_string());
        let area = Rect::new(0, 1, frame.width, frame.height.saturating_sub(2));
        // Reserve every line's text span. Callouts and arrows use the whitespace,
        // never draw through the copy they are meant to explain.
        let protected: Vec<_> = (1..frame.height.saturating_sub(1))
            .filter_map(|y| {
                let xs: Vec<_> = (0..frame.width)
                    .filter(|&x| !frame.cell(x, y).symbol.trim().is_empty())
                    .collect();
                Some(Rect::new(*xs.first()?, y, xs.last()? - xs.first()? + 1, 1))
            })
            .collect();
        let grid = Grid::from_frame(frame)
            .with_area(area)
            .with_protect(protected);
        let mut p = plan(
            &layers,
            &FrameResolver::new(frame),
            &grid,
            &Renderers::new().register(HINT, HintRenderer),
        );
        // One presentation spotlight, with holes for EVERY active annotation. Independent
        // spotlights would intersect and dim each other's targets. This is derived geometry.
        let holes: Vec<_> = p
            .layers
            .iter()
            .flat_map(|l| l.rect.into_iter().chain(l.ring.iter().copied()))
            .collect();
        for spot in &mut p.spots {
            spot.holes.extend(holes.iter().copied());
        }
        let decor = if matches!(art, Art::Typography | Art::Motion) {
            graphics::paint_graphics(
                &mut self.canvas,
                art,
                frame,
                gfx,
                hub.session.state().doc.now_ms as f64 / 1000.0,
            )
        } else {
            self.canvas.paint_plan(&layers, frame, gfx, &p, area)
        };
        let count = (self.progress() as usize / 10).min(10);
        let bar = format!("{}{}", "━".repeat(count), "·".repeat(10 - count));
        let heading = format!(
            " CARETLINE  {:02}/{}  {}  [{}]",
            self.slide + 1,
            SLIDES.len(),
            SLIDES[self.slide].title,
            bar
        );
        layers::status(frame, 0, &heading);
        let mode = if self.paused {
            "PAUSED"
        } else if self.progress() == 100 && self.slide + 1 == SLIDES.len() {
            "DONE"
        } else if self.autoplay {
            "AUTO"
        } else {
            "MANUAL"
        };
        let footer = format!(
            " {mode} · q quit · ←→ slides · Space pause · r replay · a auto · {}",
            self.canvas.mode(gfx)
        );
        layers::status(frame, frame.height.saturating_sub(1), &footer);
        for y in [0, frame.height.saturating_sub(1)] {
            for x in 0..frame.width {
                frame.cells[y as usize * frame.width as usize + x as usize].role = brand_chrome;
            }
        }
        frame.cursor = None;
        decor
    }
}

pub(super) fn main(args: &DemoArgs) -> Result<(), String> {
    if args.font_licenses {
        println!("{}", graphics::licenses());
        return Ok(());
    }
    let seconds = args.seconds.unwrap_or(14.0);
    if !seconds.is_finite() || !(2.0..=300.0).contains(&seconds) {
        return Err("showcase --seconds must be between 2 and 300".into());
    }
    if args.headless {
        let now = Instant::now();
        let mut hub = Hub::new(
            Session::new(state(
                0,
                Viewport {
                    width: 100,
                    height: 30,
                },
            )),
            None,
        );
        let mut demo = Showcase::new(seconds, now);
        demo.motion = !args.reduced_motion;
        demo.poll(&mut hub, now);
        demo.poll(&mut hub, now + demo.duration * SLIDES.len() as u32);
        let ok = demo.checks.len() == 13 && demo.checks.values().all(|v| *v);
        println!("{}", serde_json::to_string_pretty(&json!({"ok":ok, "slides":SLIDES.len(), "checks":demo.checks, "rev":hub.session.rev()})).unwrap());
        return if ok {
            Ok(())
        } else {
            Err("showcase invariant checks failed".into())
        };
    }
    if let Some(size) = &args.snapshot {
        let (w, h) = crate::parse_size(size)?;
        let f = snapshot(w, h, args.keys.as_deref(), seconds, args.reduced_motion)?;
        print!(
            "{}",
            if args.format == "ansi" {
                f.to_ansi()
            } else {
                f.to_text()
            }
        );
        return Ok(());
    }
    let (width, height) = crossterm::terminal::size().unwrap_or((100, 30));
    let socket = args
        .socket
        .clone()
        .map_or_else(crate::hub::default_socket_path, Ok)?;
    let mut demo = Showcase::new(seconds, Instant::now());
    demo.motion = !args.reduced_motion;
    runtime::run_interactive(
        state(0, Viewport { width, height }),
        runtime::Interactive {
            trace: None,
            mouse: !args.no_mouse,
            listen: Some(socket),
            file: None,
            // Continuous frame checkpoints stay bounded while the finale loops.
            trace_limit: 512,
            max_fps: 60,
            frame_clock: 0,
            stats: false,
            demo: Some(Box::new(demo)),
        },
    )
}

fn snapshot(
    w: u16,
    h: u16,
    keys: Option<&str>,
    seconds: f64,
    reduced_motion: bool,
) -> Result<Frame, String> {
    let mut now = Instant::now();
    let mut hub = Hub::new(
        Session::new(state(
            0,
            Viewport {
                width: w,
                height: h,
            },
        )),
        None,
    );
    let mut demo = Showcase::new(seconds, now);
    demo.motion = !reduced_motion;
    demo.poll(&mut hub, now);
    for item in caretline::parse_keys(keys.unwrap_or(""))? {
        match item {
            caretline::keymap::ScriptItem::Key(key) => {
                if matches!(demo.press(&mut hub, &key, now), KeyAction::Quit) {
                    break;
                }
            }
            caretline::keymap::ScriptItem::Wait(ms) => {
                now += Duration::from_millis(ms);
                demo.poll(&mut hub, now);
            }
        }
    }
    let rows = demo.pane_rows(&hub, h);
    let mut frame = runtime::compose(&hub, rows);
    demo.decorate(&hub, &mut frame, &Gfx::default());
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use caretline_layers::kitty::CellPx;

    fn setup(seconds: f64, now: Instant) -> (Showcase, Hub) {
        let mut hub = Hub::new(
            Session::new(state(
                0,
                Viewport {
                    width: 100,
                    height: 30,
                },
            )),
            None,
        );
        let mut demo = Showcase::new(seconds, now);
        demo.poll(&mut hub, now);
        (demo, hub)
    }

    fn key(code: KeyCode) -> Key {
        Key {
            code,
            mods: Default::default(),
        }
    }

    #[test]
    fn every_live_invariant_holds_and_delayed_polls_skip_no_cues() {
        let now = Instant::now();
        let (mut fine, mut a) = setup(2.0, now);
        let (mut coarse, mut b) = setup(2.0, now);
        for tick in 1..=SLIDES.len() as u64 * 20 {
            fine.poll(&mut a, now + Duration::from_millis(tick * 100));
        }
        coarse.poll(&mut b, now + Duration::from_secs(SLIDES.len() as u64 * 2));
        assert_eq!(fine.slide, SLIDES.len() - 1);
        assert_eq!(fine.checks.len(), 13);
        assert!(fine.checks.values().all(|v| *v), "{:?}", fine.checks);
        assert_eq!(fine.checks, coarse.checks);
        assert_eq!(a.session.state().to_json(), b.session.state().to_json());
        assert!(
            a.session
                .state()
                .doc
                .text
                .to_string()
                .contains("13/13 checks verified")
        );
    }

    #[test]
    fn pause_navigation_replay_and_resume_have_predictable_clocks() {
        let now = Instant::now();
        let (mut demo, mut hub) = setup(14.0, now);
        demo.press(&mut hub, &key(KeyCode::Char(' ')), now);
        let later = now + Duration::from_secs(100);
        assert!(demo.poll(&mut hub, later).is_none());
        assert_eq!(demo.progress(), 0);
        demo.press(&mut hub, &key(KeyCode::Right), later);
        demo.poll(&mut hub, later + Duration::from_secs(100));
        assert_eq!(
            demo.slide, 1,
            "manual navigation runs the slide, then holds"
        );
        assert!(
            hub.session
                .state()
                .doc
                .text
                .to_string()
                .contains("Hello, Caretline.")
        );
        demo.press(&mut hub, &key(KeyCode::Char('r')), later);
        assert!(!hub.session.state().doc.text.to_string().contains("Hello"));
        demo.press(&mut hub, &key(KeyCode::Home), later);
        assert!(demo.autoplay && demo.checks.is_empty());
        demo.poll(&mut hub, later + Duration::from_secs(15));
        assert_eq!(demo.slide, 1);
    }

    #[test]
    fn annotations_never_overwrite_the_text_they_explain() {
        let now = Instant::now();
        let (mut demo, mut hub) = setup(14.0, now);
        for slide in 0..SLIDES.len() {
            demo.enter(&mut hub, slide);
            demo.autoplay = false;
            demo.last = now;
            demo.poll(&mut hub, now + Duration::from_secs(14));
            let rows = demo.pane_rows(&hub, 30);
            let original = runtime::compose(&hub, rows);
            let mut painted = original.clone();
            demo.decorate(&hub, &mut painted, &Gfx::default());
            for y in 1..original.height.saturating_sub(1) {
                for x in 0..original.width {
                    let cell = original.cell(x, y);
                    if cell.char_idx.is_some() && !cell.symbol.trim().is_empty() {
                        assert_eq!(
                            cell.symbol,
                            painted.cell(x, y).symbol,
                            "slide {slide}, ({x},{y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn multiple_pixel_layers_cache_and_rapid_transport_mode_switches_clean_up() {
        let now = Instant::now();
        let (mut demo, mut hub) = setup(14.0, now);
        demo.press(&mut hub, &key(KeyCode::Char('4')), now);
        demo.poll(&mut hub, now + Duration::from_secs(8));
        let gfx = Gfx {
            cell_px: Some(CellPx::new(8, 16)),
            ..Default::default()
        };
        let mut f = runtime::compose(&hub, 0);
        let first = demo.decorate(&hub, &mut f, &gfx);
        assert!(
            String::from_utf8_lossy(&first.bytes)
                .matches("a=t,")
                .count()
                >= 7,
            "two panels, rings, spotlight"
        );
        let mut f = runtime::compose(&hub, 0);
        assert!(demo.decorate(&hub, &mut f, &gfx).bytes.is_empty());
        demo.press(&mut hub, &key(KeyCode::Char('t')), now);
        demo.press(&mut hub, &key(KeyCode::Char('p')), now);
        let mut f = runtime::compose(&hub, 0);
        let output = demo.decorate(&hub, &mut f, &gfx);
        assert!(String::from_utf8_lossy(&output.bytes).contains("a=d,d=I"));
        assert!(demo.hide().is_empty());
        demo.press(&mut hub, &key(KeyCode::Char('t')), now); // back to inline
        demo.press(&mut hub, &key(KeyCode::Char('p')), now);
        let mut f = runtime::compose(&hub, 0);
        demo.decorate(&hub, &mut f, &gfx);
        demo.press(&mut hub, &key(KeyCode::Char('t')), now);
        assert!(
            String::from_utf8_lossy(&demo.hide()).contains("a=d,d=I"),
            "quit drains pending cleanup too"
        );
        assert!(demo.hide().is_empty());
    }

    #[test]
    fn finale_motion_pauses_and_continuous_frame_history_is_bounded() {
        let now = Instant::now();
        let (mut demo, mut hub) = setup(2.0, now);
        hub.session.set_trace_limit(32);
        demo.press(&mut hub, &key(KeyCode::End), now);
        demo.poll(&mut hub, now + Duration::from_secs(1));
        let moving = hub.session.state().doc.text.to_string();
        demo.press(
            &mut hub,
            &key(KeyCode::Char(' ')),
            now + Duration::from_secs(1),
        );
        demo.poll(&mut hub, now + Duration::from_secs(20));
        assert_eq!(moving, hub.session.state().doc.text.to_string());
        demo.press(
            &mut hub,
            &key(KeyCode::Char(' ')),
            now + Duration::from_secs(20),
        );
        for frame in 1..=100 {
            demo.poll(
                &mut hub,
                now + Duration::from_secs(20) + Duration::from_millis(frame * 42),
            );
        }
        assert_ne!(moving, hub.session.state().doc.text.to_string());
        assert!(hub.session.trace().len() <= 32);
        demo.motion = false;
        demo.last_frame = None;
        demo.poll(&mut hub, now + Duration::from_secs(30));
        let still = hub.session.state().to_json();
        assert!(demo.poll(&mut hub, now + Duration::from_secs(35)).is_none());
        assert_eq!(still, hub.session.state().to_json());
    }

    #[test]
    fn small_windows_use_the_same_presentation_without_panicking() {
        for (w, h) in [(1, 1), (20, 6), (44, 16), (80, 24)] {
            for slide in 1..=SLIDES.len() {
                let keys = format!("1{}<wait:14000>", "<right>".repeat(slide - 1));
                let f = snapshot(w, h, Some(&keys), 14.0, false).unwrap();
                assert_eq!(f.width, w);
                assert_eq!(f.height, h);
            }
        }
    }
}
