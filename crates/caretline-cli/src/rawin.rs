//! Raw terminal input: the runtime reads stdin's bytes itself (instead of crossterm's event
//! reader) when it may get terminal replies mid-session: the pixel probe's answers and the
//! cell size after a font change. A reply becomes an `Input::Reply`, never a key: crossterm's
//! reader would turn `ESC _ G … ESC \` into Alt-_ and a run of typed letters.
//!
//! Keys, mouse reports and pastes come out as crossterm `Event`s, so the rest of the runtime
//! is the same either way. Replies are recognised by `caretline_layers::probe::scan`.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use caretline_layers::probe::{self, Reply, Scan};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::hub::Input;

/// What the parser read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Event(Event),
    Reply(Reply),
}

/// A byte-stream parser: keys, mouse reports, pastes and terminal replies.
#[derive(Default)]
pub struct Parser {
    buf: Vec<u8>,
    /// Inside a bracketed paste.
    paste: bool,
    /// Events read during the probe, for the event loop.
    early: Vec<Event>,
}

const PASTE_END: &[u8] = b"\x1b[201~";

impl Parser {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Whether bytes are waiting for more (an escape that may be a sequence's start).
    pub fn pending(&self) -> bool {
        !self.buf.is_empty()
    }

    /// The next token. With `flush` (no byte came for a while), an unfinished escape is read
    /// as keys: a lone ESC is the Esc key.
    pub fn next(&mut self, flush: bool) -> Option<Token> {
        loop {
            if self.buf.is_empty() {
                return None;
            }
            if self.paste {
                let end = self
                    .buf
                    .windows(PASTE_END.len())
                    .position(|w| w == PASTE_END)?;
                let text = String::from_utf8_lossy(&self.buf[..end])
                    .replace("\r\n", "\n")
                    .replace('\r', "\n");
                self.buf.drain(..end + PASTE_END.len());
                self.paste = false;
                return Some(Token::Event(Event::Paste(text)));
            }
            if self.buf[0] != 0x1b {
                let (ev, n) = match plain(&self.buf) {
                    Some(v) => v,
                    None if flush => (None, 1),
                    None => return None,
                };
                self.buf.drain(..n);
                match ev {
                    Some(ev) => return Some(Token::Event(ev)),
                    None => continue,
                }
            }
            match probe::scan(&self.buf) {
                Scan::Reply(r, n) => {
                    self.buf.drain(..n);
                    return Some(Token::Reply(r));
                }
                Scan::Partial | Scan::No => {}
            }
            match escape(&self.buf) {
                Esc::Key(ev, n) => {
                    self.buf.drain(..n);
                    return Some(Token::Event(ev));
                }
                Esc::Skip(n) => {
                    self.buf.drain(..n);
                }
                Esc::Paste(n) => {
                    self.buf.drain(..n);
                    self.paste = true;
                }
                Esc::Wait if flush => {
                    // An escape that never finished: ESC alone is the Esc key, otherwise
                    // Alt and the next char.
                    if self.buf.len() == 1 {
                        self.buf.clear();
                        return Some(Token::Event(key(KeyCode::Esc, KeyModifiers::NONE)));
                    }
                    self.buf.remove(0);
                    if let Some((Some(Event::Key(mut k)), n)) = plain(&self.buf) {
                        self.buf.drain(..n);
                        k.modifiers |= KeyModifiers::ALT;
                        return Some(Token::Event(Event::Key(k)));
                    }
                }
                Esc::Wait => return None,
            }
        }
    }
}

fn key(code: KeyCode, mods: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, mods))
}

