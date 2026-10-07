//! Overlay text: widths (the engine's rule), the `{{key:…}}` and `{{code:…}}` markup, and
//! word wrap that never splits a grapheme or a key badge.

use std::collections::{BTreeMap, HashMap};

use unicode_segmentation::UnicodeSegmentation;

use crate::Glyphs;

/// A grapheme's display width, as caretline measures it: ASCII is 1, emoji sequences 2,
/// anything else its Unicode width and at least 1.
pub fn width(g: &str) -> usize {
    if g.is_empty() {
        return 0;
    }
    if g.is_ascii() {
        return 1;
    }
    let emoji_seq = g
        .chars()
        .any(|c| c == '\u{200D}' || c == '\u{FE0F}' || ('\u{1F3FB}'..='\u{1F3FF}').contains(&c))
        || (g.chars().count() == 2 && g.chars().all(|c| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)));
    if emoji_seq {
        2
    } else {
        unicode_width::UnicodeWidthStr::width(g).max(1)
    }
}

/// The width of a string, grapheme by grapheme.
pub fn str_width(s: &str) -> usize {
    s.graphemes(true).map(|g| width(printable(g))).sum()
}

/// Control and zero-width clusters would drive the terminal or take no cell: they draw as
/// U+FFFD, as in the editor.
pub fn printable(g: &str) -> &str {
    if g.chars().any(|c| c.is_control()) || unicode_width::UnicodeWidthStr::width(g) == 0 {
        "\u{FFFD}"
    } else {
        g
    }
}

/// The label of the keys bound to a command id (`⌃O`, `F2`), or `None` if it has none. The
/// host supplies it, so remapped keys show as bound.
pub trait KeyLabels {
    fn label(&self, command: &str) -> Option<String>;
}

impl<F: Fn(&str) -> Option<String>> KeyLabels for F {
    fn label(&self, command: &str) -> Option<String> {
        self(command)
    }
}

impl KeyLabels for BTreeMap<String, String> {
    fn label(&self, command: &str) -> Option<String> {
        self.get(command).cloned()
    }
}

impl KeyLabels for HashMap<String, String> {
    fn label(&self, command: &str) -> Option<String> {
        self.get(command).cloned()
    }
}

/// No key labels: a badge shows the command id.
pub struct NoKeys;

impl KeyLabels for NoKeys {
    fn label(&self, _: &str) -> Option<String> {
        None
    }
}

/// One grapheme to draw, with the base role it's drawn in (`callout`, `key`, `code`…).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Piece {
    pub g: String,
    pub w: u16,
    pub role: &'static str,
}

/// A unit of text for wrapping.
#[derive(Debug, Clone)]
pub(crate) enum Tok {
    /// Graphemes that stay together unless the word is wider than a line.
    Word(Vec<Piece>),
    /// A badge: never broken.
    Badge(Vec<Piece>),
    Space,
    Break,
}

fn pieces(s: &str, role: &'static str) -> Vec<Piece> {
    s.graphemes(true)
        .map(|g| {
            let p = printable(g);
            Piece {
                g: p.to_string(),
                w: width(p) as u16,
                role,
            }
        })
        .collect()
}

/// A key badge for a command id: ` ⌃O ` (or `[Ctrl-O]` in ASCII) in the key role.
pub(crate) fn badge(command: &str, keys: &dyn KeyLabels, glyphs: Glyphs) -> Vec<Piece> {
    let label = keys.label(command).unwrap_or_else(|| command.to_string());
    let text = match glyphs {
        Glyphs::Ascii => format!("[{label}]"),
        _ => format!(" {label} "),
    };
    pieces(&text, "key")
}

/// Splits text with markup into tokens. `role` is the base role of plain text.
pub(crate) fn tokens(
    text: &str,
    role: &'static str,
    keys: &dyn KeyLabels,
    glyphs: Glyphs,
) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let (plain, markup, next) = match rest.find("{{") {
            Some(i) => match rest[i..].find("}}") {
                Some(j) => (&rest[..i], Some(&rest[i + 2..i + j]), &rest[i + j + 2..]),
                None => (rest, None, ""),
            },
            None => (rest, None, ""),
        };
        words(plain, role, &mut out);
        match markup {
            Some(m) if m.starts_with("key:") => {
                out.push(Tok::Badge(badge(m[4..].trim(), keys, glyphs)))
            }
            Some(m) if m.starts_with("code:") => words(&m[5..], "code", &mut out),
            Some(m) => words(&format!("{{{{{m}}}}}"), role, &mut out),
            None => {}
        }
        rest = next;
    }
    out
}

fn words(s: &str, role: &'static str, out: &mut Vec<Tok>) {
    let mut word = Vec::new();
    for g in s.graphemes(true) {
        match g {
            "\n" | "\r\n" => {
                if !word.is_empty() {
                    out.push(Tok::Word(std::mem::take(&mut word)));
                }
                out.push(Tok::Break);
            }
            " " | "\t" => {
                if !word.is_empty() {
                    out.push(Tok::Word(std::mem::take(&mut word)));
                }
                out.push(Tok::Space);
            }
            _ => word.extend(pieces(g, role)),
        }
    }
    if !word.is_empty() {
        out.push(Tok::Word(word));
    }
}

/// Wraps tokens to lines of at most `width` cells. A word wider than a line breaks between
/// graphemes; a grapheme that would cross the edge goes to the next line.
pub(crate) fn wrap(toks: &[Tok], width: u16) -> Vec<Vec<Piece>> {
    let width = width.max(2);
    let mut lines: Vec<Vec<Piece>> = vec![Vec::new()];
    let mut w = 0u16;
    let mut pending_space = false;
    for t in toks {
        match t {
            Tok::Break => {
                lines.push(Vec::new());
                w = 0;
                pending_space = false;
            }
            Tok::Space => pending_space = w > 0,
            Tok::Word(ps) | Tok::Badge(ps) => {
                let tw: u16 = ps.iter().map(|p| p.w).sum();
                let space = u16::from(pending_space);
                if w > 0 && w + space + tw > width {
                    lines.push(Vec::new());
                    w = 0;
                } else if pending_space {
                    lines.last_mut().unwrap().push(Piece {
                        g: " ".into(),
                        w: 1,
                        role: ps[0].role_for_space(),
                    });
                    w += 1;
                }
                pending_space = false;
                for p in ps {
                    if w + p.w > width {
                        lines.push(Vec::new());
                        w = 0;
                    }
                    lines.last_mut().unwrap().push(p.clone());
                    w += p.w;
                }
            }
        }
    }
    while lines.len() > 1 && lines.last().is_some_and(Vec::is_empty) {
        lines.pop();
    }
    lines
}

impl Piece {
    /// The role of a space before this piece: plain text, unless it continues code.
    fn role_for_space(&self) -> &'static str {
        if self.role == "code" {
            "code"
        } else {
            "callout"
        }
    }
}

/// The width of a line of pieces.
pub(crate) fn line_width(line: &[Piece]) -> u16 {
    line.iter().map(|p| p.w).sum()
}

/// `s` cut to at most `max` cells, ending in `…` when cut.
pub(crate) fn truncate(s: &str, max: usize, glyphs: Glyphs) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    let ell = if glyphs == Glyphs::Ascii {
        "..."
    } else {
        "…"
    };
    let room = max.saturating_sub(str_width(ell));
    let mut out = String::new();
    let mut w = 0;
    for g in s.graphemes(true) {
        let gw = width(printable(g));
        if w + gw > room {
            break;
        }
        out.push_str(printable(g));
        w += gw;
    }
    if max >= str_width(ell) {
        out.push_str(ell);
    }
    out
}
