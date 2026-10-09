//! The terminal harness (step 4): a key press driven from the keyboard to the screen, in
//! process. [`Harness::press`] sends each key through a simulated terminal ([`TermKeyboard`]:
//! its bindings and its encoding), the runtime's decoder (`rawin::Parser`), `terminal_msgs`
//! and the keymap, `update`, the view, the runtime's real `draw`, and a terminal emulator; then
//! checks that what the screen shows is the state ([`Harness::check`]). No thread, clock, TTY
//! or system clipboard: time is a counter and the clipboard is a field, as in the engine.
//!
//! ```text
//! let mut h = Harness::new("hello ▮world", TermKeyboard::ghostty_default(), 40, 6);
//! h.press("<d-a>");               // default Ghostty keeps Cmd-A: nothing happens
//! assert_eq!(h.show(), "hello ▮world");
//! ```

use std::process::Command;

use caretline::helix::{Range, Selection};
use caretline::view::Role;
use caretline::{Effect, Frame, Key, Msg, State, Viewport, update, view};
use crossterm::event::Event;

use crate::keyboard::{Sent, TermKeyboard};
use crate::rawin::{Parser, Token};
use crate::screen::{Painter, Screen, Vt100, assert_screen_is_frame, term_color};

/// Text with carets: `▮` is a caret (several make several carets, the last the primary), and
/// `⟦…⟧` around one of them a selection (`⟦abc▮⟧` left to right, `⟦▮abc⟧` right to left).
pub fn parse(notation: &str) -> (String, Selection) {
    let mut text = String::new();
    let mut n = 0usize;
    let mut carets = Vec::new();
    let mut open = None;
    let mut ranges: Vec<Range> = Vec::new();
    for c in notation.chars() {
        match c {
            '▮' => carets.push(n),
            '⟦' => open = Some(n),
            '⟧' => {
                let o = open.take().expect("⟧ without ⟦");
                let head = carets.pop().expect("⟦…⟧ needs its ▮");
                let anchor = if head == o { n } else { o };
                ranges.push(Range::new(anchor, head));
            }
            c => {
                text.push(c);
                n += 1;
            }
        }
    }
    ranges.extend(carets.into_iter().map(Range::point));
    ranges.sort_by_key(|r| r.head);
    assert!(!ranges.is_empty(), "notation needs a caret ▮: {notation:?}");
    let mut sel = Selection::single(ranges[0].anchor, ranges[0].head);
    for r in &ranges[1..] {
        sel = sel.push(*r);
    }
    (text, sel)
}

/// The state's selection in the same notation.
pub fn show(state: &State) -> String {
    let text = state.doc.text.to_string();
    let sel = &state.view.selection;
    let n = text.chars().count();
    let mut out = String::new();
    for (i, c) in text.chars().chain(std::iter::once('\0')).enumerate() {
        for r in sel.iter() {
            let (from, to) = (r.from(), r.to());
            if r.is_empty() {
                if i == r.head {
                    out.push('▮');
                }
            } else if i == from {
                out.push('⟦');
                if r.head == from {
                    out.push('▮');
                }
            }
            if !r.is_empty() && i == to {
                if r.head == to {
                    out.push('▮');
                }
                out.push('⟧');
            }
        }
        if i < n {
            out.push(c);
        }
    }
    out
}

/// One editor, its terminal and its screen.
pub struct Harness {
    pub state: State,
    pub keyboard: TermKeyboard,
    pub screen: Vt100,
    /// The system clipboard: copies land here, pastes read it.
    pub clipboard: Option<String>,
    /// What each press did, for failure messages.
    pub log: Vec<String>,
    parser: Parser,
    painter: Painter,
    now_ms: u64,
}

impl Harness {
    /// An editor on `notation` in a `width`×`height` terminal using `keyboard`.
    pub fn new(notation: &str, keyboard: TermKeyboard, width: u16, height: u16) -> Harness {
        let (text, sel) = parse(notation);
        let mut state = State::new(&text, Some("test.txt".into()), Viewport { width, height });
        state.view.selection = sel;
        update(&mut state, Msg::resize(width, height));
        let mut h = Harness {
            state,
            keyboard,
            screen: Vt100::new(width, height),
            clipboard: None,
            log: Vec::new(),
            parser: Parser::default(),
            painter: Painter::new(width, height),
            now_ms: 1_000,
        };
        h.paint();
        h.check();
        h
    }

    /// Presses each key of a key script (`<d-a>`, `x`, `<s-left>`…), checking the screen
    /// after each.
    pub fn press(&mut self, script: &str) {
        let items = caretline::keymap::parse_keys(script).expect("key script");
        for item in items {
            match item {
                caretline::keymap::ScriptItem::Key(k) => self.press_key(k),
                caretline::keymap::ScriptItem::Wait(ms) => {
                    self.now_ms += ms;
                    self.apply(vec![Msg::Tick {
                        now_ms: self.now_ms,
                    }]);
                    self.paint();
                    self.check();
                }
            }
        }
    }