/// A byte that isn't ESC: a char or a control key. `None` while a UTF-8 char is unfinished.
fn plain(b: &[u8]) -> Option<(Option<Event>, usize)> {
    let c = b[0];
    let none = KeyModifiers::NONE;
    let ev = match c {
        b'\r' | b'\n' => key(KeyCode::Enter, none),
        b'\t' => key(KeyCode::Tab, none),
        0x7f | 0x08 => key(KeyCode::Backspace, none),
        0x00 => key(KeyCode::Char(' '), KeyModifiers::CONTROL),
        0x01..=0x1a => key(KeyCode::Char((b'a' + c - 1) as char), KeyModifiers::CONTROL),
        0x1c..=0x1f => key(
            KeyCode::Char((b'4' + c - 0x1c) as char),
            KeyModifiers::CONTROL,
        ),
        _ if c < 0x80 => {
            let ch = c as char;
            let mods = if ch.is_ascii_uppercase() {
                KeyModifiers::SHIFT
            } else {
                none
            };
            key(KeyCode::Char(ch), mods)
        }
        _ => {
            let len = match c {
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => return Some((None, 1)),
            };
            if b.len() < len {
                return None;
            }
            return Some(
                match std::str::from_utf8(&b[..len])
                    .ok()
                    .and_then(|s| s.chars().next())
                {
                    Some(ch) => (Some(key(KeyCode::Char(ch), none)), len),
                    None => (None, 1),
                },
            );
        }
    };
    Some((Some(ev), 1))
}

enum Esc {
    Key(Event, usize),
    /// A sequence the editor ignores, and its length.
    Skip(usize),
    /// A bracketed paste starts.
    Paste(usize),
    Wait,
}

/// xterm's modifier parameter (1 + bits: shift 1, alt 2, ctrl 4, super 8).
fn mods(p: u32) -> KeyModifiers {
    let bits = p.saturating_sub(1);
    let mut m = KeyModifiers::NONE;
    if bits & 1 != 0 {
        m |= KeyModifiers::SHIFT;
    }
    if bits & 2 != 0 {
        m |= KeyModifiers::ALT;
    }
    if bits & 4 != 0 {
        m |= KeyModifiers::CONTROL;
    }
    if bits & 8 != 0 {
        m |= KeyModifiers::SUPER;
    }
    m
}

fn escape(b: &[u8]) -> Esc {
    if b.len() < 2 {
        return Esc::Wait;
    }
    match b[1] {
        b'[' => csi(b),
        b'O' => {
            let Some(&c) = b.get(2) else { return Esc::Wait };
            let code = match c {
                b'A' => KeyCode::Up,
                b'B' => KeyCode::Down,
                b'C' => KeyCode::Right,
                b'D' => KeyCode::Left,
                b'H' => KeyCode::Home,
                b'F' => KeyCode::End,
                b'P'..=b'S' => KeyCode::F(c - b'P' + 1),
                _ => return Esc::Skip(3),
            };
            Esc::Key(key(code, KeyModifiers::NONE), 3)
        }
        // An unfinished APC or DCS that may still be a reply.
        b'_' | b'P' if probe::scan(b) == Scan::Partial => Esc::Wait,
        0x1b => Esc::Key(key(KeyCode::Esc, KeyModifiers::NONE), 1),
        _ => match plain(&b[1..]) {
            Some((Some(Event::Key(mut k)), n)) => {
                k.modifiers |= KeyModifiers::ALT;
                Esc::Key(Event::Key(k), n + 1)
            }
            Some((_, n)) => Esc::Skip(n + 1),
            None => Esc::Wait,
        },
    }
}

fn csi(b: &[u8]) -> Esc {
    let Some(end) = b[2..].iter().position(|c| (0x40..=0x7e).contains(c)) else {
        return if b[2..].iter().all(|c| (0x20..=0x3f).contains(c)) {
            Esc::Wait
        } else {
            Esc::Skip(2)
        };
    };
    let n = 2 + end + 1;
    let params = String::from_utf8_lossy(&b[2..2 + end]).into_owned();
    let fin = b[2 + end];
    if let Some(sgr) = params.strip_prefix('<') {
        return mouse(sgr, fin).map_or(Esc::Skip(n), |ev| Esc::Key(ev, n));
    }
    if params.starts_with(['?', '>', '=']) {
        return Esc::Skip(n);
    }
    let nums: Vec<u32> = params
        .split(';')
        .map(|s| s.split(':').next().unwrap_or("").parse().unwrap_or(0))
        .collect();
    let m = mods(nums.get(1).copied().unwrap_or(1));
    let code = match fin {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'P'..=b'S' => KeyCode::F(fin - b'P' + 1),
        b'Z' => return Esc::Key(key(KeyCode::BackTab, KeyModifiers::SHIFT), n),
        b'~' => match nums.first().copied().unwrap_or(0) {
            200 => return Esc::Paste(n),
            1 | 7 => KeyCode::Home,
            2 => KeyCode::Insert,
            3 => KeyCode::Delete,
            4 | 8 => KeyCode::End,
            5 => KeyCode::PageUp,
            6 => KeyCode::PageDown,
            11..=15 => KeyCode::F((nums[0] - 10) as u8),
            17..=21 => KeyCode::F((nums[0] - 11) as u8),
            23 | 24 => KeyCode::F((nums[0] - 12) as u8),
            _ => return Esc::Skip(n),
        },
        _ => return Esc::Skip(n),
    };
    Esc::Key(key(code, m), n)
}

