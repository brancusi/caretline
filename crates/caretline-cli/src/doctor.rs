//! `caretline doctor`: the keys the terminal takes before caretline sees them. A terminal's
//! own bindings run first: Ghostty's default Cmd-A selects the whole screen, Cmd-C copies the
//! terminal's selection, Cmd-Up jumps between prompts. The check compares the terminal's
//! bindings with caretline's keymap, so it never drifts from what the keys do.

use std::process::Command;

use caretline::commands::{binding_key, command_for, default_keymap, key_notation};
use caretline::keymap::{Key, KeyCode, Mods};

use crate::keys::label;

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
}

/// A caretline binding the terminal takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub keys: &'static str,
    pub command: &'static str,
    pub term: TermBinding,
    /// The config line that hands the key to caretline.
    pub fix: String,
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
    let mut spec = spec;
    // Prefixes: `performable:`, `global:`, `all:`, `unconsumed:`.
    while let Some((p, rest)) = spec.split_once(':') {
        if p.contains(['+', '=']) {
            break;
        }
        performable |= p == "performable";
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

/// The key a Ghostty `text:`/`esc:` action sends, when it is one key: `text:\x01` is Ctrl-A,
/// `esc:b` is Alt-B.
fn sent_key(action: &str) -> Option<Key> {
    if let Some(t) = action.strip_prefix("text:") {
        let t = t.replace("\\\\", "\\");
        let hex = t.strip_prefix("\\x")?;
        let b = u8::from_str_radix(hex, 16).ok()?;
        if (1..=26).contains(&b) {
            let mut mods = Mods::none();
            mods.ctrl = true;
            return Some(Key {
                code: KeyCode::Char((b'a' + b - 1) as char),
                mods,
            });
        }
        return None;
    }
    let c = action.strip_prefix("esc:")?;
    let mut chars = c.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            let mut mods = Mods::none();
            mods.alt = true;
            Some(Key {
                code: KeyCode::Char(c),
                mods,
            })
        }
        _ => None,
    }
}

/// Ghostty actions that leave a key to the program. Its defaults bind copy, selection
/// adjustment and search's end `performable` (they take the key only when there is a terminal
/// selection or a search), which `+list-keybinds` doesn't print; a paste arrives as a paste;
/// and Cmd-Q quitting the terminal is Ghostty's to keep.
const PASSES: &[&str] = &[
    "copy_to_clipboard",
    "adjust_selection",
    "end_search",
    "paste_from_clipboard",
    "quit",
];

/// The caretline bindings (plain and outline keymaps) the terminal's bindings take. A key the
/// terminal rewrites into another key that runs the same command is not one, nor is a key
/// whose action leaves it to the program ([`PASSES`]).
pub fn conflicts(term: &[TermBinding]) -> Vec<Conflict> {
    let mut out: Vec<Conflict> = Vec::new();
    for outline in [false, true] {
        for b in default_keymap(outline) {
            let Some(key) = binding_key(&b) else {
                continue;
            };
            let notation = key_notation(&key);
            let Some(t) = term.iter().find(|t| key_notation(&t.key) == notation) else {
                continue;
            };
            let action = t.action.split(':').next().unwrap_or_default();
            if t.performable || PASSES.contains(&action) || out.iter().any(|c| c.keys == b.keys) {
                continue;
            }
            if let Some(sent) = sent_key(&t.action)
                && command_for(outline, &sent) == Some(b.command)
            {
                continue;
            }
            out.push(Conflict {
                keys: b.keys,
                command: b.command,
                term: t.clone(),
                fix: format!("keybind = {}=unbind", t.trigger),
            });
        }
    }
    out
}

/// The report for a terminal's bindings.
pub fn report(term: &[TermBinding]) -> String {
    let found = conflicts(term);
    if found.is_empty() {
        return "Ghostty passes every caretline key through.\n".into();
    }
    let mut s = String::from("Ghostty takes these keys before caretline sees them:\n\n");
    for c in &found {
        s.push_str(&format!(
            "  {:<16} {:<30} Ghostty: {}\n",
            label(c.keys),
            c.command,
            c.term.action
        ));
    }
    s.push_str(
        "\nTo hand them to caretline, add these lines to Ghostty's config and reload it\n\
         (Cmd-Shift-,). Ghostty's bindings apply in every program, so an unbound key loses\n\
         Ghostty's action everywhere.\n\n",
    );
    for c in &found {
        s.push_str(&format!("  {}\n", c.fix));
    }
    s
}

