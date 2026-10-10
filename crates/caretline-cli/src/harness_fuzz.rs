//! The harness fuzz (terminal harness step 5): random documents, carets and terminal sizes,
//! random physical key presses on four Ghosttys (default and with `caretline doctor`'s fixes,
//! with and without the keyboard protocol), and the screen-truth check after every key. A
//! failure is shrunk to the fewest keys that still fail and printed as a `caretline sim`
//! command that replays it. `CARETLINE_HARNESS_SEEDS` runs more seeds than the default.

use caretline::commands::{binding_key, default_keymap, key_notation};
use caretline::keymap::{Key, ScriptItem, parse_keys};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::harness::Harness;
use crate::keyboard::TermKeyboard;

const SEEDS: u64 = 32;

/// Text pieces. Left out, as `vt100` gets them wrong where Ghostty doesn't (each found by this
/// fuzz): emoji joined with ZWJ (👨‍👩‍👧) and skin-tone modifiers (👍🏽), which it splits into
/// several cells, flags (🇫🇷), which it stores as two narrow halves so a repaint leaves one
/// behind, emoji presentation by VS16 (❤️), which it keeps one cell wide, and U+FFFD, which it
/// drops (the editor draws a zero-width space as one).
/// Swapping the emulator for libghostty-vt would let them back in.
const PIECES: &[&str] = &[
    "a", "b", "word", "Hello", "the", " ", "  ", "\t", "\n", "\n", ".", ",", "-", "e\u{301}",
    "漢字", "カナ", "é",
];

/// One case: what the terminal is, the document, and the keys.
#[derive(Clone, Debug)]
struct Case {
    notation: String,
    fixed: bool,
    legacy: bool,
    size: (u16, u16),
    keys: Vec<Key>,
}

impl Case {
    fn keyboard(&self) -> TermKeyboard {
        let mut kb = TermKeyboard::ghostty_default()
            .with_config(&fixes().join("\n").repeat(self.fixed as usize));
        if self.legacy {
            kb = kb.legacy();
        }
        kb
    }

    /// Runs the case; the check's message if the screen ever doesn't show the state.
    fn run(&self) -> Result<(), String> {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut h = Harness::new(&self.notation, self.keyboard(), self.size.0, self.size.1);
            for k in &self.keys {
                h.press_key(*k);
            }
        }));
        std::panic::set_hook(hook);
        result.map_err(|e| {
            e.downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default()
        })
    }

    fn script(&self) -> String {
        self.keys.iter().map(key_notation).collect()
    }

    /// The `caretline sim` command that replays the case.
    fn sim(&self) -> String {
        let mut cmd = format!(
            "caretline sim '{}' '{}' --size {}x{}",
            escape(&self.notation),
            self.script(),
            self.size.0,
            self.size.1
        );
        if self.legacy {
            cmd.push_str(" --legacy");
        }
        if self.fixed {
            for f in fixes() {
                cmd.push_str(&format!(" --config '{f}'"));
            }
        }
        cmd
    }
}

/// The lines `caretline doctor` gives for default Ghostty.
fn fixes() -> Vec<String> {
    crate::doctor::conflicts(&TermKeyboard::ghostty_default())
        .into_iter()
        .map(|c| c.fix)
        .collect()
}

/// Text for a `caretline sim` argument: control chars as `\n`, `\t`, `\u{…}`.
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() || c == '\u{200b}' => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32))
            }
            c => out.push(c),
        }
    }
    out
}

/// Every key a binding names, and keys that type: weighted toward editing.
fn keys() -> Vec<Key> {
    let mut keys: Vec<Key> = [false, true]
        .into_iter()
        .flat_map(default_keymap)
        .filter_map(|b| binding_key(&b))
        .collect();
    for s in [
        "a", "Z", "<space>", "é", "漢", "<cr>", "<bs>", "<del>", "<tab>",
    ] {
        for item in parse_keys(s).unwrap() {
            if let ScriptItem::Key(k) = item {
                // Typing and deleting: three times as likely as any one command.
                keys.extend([k, k, k]);
            }
        }
    }
    keys
}

fn case(rng: &mut StdRng, pool: &[Key], fixed: bool, legacy: bool) -> Case {
    let n = rng.random_range(0..40);
    let pieces: Vec<&str> = (0..n)
        .map(|_| PIECES[rng.random_range(0..PIECES.len())])
        .collect();
    // One to three carets between pieces: where a person's keys can put one (never inside a
    // grapheme, such as between a flag's two halves).
    let mut at: Vec<usize> = (0..rng.random_range(1..=3))
        .map(|_| rng.random_range(0..=pieces.len()))
        .collect();
    at.sort();
    at.dedup();
    let mut notation = String::new();
    for i in 0..=pieces.len() {
        if at.contains(&i) {
            notation.push('▮');
        }
        if let Some(p) = pieces.get(i) {
            notation.push_str(p);
        }
    }
    let size = (rng.random_range(8..48), rng.random_range(3..10));
    let keys = (0..rng.random_range(1..48))
        .map(|_| pool[rng.random_range(0..pool.len())])
        .collect();
    Case {
        notation,
        fixed,
        legacy,
        size,
        keys,
    }
}

/// Drops keys while the case still fails: the fewest keys that show the bug.
fn shrink(mut case: Case) -> Case {
    loop {
        let mut smaller = None;
        for i in 0..case.keys.len() {
            let mut c = case.clone();
            c.keys.remove(i);
            if c.run().is_err() {
                smaller = Some(c);
                break;
            }
        }
        match smaller {
            Some(c) => case = c,
            None => return case,
        }
    }
}

#[test]
fn the_screen_shows_the_state_under_random_keys() {
    let seeds: u64 = std::env::var("CARETLINE_HARNESS_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(SEEDS);
    let pool = keys();
    let mut presses = 0;
    for seed in 0..seeds {
        for (fixed, legacy) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut rng = StdRng::seed_from_u64(seed * 4 + fixed as u64 * 2 + legacy as u64);
            let c = case(&mut rng, &pool, fixed, legacy);
            // The printed script replays the same keys.
            let replayed: Vec<Key> = parse_keys(&c.script())
                .unwrap()
                .into_iter()
                .filter_map(|i| match i {
                    ScriptItem::Key(k) => Some(k),
                    ScriptItem::Wait(_) => None,
                })
                .collect();
            assert_eq!(replayed.len(), c.keys.len(), "script {}", c.script());
            presses += c.keys.len();
            if let Err(e) = c.run() {
                let small = shrink(c);
                panic!(
                    "seed {seed}: the screen doesn't show the state.\n{e}\n\nreplay ({} keys):\n  {}",
                    small.keys.len(),
                    small.sim()
                );
            }
        }
    }
    assert!(presses > 0);
}
