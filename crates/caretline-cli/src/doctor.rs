//! `caretline doctor`: the keys the terminal takes before caretline sees them. A terminal's
//! own bindings run first: Ghostty's default Cmd-A selects the whole screen, Cmd-C copies the
//! terminal's selection, Cmd-Up jumps between prompts. The check compares the terminal's
//! bindings with caretline's keymap, so it never drifts from what the keys do.
//!
//! `caretline doctor --keys` shows each key live: the bytes the terminal sent, the key
//! caretline decoded, and the command it runs.

use std::process::Command;

use caretline::commands::{binding_key, command_for, default_keymap, key_notation};
use caretline::keymap::KeyCode;

use crate::keyboard::{Sent, TermBinding, TermKeyboard, decode};

use crate::keys::label;

/// A caretline binding the terminal takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub keys: &'static str,
    pub command: &'static str,
    pub term: TermBinding,
    /// The config line that hands the key to caretline.
    pub fix: String,
}

/// The caretline bindings (plain and outline keymaps) the terminal's bindings take: the
/// terminal keeps the key, or rewrites it into bytes that run another command. A key the
/// terminal rewrites into another key for the same command is not one (Cmd-Left is sent as
/// Ctrl-A, which also goes to the line's start).
pub fn conflicts(kb: &TermKeyboard) -> Vec<Conflict> {
    let mut out: Vec<Conflict> = Vec::new();
    for outline in [false, true] {
        for b in default_keymap(outline) {
            let Some(key) = binding_key(&b) else {
                continue;
            };
            if out.iter().any(|c| c.keys == b.keys) {
                continue;
            }
            let term = match kb.press(&key) {
                // Cmd-Q quitting the terminal is the terminal's to keep.
                Sent::Taken(t) if t.action == "quit" => continue,
                Sent::Taken(t) => t,
                Sent::Paste => continue,
                Sent::Bytes(bytes) => match kb.binding(&key) {
                    Some(t)
                        if !matches!(decode(&bytes).as_slice(),
                            [k] if command_for(outline, k) == Some(b.command)) =>
                    {
                        t.clone()
                    }
                    _ => continue,
                },
            };
            out.push(Conflict {
                keys: b.keys,
                command: b.command,
                fix: format!("keybind = {}=unbind", term.trigger),
                term,
            });
        }
    }
    out
}