    /// Presses one key.
    pub fn press_key(&mut self, key: Key) {
        let name = caretline::commands::key_notation(&key);
        match self.keyboard.press(&key) {
            Sent::Bytes(b) => {
                self.log
                    .push(format!("{name}: sent {}", crate::doctor::escaped(&b)));
                self.feed(&b);
            }
            Sent::Paste => {
                let text = self.clipboard.clone().unwrap_or_default();
                self.log
                    .push(format!("{name}: the terminal pastes {text:?}"));
                let bytes = [b"\x1b[200~".as_slice(), text.as_bytes(), b"\x1b[201~"].concat();
                self.feed(&bytes);
            }
            Sent::Taken(t) => {
                self.log
                    .push(format!("{name}: kept by the terminal ({})", t.action));
            }
        }
        self.paint();
        self.check();
    }

    /// Bytes from the terminal, as the reader passes them on.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.push(bytes);
        while let Some(t) = self.parser.next(true) {
            let Token::Event(ev) = t else { continue };
            if let Event::Key(k) = &ev
                && !matches!(
                    k.kind,
                    crossterm::event::KeyEventKind::Press | crossterm::event::KeyEventKind::Repeat
                )
            {
                continue;
            }
            let clip = self.clipboard.clone();
            let msgs = crate::runtime::terminal_msgs(&self.state, ev, || clip);
            if msgs.is_empty() {
                continue;
            }
            self.now_ms += 10;
            let mut all = vec![Msg::Tick {
                now_ms: self.now_ms,
            }];
            all.extend(msgs);
            self.apply(all);
        }
    }

    fn apply(&mut self, msgs: Vec<Msg>) {
        for m in msgs {
            if let Some(last) = self.log.last_mut()
                && !matches!(m, Msg::Tick { .. })
            {
                last.push_str(&format!(" → {m:?}"));
            }
            for e in update(&mut self.state, m) {
                if let Effect::ClipboardSet { text } = e {
                    self.clipboard = Some(text);
                }
            }
        }
    }

    fn paint(&mut self) {
        let f = self.frame();
        let bytes = self.painter.paint(&f);
        self.screen.feed(&bytes);
    }

    /// The frame the editor draws now.
    pub fn frame(&self) -> Frame {
        view(&self.state)
    }

    /// The selection in notation.
    pub fn show(&self) -> String {
        show(&self.state)
    }

    /// The screen's text, row by row.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn screen_text(&self) -> String {
        self.screen.text()
    }

    /// The screen-truth oracle: the screen is the frame (every cell's text, colours and
    /// reverse, and the cursor), and it shows the state's selection: each caret but the primary
    /// a reverse cell, the primary the terminal's cursor, each selected char in the
    /// selection's colours, and no other char in them.
    pub fn check(&self) {
        let f = self.frame();
        let ctx = format!("after {:?}", self.log);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_screen_is_frame(&self.screen, &f)
        }));
        if let Err(e) = result {
            std::panic::resume_unwind(Box::new(format!("{ctx}: {}", panic_text(&e))));
        }
        let sel = &self.state.view.selection;
        let primary = sel.primary_index();
        let selected_bg = term_color(crate::runtime::style_in(&f, Role::Selection).bg);
        for (i, cell) in f.cells.iter().enumerate() {
            // A wide char's second cell draws nothing of its own.
            let Some(c) = cell
                .char_idx
                .filter(|_| !cell.symbol.is_empty())
                .map(|c| c as usize)
            else {
                continue;
            };
            let (x, y) = ((i % f.width as usize) as u16, (i / f.width as usize) as u16);
            let on = self.screen.cell(x, y);
            let caret = self.state.view.focused
                && sel
                    .iter()
                    .enumerate()
                    .any(|(k, r)| k != primary && r.is_empty() && r.head == c);
            let selected = sel.iter().any(|r| r.from() <= c && c < r.to());
            assert_eq!(on.reverse, caret, "{ctx}: char {c} at ({x},{y}) caret");
            if !caret {
                assert_eq!(
                    on.bg == selected_bg && selected_bg != crate::screen::Color::Default,
                    selected,
                    "{ctx}: char {c} at ({x},{y}) selected"
                );
            }
        }
        if self.state.view.focused {
            assert_eq!(self.screen.cursor(), f.cursor, "{ctx}: the cursor");
            assert!(
                self.screen.cursor().is_some(),
                "{ctx}: a focused editor shows its cursor"
            );
        }
    }
}