/// An SGR mouse report (`CSI < b ; x ; y M|m`).
fn mouse(params: &str, fin: u8) -> Option<Event> {
    let v: Vec<u32> = params
        .split(';')
        .map(|s| s.parse().ok())
        .collect::<Option<_>>()?;
    let [cb, x, y] = v[..] else { return None };
    let mut modifiers = KeyModifiers::NONE;
    if cb & 4 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if cb & 8 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if cb & 16 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    let button = match cb & 3 {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        _ => MouseButton::Right,
    };
    let kind = if cb & 64 != 0 {
        match cb & 3 {
            0 => MouseEventKind::ScrollUp,
            1 => MouseEventKind::ScrollDown,
            2 => MouseEventKind::ScrollLeft,
            _ => MouseEventKind::ScrollRight,
        }
    } else if cb & 32 != 0 {
        if cb & 3 == 3 {
            MouseEventKind::Moved
        } else {
            MouseEventKind::Drag(button)
        }
    } else if fin == b'm' {
        MouseEventKind::Up(button)
    } else {
        MouseEventKind::Down(button)
    };
    Some(Event::Mouse(MouseEvent {
        kind,
        column: x.saturating_sub(1).min(u16::MAX as u32) as u16,
        row: y.saturating_sub(1).min(u16::MAX as u32) as u16,
        modifiers,
    }))
}

