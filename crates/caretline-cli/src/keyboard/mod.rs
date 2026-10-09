//! A simulated terminal keyboard: what the terminal does with a key press. It either keeps
//! the key for one of its own bindings, or sends bytes: a binding's `text:`/`esc:`/`csi:`
//! bytes, or the key encoded the way the terminal encodes it (the kitty keyboard protocol's
//! disambiguate flag, which caretline pushes, or the legacy xterm forms without it).
//!
//! The model is Ghostty's, checked against a table of what Ghostty 1.3.1 really sent
//! (`ghostty-1.3.1-measured.txt`); its default bindings are `ghostty-1.3.1-defaults.txt`.
//! `caretline doctor` and the key tests share it: a key is reachable when its bytes, decoded
//! by `rawin::Parser`, run the key's command.

use caretline::keymap::{Key, KeyCode, Mods};

/// `ghostty +list-keybinds --default` for Ghostty 1.3.1.
#[cfg_attr(not(test), allow(dead_code))]
pub const GHOSTTY_DEFAULTS: &str = include_str!("ghostty-1.3.1-defaults.txt");

/// One of the terminal's bindings: the trigger as the terminal spells it, the key, and what
/// the terminal does with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermBinding {
    pub trigger: String,
    pub key: Key,
    pub action: String,
    /// Ghostty's `performable:`: it only takes the key when the action can run (a copy with
    /// nothing selected passes the key through).
    pub performable: bool,
    /// Ghostty's `unconsumed:`: it runs the action and still sends the key.
    pub unconsumed: bool,
}

/// What the terminal does with a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    /// The bytes the program reads.
    Bytes(Vec<u8>),
    /// The terminal pastes its clipboard: the program reads a bracketed paste.
    Paste,
    /// The terminal keeps the key for this binding of its own.
    Taken(TermBinding),
}

/// A terminal's keyboard: its bindings (later ones win, as in a config file) and whether the
/// program pushed the kitty keyboard protocol.
#[derive(Debug, Clone)]
pub struct TermKeyboard {
    pub bindings: Vec<TermBinding>,
    pub kitty: bool,
}

/// Ghostty actions that leave a key to the program. Its defaults bind copy, selection
/// adjustment and search's end `performable` (they take the key only when there is a terminal
/// selection or a search, and the model has neither), which `+list-keybinds` doesn't print.
const PASSES: &[&str] = &[
    "copy_to_clipboard",
    "adjust_selection",
    "end_search",
    "unbind",
];

// The constructors the key tests use, and the harness will.
#[cfg_attr(not(test), allow(dead_code))]
impl TermKeyboard {
    /// Ghostty 1.3.1 with its default bindings, the keyboard protocol pushed.
    pub fn ghostty_default() -> TermKeyboard {
        TermKeyboard::ghostty(GHOSTTY_DEFAULTS)
    }

    /// Ghostty with these bindings (`+list-keybinds` output), the keyboard protocol pushed.
    pub fn ghostty(listing: &str) -> TermKeyboard {
        TermKeyboard {
            bindings: parse_ghostty(listing),
            kitty: true,
        }
    }

    /// No bindings: every key is encoded.
    pub fn bare() -> TermKeyboard {
        TermKeyboard {
            bindings: Vec::new(),
            kitty: true,
        }
    }

    /// Config lines on top (`keybind = super+a=unbind`): later bindings win.
    pub fn with_config(mut self, config: &str) -> TermKeyboard {
        self.bindings.extend(parse_ghostty(config));
        self
    }

    /// Without the keyboard protocol: the legacy forms.
    pub fn legacy(mut self) -> TermKeyboard {
        self.kitty = false;
        self
    }

    /// The binding the terminal runs for `key`, if any (the last one for the key).
    pub fn binding(&self, key: &Key) -> Option<&TermBinding> {
        let want = normal(key);
        self.bindings.iter().rev().find(|b| normal(&b.key) == want)
    }