/// Ghostty's binary: beside the running terminal (`GHOSTTY_BIN_DIR`), else on the path, else
/// the macOS app.
fn ghostty_bin() -> String {
    if let Some(dir) = std::env::var_os("GHOSTTY_BIN_DIR") {
        let p = std::path::Path::new(&dir).join("ghostty");
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }
    let app = "/Applications/Ghostty.app/Contents/MacOS/ghostty";
    if cfg!(target_os = "macos") && std::path::Path::new(app).exists() {
        return app.into();
    }
    "ghostty".into()
}

/// `caretline doctor`.
pub fn main(args: &[String]) -> Result<(), String> {
    if let Some(a) = args.first() {
        return Err(format!("doctor takes no arguments (got {a})"));
    }
    let ghostty = std::env::var("TERM_PROGRAM").is_ok_and(|t| t == "ghostty")
        || std::env::var_os("GHOSTTY_RESOURCES_DIR").is_some();
    if !ghostty {
        let term = std::env::var("TERM_PROGRAM").unwrap_or_else(|_| "this terminal".into());
        println!("caretline doctor checks Ghostty's key bindings; {term} isn't Ghostty.");
        return Ok(());
    }
    let out = Command::new(ghostty_bin())
        .arg("+list-keybinds")
        .output()
        .map_err(|e| format!("running ghostty +list-keybinds: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ghostty +list-keybinds failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    print!(
        "{}",
        report(&parse_ghostty(&String::from_utf8_lossy(&out.stdout)))
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slice of Ghostty's default bindings (`ghostty +list-keybinds --default`).
    const DEFAULTS: &str = r"keybind = super+c=copy_to_clipboard:mixed
keybind = super+v=paste_from_clipboard
keybind = super+a=select_all
keybind = super+z=undo
keybind = super+shift+z=redo
keybind = super+ctrl+==equalize_splits
keybind = super+arrow_up=jump_to_prompt:-1
keybind = super+arrow_right=text:\\x05
keybind = super+arrow_left=text:\\x01
keybind = super+backspace=text:\\x15
keybind = alt+arrow_left=esc:b
keybind = ctrl+a>n=new_window
";

    #[test]
    fn parses_triggers_actions_and_prefixes() {
        let b = parse_ghostty(DEFAULTS);
        assert_eq!(b[0].trigger, "super+c");
        assert_eq!(b[0].action, "copy_to_clipboard:mixed");
        assert_eq!(key_notation(&b[0].key), "<d-c>");
        assert_eq!(key_notation(&b[5].key), "<c-d-=>");
        assert_eq!(b[5].action, "equalize_splits");
        assert!(
            b.iter().all(|t| !t.trigger.contains('>')),
            "sequences skipped"
        );
        let p = parse_ghostty("keybind = performable:super+c=copy_to_clipboard:mixed");
        assert!(p[0].performable);
        assert_eq!(p[0].trigger, "super+c");
    }

    #[test]
    fn ghostty_defaults_take_select_all_undo_and_doc_start() {
        let found = conflicts(&parse_ghostty(DEFAULTS));
        let keys: Vec<&str> = found.iter().map(|c| c.keys).collect();
        // Cmd-C copies only a terminal selection and Cmd-V is a paste either way; Cmd-Left and
        // Cmd-Right become Ctrl-A and Ctrl-E, Cmd-Backspace Ctrl-U and Alt-Left Esc b, which
        // run the same commands.
        assert_eq!(keys, ["<d-up>", "<d-a>", "<d-z>", "<d-s-z>"]);
        let a = found.iter().find(|c| c.keys == "<d-a>").unwrap();
        assert_eq!(a.fix, "keybind = super+a=unbind");
    }

    #[test]
    fn a_performable_binding_passes() {
        let b = parse_ghostty("keybind = performable:super+a=select_all\n");
        assert!(conflicts(&b).is_empty());
    }
}