/// Reads stdin's bytes (raw mode is on), waiting at most `timeout`. Returns whether any came.
pub fn fill(parser: &mut Parser, timeout: Duration) -> bool {
    let mut fds = libc::pollfd {
        fd: 0,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: one valid pollfd for stdin.
    if unsafe { libc::poll(&mut fds, 1, ms) } <= 0 {
        return false;
    }
    let mut b = [0u8; 8192];
    // SAFETY: reads into a buffer of the length given.
    let n = unsafe { libc::read(0, b.as_mut_ptr() as *mut libc::c_void, b.len()) };
    if n > 0 {
        parser.push(&b[..n as usize]);
    }
    n > 0
}

/// Writes the probe and reads until DA1 answers or `timeout` passes. Every reply read is
/// returned, in order (the probe's, and any other); keys typed meanwhile stay in the parser
/// for the event loop.
pub fn probe(parser: &mut Parser, out: &mut dyn std::io::Write, timeout: Duration) -> Vec<Reply> {
    if out
        .write_all(&probe::request())
        .and_then(|_| out.flush())
        .is_err()
    {
        return Vec::new();
    }
    let end = Instant::now() + timeout;
    let mut replies = Vec::new();
    let mut keys = Vec::new();
    loop {
        while let Some(t) = parser.next(false) {
            match t {
                Token::Reply(r) => {
                    let fence = matches!(r, Reply::Da1(_));
                    replies.push(r);
                    if fence {
                        parser.unread(keys);
                        return replies;
                    }
                }
                Token::Event(e) => keys.push(e),
            }
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() || !fill(parser, left) && Instant::now() >= end {
            parser.unread(keys);
            return replies;
        }
    }
}

impl Parser {
    /// Puts events read during the probe back, ahead of anything else.
    fn unread(&mut self, events: Vec<Event>) {
        self.early.extend(events);
    }
}

/// Runs the reader on its own thread: tokens go to the event loop as `Input::Terminal` and
/// `Input::Reply`; a size change (polled, since the reader owns stdin) as a resize event.
pub fn spawn(mut parser: Parser, tx: Sender<Input>) {
    std::thread::spawn(move || {
        for e in std::mem::take(&mut parser.early) {
            if tx.send(Input::Terminal(e)).is_err() {
                return;
            }
        }
        let mut size = crossterm::terminal::size().ok();
        loop {
            let wait = if parser.pending() {
                Duration::from_millis(30)
            } else {
                Duration::from_millis(100)
            };
            let got = fill(&mut parser, wait);
            while let Some(t) = parser.next(!got) {
                let input = match t {
                    Token::Event(e) => Input::Terminal(e),
                    Token::Reply(r) => Input::Reply(r),
                };
                if tx.send(input).is_err() {
                    return;
                }
            }
            let now = crossterm::terminal::size().ok();
            if now != size {
                size = now;
                if let Some((w, h)) = now
                    && tx.send(Input::Terminal(Event::Resize(w, h))).is_err()
                {
                    return;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(bytes: &[u8]) -> Vec<Token> {
        let mut p = Parser::default();
        p.push(bytes);
        let mut out = Vec::new();
        while let Some(t) = p.next(true) {
            out.push(t);
        }
        out
    }

    fn k(code: KeyCode, m: KeyModifiers) -> Token {
        Token::Event(key(code, m))
    }

    #[test]
    fn replies_mid_session_are_replies_not_keys() {
        let got =
            all(b"a\x1b_Gi=31;OK\x1b\\b\x1b[6;34;16t\x1bP>|ghostty 1.3.1\x1b\\\x1b[?62;22c\x1b[A");
        let none = KeyModifiers::NONE;
        assert_eq!(
            got,
            vec![
                k(KeyCode::Char('a'), none),
                Token::Reply(Reply::Graphics {
                    id: Some(31),
                    ok: true,
                    message: "OK".into()
                }),
                k(KeyCode::Char('b'), none),
                Token::Reply(Reply::CellSize(caretline_layers::kitty::CellPx::new(
                    16, 34
                ))),
                Token::Reply(Reply::Version("ghostty 1.3.1".into())),
                Token::Reply(Reply::Da1(vec![62, 22])),
                k(KeyCode::Up, none),
            ]
        );
    }

    #[test]
    fn keys_mouse_and_paste() {
        let none = KeyModifiers::NONE;
        assert_eq!(all(b"\x1b[1;3D"), vec![k(KeyCode::Left, KeyModifiers::ALT)]);
        assert_eq!(
            all(b"\x1b[6~\x1bOP\x1b[Z"),
            vec![
                k(KeyCode::PageDown, none),
                k(KeyCode::F(1), none),
                k(KeyCode::BackTab, KeyModifiers::SHIFT),
            ]
        );
        assert_eq!(
            all(b"\x03\x1bx\x1b"),
            vec![
                k(KeyCode::Char('c'), KeyModifiers::CONTROL),
                k(KeyCode::Char('x'), KeyModifiers::ALT),
                k(KeyCode::Esc, none),
            ]
        );
        assert_eq!(
            all("é⌥".as_bytes()),
            vec![k(KeyCode::Char('é'), none), k(KeyCode::Char('⌥'), none)]
        );
        assert_eq!(
            all(b"\x1b[200~hi\r\nyou\x1b[201~"),
            vec![Token::Event(Event::Paste("hi\nyou".into()))]
        );
        let Token::Event(Event::Mouse(m)) = &all(b"\x1b[<65;10;5M")[0] else {
            panic!()
        };
        assert_eq!(
            (m.kind, m.column, m.row),
            (MouseEventKind::ScrollDown, 9, 4)
        );
        let Token::Event(Event::Mouse(m)) = &all(b"\x1b[<0;3;4M")[0] else {
            panic!()
        };
        assert_eq!(m.kind, MouseEventKind::Down(MouseButton::Left));
    }

    #[test]
    fn split_reads_wait_for_the_rest() {
        let mut p = Parser::default();
        p.push(b"\x1b_Gi=31;O");
        assert_eq!(p.next(false), None);
        p.push(b"K\x1b\\");
        assert!(matches!(
            p.next(false),
            Some(Token::Reply(Reply::Graphics { ok: true, .. }))
        ));
        p.push(b"\x1b[");
        assert_eq!(p.next(false), None);
        p.push(b"B");
        assert_eq!(p.next(false), Some(k(KeyCode::Down, KeyModifiers::NONE)));
    }
}