fn panic_text(e: &Box<dyn std::any::Any + Send>) -> String {
    e.downcast_ref::<String>()
        .cloned()
        .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

const SIM_HELP: &str = "\
caretline sim TEXT KEYS [--legacy] [--mine] [--config LINE]... [--size WxH]

Runs KEYS through a simulated Ghostty, the editor and a terminal emulator, in process,
and shows what each key did and the screen it left, checked against the editor's state.

  TEXT    the document, with \u{25ae} for each caret and \u{27e6}\u{2026}\u{27e7} around a selection
  KEYS    a key script: letters, and <d-a> (Cmd-A), <s-left>, <c-a-bs>, <cr>, <wait:500>...
  --legacy       without the kitty keyboard protocol
  --mine         your Ghostty's bindings (ghostty +list-keybinds), not 1.3.1's defaults
  --config LINE  a Ghostty config line on top, e.g. 'keybind = super+a=unbind'
  --size WxH     the terminal (default 40x8)

  caretline sim 'hello \u{25ae}world' '<d-a>'
  caretline sim 'hello \u{25ae}world' '<d-a>' --config 'keybind = super+a=unbind'
  caretline sim '\u{25ae}one\n\u{25ae}two' 'x<s-right>'
";

/// `caretline sim`.
pub fn main(args: &[String]) -> Result<(), String> {
    let mut pos = Vec::new();
    let (mut legacy, mut mine) = (false, false);
    let mut config = Vec::new();
    let (mut w, mut h) = (40u16, 8u16);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{SIM_HELP}");
                return Ok(());
            }
            "--legacy" => legacy = true,
            "--mine" => mine = true,
            "--config" => config.push(it.next().ok_or("--config needs a line")?.clone()),
            "--size" => {
                let s = it.next().ok_or("--size needs WxH")?;
                let (a, b) = s.split_once('x').ok_or("--size needs WxH")?;
                w = a.parse().map_err(|_| "bad --size")?;
                h = b.parse().map_err(|_| "bad --size")?;
            }
            _ => pos.push(a.replace("\\n", "\n")),
        }
    }
    let [text, keys] = &pos[..] else {
        return Err(format!("sim takes TEXT and KEYS\n\n{SIM_HELP}"));
    };
    if !text.contains('▮') {
        return Err("TEXT needs a caret: \u{25ae}".into());
    }
    let mut kb = if mine {
        let out = Command::new(crate::doctor::ghostty_bin())
            .arg("+list-keybinds")
            .output()
            .map_err(|e| format!("running ghostty +list-keybinds: {e}"))?;
        TermKeyboard::ghostty(&String::from_utf8_lossy(&out.stdout))
    } else {
        TermKeyboard::ghostty_default()
    };
    kb = kb.with_config(&config.join("\n"));
    if legacy {
        kb = kb.legacy();
    }
    let items = caretline::keymap::parse_keys(keys)?;
    let mut harness = Harness::new(text, kb, w, h);
    println!(
        "Ghostty ({}), {}, {w}x{h}",
        if mine {
            "your bindings"
        } else {
            "1.3.1 defaults"
        },
        if legacy {
            "legacy keys"
        } else {
            "kitty keyboard protocol"
        }
    );
    println!("  {}", harness.show().replace('\n', "\u{23ce}"));
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut failed = None;
    for item in items {
        let before = harness.log.len();
        let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match item {
            caretline::keymap::ScriptItem::Key(k) => harness.press_key(k),
            caretline::keymap::ScriptItem::Wait(ms) => harness.press(&format!("<wait:{ms}>")),
        }));
        for line in &harness.log[before..] {
            println!("{line}");
        }
        println!("  {}", harness.show().replace('\n', "\u{23ce}"));
        if let Err(e) = step {
            failed = Some(panic_text(&e));
            break;
        }
    }
    std::panic::set_hook(quiet);
    println!();
    let rule = "\u{2500}".repeat(w as usize);
    println!("\u{250c}{rule}\u{2510}");
    for y in 0..h {
        let mut row = String::new();
        for x in 0..w {
            let c = harness.screen.cell(x, y);
            match (c.wide_tail, c.symbol.is_empty()) {
                (true, _) => {}
                (false, true) => row.push(' '),
                (false, false) => row.push_str(&c.symbol),
            }
        }
        println!("\u{2502}{row}\u{2502}");
    }
    println!("\u{2514}{rule}\u{2518}");
    match failed {
        None => {
            println!("the screen shows the state after every key");
            Ok(())
        }
        Some(e) => Err(format!("the screen doesn't show the state: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ghostty() -> TermKeyboard {
        TermKeyboard::ghostty_default()
    }

    /// Ghostty with the keys `caretline doctor` hands back.
    fn ghostty_fixed() -> TermKeyboard {
        let fixes: Vec<String> = crate::doctor::conflicts(&ghostty())
            .into_iter()
            .map(|c| c.fix)
            .collect();
        ghostty().with_config(&fixes.join("\n"))
    }

    #[test]
    fn notation_round_trips() {
        for n in [
            "hello ▮world",
            "⟦hello▮⟧ world",
            "⟦▮hello⟧",
            "▮one\n▮two\n▮three",
            "ab▮",
        ] {
            let (text, sel) = parse(n);
            let s = State::from_parts(caretline::state::Document::new(&text, None), {
                let mut v = State::new(
                    &text,
                    None,
                    Viewport {
                        width: 20,
                        height: 5,
                    },
                )
                .view;
                v.selection = sel;
                v
            });
            assert_eq!(show(&s), n);
        }
    }

    /// Hand-found: default Ghostty keeps Cmd-A for `select_all`, which selected the whole
    /// terminal screen. The editor never sees it.
    #[test]
    fn cmd_a_on_default_ghostty_is_kept() {
        let mut h = Harness::new("hello ▮world", ghostty(), 30, 4);
        h.press("<d-a>");
        assert_eq!(h.show(), "hello ▮world");
        assert!(
            h.log[0].contains("kept by the terminal (select_all)"),
            "{:?}",
            h.log
        );
    }

    #[test]
    fn cmd_a_unbound_selects_all() {
        let mut h = Harness::new(
            "hello ▮world",
            ghostty().with_config("keybind = super+a=unbind"),
            30,
            4,
        );
        h.press("<d-a>");
        assert_eq!(h.show(), "⟦hello world▮⟧");
    }

    /// Hand-found: other carets were in the state and invisible. Typing goes to all three.
    #[test]
    fn three_carets_show_and_type() {
        let mut h = Harness::new("▮one\n▮two\n▮three", ghostty(), 20, 5);
        h.press("x");
        assert_eq!(h.show(), "x▮one\nx▮two\nx▮three");
        assert!(
            h.screen_text().starts_with("xone\nxtwo\nxthree"),
            "{}",
            h.screen_text()
        );
    }

    /// Ghostty rewrites Cmd-Left into Ctrl-A; it still goes to the line's start.
    #[test]
    fn cmd_left_is_rewritten_and_still_works() {
        let mut h = Harness::new("hello wor▮ld", ghostty(), 30, 4);
        h.press("<d-left>");
        assert_eq!(h.show(), "▮hello world");
        assert!(h.log[0].contains("sent \\x01"), "{:?}", h.log);
    }

    /// Copy goes to the clipboard; Cmd-V makes Ghostty paste it back.
    #[test]
    fn copy_then_paste_round_trips() {
        let mut h = Harness::new("⟦hello▮⟧ world", ghostty_fixed(), 30, 4);
        h.press("<d-c><end><d-v>");
        assert_eq!(h.clipboard.as_deref(), Some("hello"));
        assert_eq!(h.show(), "hello worldhello▮");
    }

    /// A multi-line selection: every selected cell, across the break, in the selection's
    /// colours (the oracle), and none past it.
    #[test]
    fn a_selection_over_lines() {
        let mut h = Harness::new("one t▮wo\nthree\nfour", ghostty_fixed(), 20, 5);
        // Down twice keeps column 5, clamped to "four"'s end; then one left.
        h.press("<s-down><s-down><s-left>");
        assert_eq!(h.show(), "one t⟦wo\nthree\nfou▮⟧r");
    }

    /// Without the keyboard protocol Shift-Enter arrives in xterm's `CSI 27;2;13~`.
    #[test]
    fn legacy_shift_enter_arrives() {
        let mut h = Harness::new("ab▮", ghostty().legacy(), 20, 4);
        h.press("<s-cr>x");
        assert_eq!(h.show(), "ab\nx▮");
    }

    /// Wide graphemes under carets: the caret cell is the grapheme's first cell.
    #[test]
    fn wide_graphemes_under_carets() {
        let mut h = Harness::new("▮漢字\n▮かな", ghostty(), 20, 4);
        h.press("<right>");
        assert_eq!(h.show(), "漢▮字\nか▮な");
    }

    /// The oracle isn't vacuous: a caret drawn as plain text (the hand-found bug) fails it.
    #[test]
    fn the_oracle_catches_an_invisible_caret() {
        let mut h = Harness::new("▮one\n▮two", ghostty(), 20, 4);
        // The primary is the last caret (line 2, the cursor); line 1's is a reverse cell.
        // Redraw that cell as a plain `o`, as before carets showed, and put the cursor back.
        assert!(h.screen.cell(0, 0).reverse);
        h.screen.feed(b"\x1b[0m\x1b[1;1Ho\x1b[2;1H");
        assert_eq!(h.screen.cursor(), Some((0, 1)));
        // Text and cursor still match: only the caret's look is wrong.
        assert_eq!(h.screen_text(), crate::screen::frame_text(&h.frame()));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.check()));
        assert!(caught.is_err(), "the oracle passed an invisible caret");
    }
}
