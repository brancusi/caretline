//! One-line documents ([`crate::Config::single_line`]): the text never holds a line break.
//!
//! Every way text gets in is flattened the same way: line breaks at the end are dropped, and
//! every other line break (`\r\n` counted once) becomes one space. Local edits are flattened
//! where they are committed ([`flatten_txn`]), so typing, pasting, a host's `Edit`s and input
//! rules all agree; changes from elsewhere and loaded states are flattened as text
//! ([`flatten`]).

use crate::helix::chars::char_is_line_ending;
use crate::helix::transaction::Operation;
use crate::helix::{Range, Rope, Selection, SmallVec, Tendril, Transaction};

/// A line break: `\n`, `\r`, and with the `unicode-lines` feature every Unicode line separator.
pub(crate) fn is_break(c: char) -> bool {
    c == '\n' || c == '\r' || char_is_line_ending(c)
}

pub(crate) fn has_break(s: &str) -> bool {
    s.chars().any(is_break)
}

/// Whether the rope holds a line break.
pub(crate) fn rope_has_break(text: &Rope) -> bool {
    text.len_lines() > 1 || text.chars().any(is_break)
}

/// `text` on one line: line breaks at the end dropped, every other one (CRLF counted once) a
/// space.
pub(crate) fn flatten(text: &str) -> String {
    if !has_break(text) {
        return text.to_string();
    }
    flatten_mapped(text).0
}

/// [`flatten`], and where each char boundary of `text` (`0..=len` in chars) lands in it.
fn flatten_mapped(text: &str) -> (String, Vec<usize>) {
    let chars: Vec<char> = text.chars().collect();
    let cut = chars.len() - chars.iter().rev().take_while(|c| is_break(**c)).count();
    let mut out = String::with_capacity(text.len());
    let mut map = vec![0; chars.len() + 1];
    let mut n = 0;
    let mut i = 0;
    while i < cut {
        map[i] = n;
        let c = chars[i];
        if c == '\r' && chars.get(i + 1) == Some(&'\n') {
            out.push(' ');
            n += 1;
            map[i + 1] = n;
            i += 2;
            continue;
        }
        out.push(if is_break(c) { ' ' } else { c });
        n += 1;
        i += 1;
    }
    for m in &mut map[cut..] {
        *m = n;
    }
    (out, map)
}

/// `txn` (against `old`) with every text it inserts flattened, and its selection moved to
/// match. Unchanged when it inserts no line break.
pub(crate) fn flatten_txn(old: &Rope, txn: Transaction) -> Transaction {
    let breaks = txn.changes().changes().iter().any(|op| matches!(op, Operation::Insert(s) if has_break(s)));
    if !breaks {
        return txn;
    }
    // Each change: old `[from, to)`, its inserted text, and where it starts in the new text.
    let mut changes: Vec<(usize, usize, String, usize)> = Vec::new();
    let mut open: Option<(usize, usize, String, usize)> = None;
    let (mut old_pos, mut new_pos) = (0, 0);
    for op in txn.changes().changes() {
        match op {
            Operation::Retain(n) => {
                changes.extend(open.take());
                old_pos += n;
                new_pos += n;
            }
            Operation::Delete(n) => {
                open.get_or_insert((old_pos, old_pos, String::new(), new_pos)).1 += n;
                old_pos += n;
            }
            Operation::Insert(s) => {
                open.get_or_insert((old_pos, old_pos, String::new(), new_pos)).2.push_str(s);
                new_pos += s.chars().count();
            }
        }
    }
    changes.extend(open.take());

    // Where each insert starts in the unflattened new text, its length, and its map.
    let mut inserts: Vec<(usize, usize, Vec<usize>)> = Vec::new();
    let mut flat: Vec<(usize, usize, Option<Tendril>)> = Vec::new();
    for (from, to, text, start) in changes {
        let (f, map) = flatten_mapped(&text);
        inserts.push((start, text.chars().count(), map));
        flat.push((from, to, (!f.is_empty()).then(|| Tendril::from(f.as_str()))));
    }
    let place = |p: usize| {
        let mut shift = 0;
        for (start, len, map) in &inserts {
            if p < *start {
                break;
            }
            if p <= start + len {
                return start - shift + map[p - start];
            }
            shift += len - map[*len];
        }
        p - shift
    };
    let out = Transaction::change(old, flat.into_iter());
    match txn.selection() {
        Some(sel) => {
            let ranges: SmallVec<[Range; 1]> = sel
                .iter()
                .map(|r| Range { anchor: place(r.anchor), head: place(r.head), old_visual_position: r.old_visual_position })
                .collect();
            out.with_selection(Selection::new(ranges, sel.primary_index()))
        }
        None => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_breaks() {
        assert_eq!(flatten("a\nb"), "a b");
        assert_eq!(flatten("a\r\nb\rc"), "a b c");
        assert_eq!(flatten("a\n\nb"), "a  b");
        assert_eq!(flatten("a\r\n\n"), "a");
        assert_eq!(flatten("\nfoo"), " foo");
        assert_eq!(flatten("\n"), "");
    }

    #[test]
    fn maps_positions() {
        let (s, map) = flatten_mapped("a\r\nb\n");
        assert_eq!(s, "a b");
        assert_eq!(map, vec![0, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn flattens_a_transaction_and_its_selection() {
        let old = Rope::from("xy");
        let txn = Transaction::change(&old, [(1, 1, Some(Tendril::from("a\r\nb")))].into_iter())
            .with_selection(Selection::point(5));
        let txn = flatten_txn(&old, txn);
        let mut text = old.clone();
        txn.apply(&mut text);
        assert_eq!(text.to_string(), "xa by");
        assert_eq!(txn.selection().unwrap().primary().head, 4);
    }
}