    /// Presses `key`.
    pub fn press(&self, key: &Key) -> Sent {
        if let Some(b) = self.binding(key) {
            let action = b.action.split(':').next().unwrap_or_default();
            if let Some(bytes) = action_bytes(&b.action) {
                return Sent::Bytes(bytes);
            }
            if action == "paste_from_clipboard" {
                return Sent::Paste;
            }
            if !(b.performable || b.unconsumed || PASSES.contains(&action)) {
                return Sent::Taken(b.clone());
            }
        }
        Sent::Bytes(encode(key, self.kitty))
    }
}

/// The editor's keys in these bytes, as the runtime decodes them.
pub fn decode(bytes: &[u8]) -> Vec<Key> {
    let mut p = crate::rawin::Parser::default();
    p.push(bytes);
    let mut keys = Vec::new();
    while let Some(t) = p.next(true) {
        if let crate::rawin::Token::Event(crossterm::event::Event::Key(k)) = t
            && let Some(key) = crate::runtime::to_key(&k)
        {
            keys.push(key);
        }
    }
    keys
}

/// A key with Shift-Tab as Tab plus Shift, and letters lowercase plus Shift.
fn normal(key: &Key) -> Key {
    let mut k = *key;
    match k.code {
        KeyCode::BackTab => {
            k.code = KeyCode::Tab;
            k.mods.shift = true;
        }
        KeyCode::Char(c) if c.is_ascii_uppercase() => {
            k.code = KeyCode::Char(c.to_ascii_lowercase());
            k.mods.shift = true;
        }
        _ => {}
    }
    k
}

/// The bytes of a `text:`, `esc:` or `csi:` action (Zig string escapes: `\\xNN`, `\\n`, …).
fn action_bytes(action: &str) -> Option<Vec<u8>> {
    let (kind, arg) = action.split_once(':')?;
    let body = unescape(arg);
    match kind {
        "text" => Some(body),
        "esc" => Some([b"\x1b".as_slice(), &body].concat()),
        "csi" => Some([b"\x1b[".as_slice(), &body].concat()),
        _ => None,
    }
}