/// The report for a terminal's bindings.
pub fn report(kb: &TermKeyboard) -> String {
    let found = conflicts(kb);
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
pub(crate) fn ghostty_bin() -> String {
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

/// The bytes as text: printable ASCII as is, ESC as `⎋`, other bytes as `\xNN`.
pub fn escaped(bytes: &[u8]) -> String {
    let mut s = String::new();
    for &b in bytes {
        match b {
            0x1b => s.push('⎋'),
            0x20..=0x7e => s.push(b as char),
            _ => s.push_str(&format!("\\x{b:02x}")),
        }
    }
    s
}

/// One decoded token, as a line of `doctor --keys`.
fn key_line(token: &crate::rawin::Token) -> String {
    use crate::rawin::Token;
    use crossterm::event::{Event, KeyEventKind};
    match token {
        Token::Event(Event::Key(k)) => {
            let kind = match k.kind {
                KeyEventKind::Press => "",
                KeyEventKind::Repeat => " (repeat)",
                KeyEventKind::Release => " (release)",
            };
            match crate::runtime::to_key(k) {
                Some(key) => {
                    let notation = key_notation(&key);
                    let does = command_for(false, &key).unwrap_or(match key.code {
                        KeyCode::Char(_) if !(key.mods.ctrl || key.mods.alt || key.mods.cmd) => {
                            "types itself"
                        }
                        _ => "nothing bound",
                    });
                    format!("{:<16} {:<12} {does}{kind}", label(&notation), notation)
                }
                None => format!("{:?}{kind}: no key caretline reads", k.code),
            }
        }
        Token::Event(Event::Paste(t)) => format!("a paste of {} chars", t.chars().count()),
        Token::Event(e) => format!("{e:?}"),
        Token::Reply(r) => format!("a terminal reply: {r:?}"),
        Token::Keyboard(f) => format!("the keyboard protocol's flags: {f}"),
    }
}

/// `caretline doctor --keys`: press keys, see what arrives. Ctrl-C twice stops.
fn keys() -> Result<(), String> {
    use crate::rawin::{self, Token};
    use crossterm::event::{Event, KeyCode as C, KeyModifiers};
    use std::io::Write;
    use std::time::Duration;

    crossterm::terminal::enable_raw_mode().map_err(|e| format!("terminal: {e}"))?;
    let mut parser = rawin::Parser::default();
    let (_, kitty) = crate::runtime::probe_terminal(&mut parser, false);
    let mut out = std::io::stdout();
    if kitty {
        let _ = write!(out, "\x1b[>1u");
    }
    let mut say = |line: &str| {
        let _ = write!(out, "{line}\r\n");
        let _ = out.flush();
    };
    say(&format!(
        "Press keys to see what caretline reads (keyboard protocol: {}). Ctrl-C twice stops.",
        if kitty { "kitty" } else { "legacy" }
    ));
    say(
        "A key that prints nothing never reached caretline: the terminal took it (see `caretline doctor`).",
    );
    say("");
    let mut ctrl_c = 0;
    while ctrl_c < 2 {
        let got = rawin::fill(&mut parser, Duration::from_millis(30));
        if !got && !parser.pending() {
            continue;
        }
        let bytes = escaped(parser.buffered());
        let mut first = true;
        while let Some(t) = parser.next(!got) {
            let quit = matches!(&t, Token::Event(Event::Key(k))
                if k.code == C::Char('c') && k.modifiers == KeyModifiers::CONTROL);
            ctrl_c = if quit { ctrl_c + 1 } else { 0 };
            let shown = if first { bytes.as_str() } else { "" };
            first = false;
            say(&format!("  {shown:<18} {}", key_line(&t)));
        }
    }
    if kitty {
        let _ = write!(std::io::stdout(), "\x1b[<u");
    }
    crossterm::terminal::disable_raw_mode().map_err(|e| format!("terminal: {e}"))?;
    Ok(())
}

/// `caretline doctor`.
pub fn main(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None => {}
        Some("--keys") => return keys(),
        Some(a) => return Err(format!("doctor takes only --keys (got {a})")),
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
        report(&TermKeyboard::ghostty(&String::from_utf8_lossy(
            &out.stdout
        )))
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
        let b = crate::keyboard::parse_ghostty(DEFAULTS);
        assert_eq!(b[0].trigger, "super+c");
        assert_eq!(b[0].action, "copy_to_clipboard:mixed");
        assert_eq!(key_notation(&b[0].key), "<d-c>");
        assert_eq!(key_notation(&b[5].key), "<c-d-=>");
        assert_eq!(b[5].action, "equalize_splits");
        assert!(
            b.iter().all(|t| !t.trigger.contains('>')),
            "sequences skipped"
        );
        let p =
            crate::keyboard::parse_ghostty("keybind = performable:super+c=copy_to_clipboard:mixed");
        assert!(p[0].performable);
        assert_eq!(p[0].trigger, "super+c");
    }

    #[test]
    fn ghostty_defaults_take_select_all_undo_and_doc_start() {
        let found = conflicts(&TermKeyboard::ghostty(DEFAULTS));
        let keys: Vec<&str> = found.iter().map(|c| c.keys).collect();
        // Cmd-C copies only a terminal selection and Cmd-V is a paste either way; Cmd-Left and
        // Cmd-Right become Ctrl-A and Ctrl-E, Cmd-Backspace Ctrl-U and Alt-Left Esc b, which
        // run the same commands.
        assert_eq!(keys, ["<d-up>", "<d-a>", "<d-z>", "<d-s-z>"]);
        let a = found.iter().find(|c| c.keys == "<d-a>").unwrap();
        assert_eq!(a.fix, "keybind = super+a=unbind");
    }

    #[test]
    fn key_lines_show_bytes_key_and_command() {
        let mut p = crate::rawin::Parser::default();
        p.push(b"\x1b[97;9u");
        assert_eq!(escaped(p.buffered()), "⎋[97;9u");
        let line = key_line(&p.next(true).unwrap());
        assert!(line.starts_with("Cmd-A"), "{line}");
        assert!(
            line.contains("<d-a>") && line.ends_with("select.all"),
            "{line}"
        );
        p.push(b"x\x01");
        assert!(key_line(&p.next(true).unwrap()).ends_with("types itself"));
        assert!(key_line(&p.next(true).unwrap()).contains("<c-a>"));
    }

    #[test]
    fn a_performable_binding_passes() {
        let b = TermKeyboard::ghostty("keybind = performable:super+a=select_all\n");
        assert!(conflicts(&b).is_empty());
    }
}