fn unescape(s: &str) -> Vec<u8> {
    // `+list-keybinds` prints `text:\\x01` for the config's `text:\x01`.
    let s = &s.replace("\\\\", "\\");
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            match b[i + 1] {
                b'x' if i + 4 <= b.len() => {
                    let hex = std::str::from_utf8(&b[i + 2..i + 4]).unwrap_or("");
                    if let Ok(v) = u8::from_str_radix(hex, 16) {
                        out.push(v);
                        i += 4;
                        continue;
                    }
                }
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                    continue;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                    continue;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                    continue;
                }
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// xterm's modifier parameter: 1 + shift 1, alt 2, ctrl 4, super 8.
fn param(m: Mods) -> u32 {
    1 + m.shift as u32 + 2 * m.alt as u32 + 4 * m.ctrl as u32 + 8 * m.cmd as u32
}

fn none(m: Mods) -> bool {
    !(m.shift || m.alt || m.ctrl || m.cmd)
}

/// The key as Ghostty encodes it, with the keyboard protocol's disambiguate flag (`kitty`) or
/// in the legacy forms.
pub fn encode(key: &Key, kitty: bool) -> Vec<u8> {
    let key = normal(key);
    let m = key.mods;
    let p = param(m);
    let csi = |s: String| format!("\x1b[{s}").into_bytes();
    // Arrows, Home and End: `CSI X`, or `CSI 1;m X`; Delete and the pages: `CSI n~`, `CSI n;m~`.
    let cursor = |fin: char| {
        if none(m) {
            csi(fin.to_string())
        } else {
            csi(format!("1;{p}{fin}"))
        }
    };
    let tilde = |n: u32| {
        if none(m) {
            csi(format!("{n}~"))
        } else {
            csi(format!("{n};{p}~"))
        }
    };
    // A key with no legacy form for its modifiers: kitty's `CSI cp;m u`, or xterm's
    // modifyOtherKeys `CSI 27;m;cp~`.
    let other = |cp: u32| {
        if kitty {
            csi(format!("{cp};{p}u"))
        } else {
            csi(format!("27;{p};{cp}~"))
        }
    };
    let alt = |b: &[u8]| {
        if m.alt {
            [b"\x1b".as_slice(), b].concat()
        } else {
            b.to_vec()
        }
    };
    match key.code {
        KeyCode::Left => cursor('D'),
        KeyCode::Right => cursor('C'),
        KeyCode::Up => cursor('A'),
        KeyCode::Down => cursor('B'),
        KeyCode::Home => cursor('H'),
        KeyCode::End => cursor('F'),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc | KeyCode::Backspace => {
            let (cp, plain): (u32, &[u8]) = match key.code {
                KeyCode::Enter => (13, b"\r"),
                KeyCode::Esc => (27, b"\x1b"),
                KeyCode::Backspace => (127, b"\x7f"),
                _ => (9, b"\t"),
            };
            if kitty {
                // Esc is always `CSI 27u`; the others are legacy bytes until modified.
                match (none(m), cp) {
                    (true, 27) => csi("27u".into()),
                    (true, _) => plain.to_vec(),
                    _ => other(cp),
                }
            } else if cp == 127 {
                // Backspace: Ctrl makes it ^H, Alt adds ESC, Shift and Cmd are dropped.
                alt(if m.ctrl { b"\x08" } else { b"\x7f" })
            } else if none(m) {
                plain.to_vec()
            } else if cp == 9 && m.shift && !(m.alt || m.ctrl || m.cmd) {
                csi("Z".into())
            } else if m.alt && !(m.shift || m.ctrl || m.cmd) {
                alt(plain)
            } else {
                other(cp)
            }
        }
        KeyCode::Char(c) => {
            if !(m.ctrl || m.alt || m.cmd) {
                let c = if m.shift {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c
                };
                return c.to_string().into_bytes();
            }
            if kitty {
                return other(c as u32);
            }
            // Legacy: Ctrl folds a letter (and space, `[\]^_`) into a control byte, Alt adds
            // ESC; anything else (Shift or Cmd with them) is modifyOtherKeys.
            if m.shift || m.cmd {
                return other(c as u32);
            }
            let ctrl = |c: char| match c {
                'a'..='z' => Some(c as u8 - b'a' + 1),
                ' ' | '@' => Some(0),
                '[' => Some(0x1b),
                '\\' => Some(0x1c),
                ']' => Some(0x1d),
                '^' => Some(0x1e),
                '_' => Some(0x1f),
                _ => None,
            };
            match (m.ctrl, ctrl(c)) {
                (true, Some(b)) => alt(&[b]),
                (true, None) => other(c as u32),
                (false, _) => alt(c.to_string().as_bytes()),
            }
        }
    }
}

/// Parses `ghostty +list-keybinds` output: `keybind = super+a=select_all` lines. Sequences
/// (`ctrl+a>n`) and keys caretline has no name for are skipped.
pub fn parse_ghostty(listing: &str) -> Vec<TermBinding> {
    listing
        .lines()
        .filter_map(|line| {
            let spec = line.trim().strip_prefix("keybind")?.trim_start();
            let spec = spec.strip_prefix('=')?.trim();
            ghostty_binding(spec)
        })
        .collect()
}

fn ghostty_binding(spec: &str) -> Option<TermBinding> {
    let mut performable = false;
    let mut unconsumed = false;
    let mut spec = spec;
    // Prefixes: `performable:`, `global:`, `all:`, `unconsumed:`.
    while let Some((p, rest)) = spec.split_once(':') {
        if p.contains(['+', '=']) {
            break;
        }
        performable |= p == "performable";
        unconsumed |= p == "unconsumed";
        spec = rest;
    }
    // The trigger ends at the `=` after the last `+` (the key itself may be `=`).
    let last_plus = spec.rfind('+').map_or(0, |i| i + 1);
    let eq = last_plus + 1 + spec[last_plus + 1..].find('=')?;
    let (trigger, action) = (&spec[..eq], &spec[eq + 1..]);
    if trigger.contains('>') {
        return None;
    }
    let mut mods = Mods::none();
    let parts: Vec<&str> = trigger.split('+').collect();
    let (name, mod_names) = parts.split_last()?;
    for m in mod_names {
        match *m {
            "super" | "cmd" | "command" => mods.cmd = true,
            "ctrl" | "control" => mods.ctrl = true,
            "alt" | "opt" | "option" => mods.alt = true,
            "shift" => mods.shift = true,
            _ => return None,
        }
    }
    let code = ghostty_key(if name.is_empty() { "+" } else { name })?;
    Some(TermBinding {
        trigger: trigger.to_string(),
        key: Key { code, mods },
        action: action.to_string(),
        performable,
        unconsumed,
    })
}

fn ghostty_key(name: &str) -> Option<KeyCode> {
    Some(match name {
        "arrow_left" | "left" => KeyCode::Left,
        "arrow_right" | "right" => KeyCode::Right,
        "arrow_up" | "up" => KeyCode::Up,
        "arrow_down" | "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "page_up" => KeyCode::PageUp,
        "page_down" => KeyCode::PageDown,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "enter" | "return" => KeyCode::Enter,
        "tab" => KeyCode::Tab,
        "escape" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "comma" => KeyCode::Char(','),
        "period" => KeyCode::Char('.'),
        "slash" => KeyCode::Char('/'),
        "backslash" => KeyCode::Char('\\'),
        "semicolon" => KeyCode::Char(';'),
        "quote" => KeyCode::Char('\''),
        "backquote" => KeyCode::Char('`'),
        "minus" => KeyCode::Char('-'),
        "equal" => KeyCode::Char('='),
        "bracket_left" => KeyCode::Char('['),
        "bracket_right" => KeyCode::Char(']'),
        n => {
            let n = n.strip_prefix("digit_").unwrap_or(n);
            let mut chars = n.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c.to_ascii_lowercase()),
                _ => return None,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use caretline::commands::{binding_key, command_for, default_keymap, key_notation};
    use caretline::keymap::{ScriptItem, parse_keys};

    fn key(notation: &str) -> Key {
        match parse_keys(notation).unwrap().as_slice() {
            [ScriptItem::Key(k)] => *k,
            other => panic!("{notation}: {other:?}"),
        }
    }

    fn names(keys: &[Key]) -> Vec<String> {
        keys.iter().map(key_notation).collect()
    }

    /// The model sends what Ghostty 1.3.1 sent, row for row.
    #[test]
    fn matches_what_ghostty_sent() {
        let measured = include_str!("ghostty-1.3.1-measured.txt");
        let mut rows = 0;
        for line in measured.lines().filter(|l| !l.starts_with('#')) {
            let f: Vec<&str> = line.split_whitespace().collect();
            let [mode, notation, want] = f[..] else {
                panic!("{line}")
            };
            let mut kb = TermKeyboard::ghostty_default().with_config("keybind = super+a=unbind");
            if mode == "legacy" {
                kb = kb.legacy();
            }
            let got = match kb.press(&key(notation)) {
                Sent::Taken(_) | Sent::Paste => None,
                Sent::Bytes(b) => Some(b),
            };
            let want = (want != "taken").then(|| unescape(want));
            assert_eq!(got, want, "{mode} {notation}");
            rows += 1;
        }
        assert_eq!(rows, 180);
    }

    /// Every key: the keys of the keymap, and each letter, digit, punctuation and named key
    /// under every modifier. Shift alone on a char that isn't a letter is no key a terminal
    /// sends (Shift-0 types `)`), so it's left out.
    fn all_keys() -> Vec<Key> {
        let mut keys: Vec<Key> = [false, true]
            .into_iter()
            .flat_map(default_keymap)
            .filter_map(|b| binding_key(&b))
            .collect();
        let bases = "a z 0 9 , . / ; [ ] - = ` space cr tab bs del esc left right up down home end pgup pgdn";
        for base in bases.split(' ') {
            for mods in [
                "", "s-", "c-", "a-", "d-", "c-s-", "a-s-", "d-s-", "c-a-", "c-d-",
            ] {
                let k = key(&format!("<{mods}{base}>"));
                let letter = matches!(k.code, KeyCode::Char(c) if c.is_ascii_alphabetic());
                if mods == "s-" && matches!(k.code, KeyCode::Char(_)) && !letter {
                    continue;
                }
                keys.push(k);
            }
        }
        keys
    }

    /// With the keyboard protocol, every key decodes back to itself.
    #[test]
    fn kitty_round_trips_every_key() {
        let kb = TermKeyboard::bare();
        for k in all_keys() {
            let Sent::Bytes(b) = kb.press(&k) else {
                unreachable!()
            };
            assert_eq!(
                names(&decode(&b)),
                [key_notation(&k)],
                "{} sent {}",
                key_notation(&k),
                crate::doctor::escaped(&b)
            );
        }
    }

    /// What legacy bytes can't carry: Backspace keeps only Ctrl (as ^H) and Alt, a double ESC
    /// is two Escs, and Ctrl with `[` or `]` is a control byte of its own.
    const LEGACY_LOSSES: &[&str] = &[
        "<a-esc> as <esc> <esc>",
        "<a-s-bs> as <a-bs>",
        "<c-[> as <esc>",
        "<c-]> as <c-5>",
        "<c-a-[> as <esc> <esc>",
        "<c-a-]> as <c-a-5>",
        "<c-a-bs> as <c-a-h>",
        "<c-bs> as <c-h>",
        "<c-d-bs> as <c-h>",
        "<c-s-bs> as <c-h>",
        "<d-bs> as <bs>",
        "<d-s-bs> as <bs>",
        "<s-bs> as <bs>",
    ];

    /// Without it, every key decodes back to itself except the ones legacy bytes can't tell
    /// apart, which come back as the key listed.
    #[test]
    fn legacy_round_trips_or_is_a_known_loss() {
        let kb = TermKeyboard::bare().legacy();
        let mut lossy = Vec::new();
        for k in all_keys() {
            let Sent::Bytes(b) = kb.press(&k) else {
                unreachable!()
            };
            let got = names(&decode(&b));
            if got != [key_notation(&k)] {
                lossy.push(format!("{} as {}", key_notation(&k), got.join(" ")));
            }
        }
        lossy.sort();
        lossy.dedup();
        assert_eq!(lossy, LEGACY_LOSSES, "legacy losses");
    }

    /// The keys Ghostty's defaults keep from caretline, reviewed: `caretline doctor` lists
    /// them with the line that hands each back.
    const KEPT_BY_DEFAULT: &[&str] = &[
        "<d-a> select_all",
        "<d-down> jump_to_prompt:1",
        "<d-end> scroll_to_bottom",
        "<d-home> scroll_to_top",
        "<d-q> quit",
        "<d-s-down> jump_to_prompt:1",
        "<d-s-up> jump_to_prompt:-1",
        "<d-s-z> redo",
        "<d-up> jump_to_prompt:-1",
        "<d-z> undo",
    ];

    /// Every binding of both keymaps, pressed on default Ghostty, either runs its command or
    /// is one Ghostty keeps ([`KEPT_BY_DEFAULT`]); with each kept key unbound, all run.
    #[test]
    fn every_binding_reaches_its_command() {
        for (kb, kept) in [
            (TermKeyboard::ghostty_default(), KEPT_BY_DEFAULT),
            (
                TermKeyboard::ghostty_default().with_config(
                    &crate::doctor::conflicts(&TermKeyboard::ghostty_default())
                        .iter()
                        .map(|c| c.fix.clone())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                &["<d-q> quit"][..],
            ),
        ] {
            let mut taken = Vec::new();
            for outline in [false, true] {
                for b in default_keymap(outline) {
                    let Some(k) = binding_key(&b) else { continue };
                    match kb.press(&k) {
                        Sent::Paste if b.command == "clip.paste" => {}
                        Sent::Paste => taken.push(format!("{} pasted", b.keys)),
                        Sent::Taken(t) => taken.push(format!("{} {}", b.keys, t.action)),
                        Sent::Bytes(bytes) => {
                            let got = decode(&bytes);
                            let runs = match got.as_slice() {
                                [g] => command_for(outline, g),
                                _ => None,
                            };
                            if runs != Some(b.command) {
                                taken.push(format!(
                                    "{} sent {} running {runs:?}",
                                    b.keys,
                                    crate::doctor::escaped(&bytes)
                                ));
                            }
                        }
                    }
                }
            }
            taken.sort();
            taken.dedup();
            assert_eq!(taken, kept, "keys that don't reach their command");
        }
    }
}
