//! Raw terminal input: the one decoder from stdin's bytes to keys, mouse reports, pastes and
//! terminal replies. The runtime reads stdin itself (not with crossterm's event reader), so a
//! reply mid-session (the pixel probe's answers, the cell size after a font change) becomes
//! an `Input::Reply`, never a key: crossterm's reader would turn `ESC _ G … ESC \` into
//! Alt-_ and a run of typed letters.
//!
//! Keys come in the legacy xterm forms and the kitty keyboard protocol's `CSI … u` (the
//! runtime pushes its disambiguate flag when the terminal answers the query). They come out
//! as crossterm `Event`s, decoded as crossterm's own reader decodes them, so the parser can be
//! checked against it. Replies are recognised by `caretline_layers::probe::scan`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use caretline_layers::probe::{self, Reply, Scan};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::hub::Input;

/// What the parser read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Event(Event),
    Reply(Reply),
    /// The kitty keyboard protocol's flags (`CSI ? flags u`): the terminal speaks it.
    Keyboard(u8),
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

    /// The bytes read and not yet decoded.
    pub fn buffered(&self) -> &[u8] {
        &self.buf
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
                Esc::Flags(f, n) => {
                    self.buf.drain(..n);
                    return Some(Token::Keyboard(f));
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

fn key_kind(code: KeyCode, mods: KeyModifiers, kind: KeyEventKind) -> Event {
    Event::Key(KeyEvent::new_with_kind(code, mods, kind))
}

/// A byte that isn't ESC: a char or a control key. `None` while a UTF-8 char is unfinished.
fn plain(b: &[u8]) -> Option<(Option<Event>, usize)> {
    let c = b[0];
    let none = KeyModifiers::NONE;
    let ev = match c {
        // In raw mode `\n` is Ctrl-J, as `\x08` is Ctrl-H: only `\r` is Enter.
        b'\r' => key(KeyCode::Enter, none),
        b'\t' => key(KeyCode::Tab, none),
        0x7f => key(KeyCode::Backspace, none),
        0x00 => key(KeyCode::Char(' '), KeyModifiers::CONTROL),
        0x01..=0x1a => key(KeyCode::Char((b'a' + c - 1) as char), KeyModifiers::CONTROL),
        0x1c..=0x1f => key(
            KeyCode::Char((b'4' + c - 0x1c) as char),
            KeyModifiers::CONTROL,
        ),
        _ if c < 0x80 => typed(c as char),
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
                    Some(ch) => (Some(typed(ch)), len),
                    None => (None, 1),
                },
            );
        }
    };
    Some((Some(ev), 1))
}

/// A typed char: an uppercase one carries Shift.
fn typed(ch: char) -> Event {
    let mods = if ch.is_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    key(KeyCode::Char(ch), mods)
}

enum Esc {
    Key(Event, usize),
    /// The kitty keyboard flags reply.
    Flags(u8, usize),
    /// A sequence the editor ignores, and its length.
    Skip(usize),
    /// A bracketed paste starts.
    Paste(usize),
    Wait,
}

/// A key's `mods[:kind]` parameter: 1 + the modifier bits (xterm's shift 1, alt 2, ctrl 4,
/// super 8, and kitty's hyper 16, meta 32; caps and num lock are dropped), then kitty's event
/// type (1 press, 2 repeat, 3 release).
fn mods_kind(p: Option<&str>) -> (KeyModifiers, KeyEventKind) {
    let mut parts = p.unwrap_or("").split(':');
    let bits = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1)
        .saturating_sub(1);
    let mut m = KeyModifiers::NONE;
    for (bit, flag) in [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
        (8, KeyModifiers::SUPER),
        (16, KeyModifiers::HYPER),
        (32, KeyModifiers::META),
    ] {
        if bits & bit != 0 {
            m |= flag;
        }
    }
    let kind = match parts.next().and_then(|s| s.parse::<u32>().ok()) {
        Some(2) => KeyEventKind::Repeat,
        Some(3) => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    };
    (m, kind)
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
    if let Some(flags) = params.strip_prefix('?')
        && fin == b'u'
    {
        return flags.parse().map_or(Esc::Skip(n), |f: u8| Esc::Flags(f, n));
    }
    if params.starts_with(['?', '>', '=']) {
        return Esc::Skip(n);
    }
    let fields: Vec<&str> = params.split(';').collect();
    let (m, kind) = mods_kind(fields.get(1).copied());
    let first = fields[0]
        .split(':')
        .next()
        .and_then(|s| s.parse::<u32>().ok());
    // xterm's modifyOtherKeys (`CSI 27;m;code ~`): Ghostty's legacy form for a key with no
    // other one, such as Shift-Enter.
    if fin == b'~'
        && first == Some(27)
        && let Some(code) = fields.get(2)
    {
        return kitty(code, m, kind).map_or(Esc::Skip(n), |ev| Esc::Key(ev, n));
    }
    let code = match fin {
        b'u' => return kitty(fields[0], m, kind).map_or(Esc::Skip(n), |ev| Esc::Key(ev, n)),
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'P'..=b'S' => KeyCode::F(fin - b'P' + 1),
        b'Z' => return Esc::Key(key(KeyCode::BackTab, KeyModifiers::SHIFT), n),
        b'~' => match first.unwrap_or(0) {
            200 => return Esc::Paste(n),
            1 | 7 => KeyCode::Home,
            2 => KeyCode::Insert,
            3 => KeyCode::Delete,
            4 | 8 => KeyCode::End,
            5 => KeyCode::PageUp,
            6 => KeyCode::PageDown,
            v @ 11..=15 => KeyCode::F((v - 10) as u8),
            v @ 17..=21 => KeyCode::F((v - 11) as u8),
            v @ 23..=26 => KeyCode::F((v - 12) as u8),
            v @ 28..=29 => KeyCode::F((v - 15) as u8),
            v @ 31..=34 => KeyCode::F((v - 17) as u8),
            _ => return Esc::Skip(n),
        },
        _ => return Esc::Skip(n),
    };
    Esc::Key(key_kind(code, m, kind), n)
}

/// A kitty keyboard protocol key (`CSI code[:shifted[:base]] ; mods[:kind] u`). With Shift
/// and a shifted code (the alternate-keys flag), the key is the shifted char without Shift.
/// Keys the editor never binds (media, lone modifiers, lock keys) are skipped.
fn kitty(codes: &str, mut m: KeyModifiers, kind: KeyEventKind) -> Option<Event> {
    let mut codes = codes.split(':').map(|s| s.parse::<u32>().ok());
    let cp = codes.next().flatten()?;
    let mut code = match cp {
        // The keypad's own codes.
        57399..=57408 => KeyCode::Char(char::from_digit(cp - 57399, 10)?),
        57409..=57416 => match cp {
            57414 => KeyCode::Enter,
            _ => KeyCode::Char("./*-+ =,".chars().nth((cp - 57409) as usize)?),
        },
        57417 => KeyCode::Left,
        57418 => KeyCode::Right,
        57419 => KeyCode::Up,
        57420 => KeyCode::Down,
        57421 => KeyCode::PageUp,
        57422 => KeyCode::PageDown,
        57423 => KeyCode::Home,
        57424 => KeyCode::End,
        57425 => KeyCode::Insert,
        57426 => KeyCode::Delete,
        57427 => KeyCode::KeypadBegin,
        57376..=57398 => KeyCode::F((cp - 57363) as u8),
        // Lock keys, media keys and lone modifiers.
        57344..=63743 => return None,
        _ => match char::from_u32(cp)? {
            '\x1b' => KeyCode::Esc,
            '\r' => KeyCode::Enter,
            '\t' if m.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            '\t' => KeyCode::Tab,
            '\x7f' => KeyCode::Backspace,
            c => KeyCode::Char(c),
        },
    };
    if m.contains(KeyModifiers::SHIFT)
        && let Some(c) = codes.next().flatten().and_then(char::from_u32)
    {
        code = KeyCode::Char(c);
        m.remove(KeyModifiers::SHIFT);
    }
    Some(key_kind(code, m, kind))
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
    fill_unless(parser, timeout, &AtomicBool::new(false))
}

/// [`fill`], but leaves the bytes unread once `stop` is set: they belong to whoever reads
/// the terminal next.
fn fill_unless(parser: &mut Parser, timeout: Duration, stop: &AtomicBool) -> bool {
    // select(2), not poll(2): on macOS poll can't wait on a tty device such as /dev/tty
    // (`install.sh --demo` gives us that as stdin) and says it's ready at once, so the read
    // below would block until the next key, and a stopping reader would take that key.
    let micros = timeout.as_micros().min(i32::MAX as u128);
    let mut tv = libc::timeval {
        tv_sec: (micros / 1_000_000) as libc::time_t,
        tv_usec: (micros % 1_000_000) as libc::suseconds_t,
    };
    // SAFETY: an fd_set zeroed then holding stdin, and a valid timeval.
    let ready = unsafe {
        let mut set: libc::fd_set = std::mem::zeroed();
        libc::FD_ZERO(&mut set);
        libc::FD_SET(0, &mut set);
        libc::select(
            1,
            &mut set,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut tv,
        )
    };
    if ready <= 0 || stop.load(Ordering::SeqCst) {
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

/// Writes `request` (which must end with DA1, `CSI c`, the fence) and reads until DA1
/// answers or `timeout` passes. Every reply read is returned, in order (DA1 included); keys
/// typed meanwhile stay in the parser for the event loop.
pub fn probe(
    parser: &mut Parser,
    out: &mut dyn std::io::Write,
    request: &[u8],
    timeout: Duration,
) -> Vec<Token> {
    if out.write_all(request).and_then(|_| out.flush()).is_err() {
        return Vec::new();
    }
    let end = Instant::now() + timeout;
    let mut replies = Vec::new();
    let mut keys = Vec::new();
    loop {
        while let Some(t) = parser.next(false) {
            match t {
                Token::Event(e) => keys.push(e),
                t => {
                    let fence = matches!(t, Token::Reply(Reply::Da1(_)));
                    replies.push(t);
                    if fence {
                        parser.unread(keys);
                        return replies;
                    }
                }
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
/// It stops reading once `stop` is set, within one poll: join it before anything else reads
/// the terminal, or it takes the next key.
pub fn spawn(mut parser: Parser, tx: Sender<Input>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for e in std::mem::take(&mut parser.early) {
            if tx.send(Input::Terminal(e)).is_err() {
                return;
            }
        }
        let mut size = crossterm::terminal::size().ok();
        while !stop.load(Ordering::SeqCst) {
            let wait = if parser.pending() {
                Duration::from_millis(30)
            } else {
                Duration::from_millis(100)
            };
            let got = fill_unless(&mut parser, wait, &stop);
            while let Some(t) = parser.next(!got) {
                let input = match t {
                    Token::Event(e) => Input::Terminal(e),
                    Token::Reply(r) => Input::Reply(r),
                    Token::Keyboard(_) => continue,
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
    })
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

    /// Bytes to the editor's key, in its notation (`None`: no key the editor reads). Each row
    /// decodes as crossterm 0.29's reader does (checked over a generated corpus when the
    /// parser replaced it), except where marked: there crossterm drops or misreads the key.
    #[test]
    fn bytes_to_keys() {
        use caretline::commands::key_notation;
        let table: &[(&[u8], Option<&str>)] = &[
            // Legacy: text, control bytes, Alt as an ESC prefix.
            (b"a", Some("<a>")),
            (b"A", Some("<s-a>")),
            ("É".as_bytes(), Some("<s-É>")),
            (b"\r", Some("<cr>")),
            (b"\n", Some("<c-j>")),
            (b"\t", Some("<tab>")),
            (b"\x7f", Some("<bs>")),
            (b"\x08", Some("<c-h>")),
            (b"\x01", Some("<c-a>")),
            (b"\x00", Some("<c-space>")),
            (b"\x1b", Some("<esc>")),
            (b"\x1bb", Some("<a-b>")),
            (b"\x1bB", Some("<a-s-b>")),
            (b"\x1b\x7f", Some("<a-bs>")),
            (b"\x1b[Z", Some("<s-tab>")),
            // xterm's modifier parameter, super included.
            (b"\x1b[D", Some("<left>")),
            (b"\x1bOD", Some("<left>")),
            (b"\x1b[1;2D", Some("<s-left>")),
            (b"\x1b[1;3D", Some("<a-left>")),
            (b"\x1b[1;5C", Some("<c-right>")),
            (b"\x1b[1;9D", Some("<d-left>")),
            (b"\x1b[1;10H", Some("<d-s-home>")),
            (b"\x1b[3~", Some("<del>")),
            (b"\x1b[3;5~", Some("<c-del>")),
            (b"\x1b[5~", Some("<pgup>")),
            (b"\x1b[6;2~", Some("<s-pgdn>")),
            (b"\x1b[4~", Some("<end>")),
            (b"\x1b[15~", None),
            // The kitty keyboard protocol (disambiguate): Ctrl, Alt and Cmd keys, Esc.
            (b"\x1b[27u", Some("<esc>")),
            (b"\x1b[97;9u", Some("<d-a>")),
            (b"\x1b[97;10u", Some("<d-s-a>")),
            (b"\x1b[122;9u", Some("<d-z>")),
            (b"\x1b[97;5u", Some("<c-a>")),
            (b"\x1b[106;5u", Some("<c-j>")),
            (b"\x1b[118;3u", Some("<a-v>")),
            (b"\x1b[47;9u", Some("<d-/>")),
            (b"\x1b[13;2u", Some("<s-cr>")),
            (b"\x1b[9;2u", Some("<s-tab>")),
            (b"\x1b[127;3u", Some("<a-bs>")),
            (b"\x1b[32;5u", Some("<c-space>")),
            // Caps and num lock don't count; hyper is no editor modifier, meta is Alt.
            (b"\x1b[97;69u", Some("<c-a>")),
            (b"\x1b[97;133u", Some("<c-a>")),
            (b"\x1b[97;33u", Some("<a-a>")),
            // Report-alternate-keys: the shifted char, without Shift.
            (b"\x1b[49:33;2u", Some("<!>")),
            // Event types: a release is still decoded (the runtime drops it).
            (b"\x1b[97;5:3u", Some("<c-a>")),
            // The keypad's own codes.
            (b"\x1b[57399u", Some("<0>")),
            (b"\x1b[57414u", Some("<cr>")),
            (b"\x1b[57417;2u", Some("<s-left>")),
            // Lock keys, media keys, lone modifiers: skipped.
            (b"\x1b[57358u", None),
            (b"\x1b[57441;2u", None),
            // xterm's modifyOtherKeys, Ghostty's legacy form for Shift-Enter and the like
            // (crossterm drops them).
            (b"\x1b[27;2;13~", Some("<s-cr>")),
            (b"\x1b[27;9;97~", Some("<d-a>")),
            (b"\x1b[27;5;27~", Some("<c-esc>")),
            // Where crossterm differs: Alt and a lone `[` or `O` (crossterm: nothing), and
            // `CSI 1;5R` as Ctrl-F3 (crossterm: a cursor report nobody asked for).
            (b"\x1b[", Some("<a-[>")),
            (b"\x1bO", Some("<a-s-o>")),
            (b"\x1b[1;5R", None),
        ];
        for (bytes, want) in table {
            let got: Vec<String> = all(bytes)
                .into_iter()
                .filter_map(|t| match t {
                    Token::Event(Event::Key(k)) => crate::runtime::to_key(&k),
                    _ => None,
                })
                .map(|k| key_notation(&k))
                .collect();
            let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
            assert_eq!(got, want, "{:?}", String::from_utf8_lossy(bytes));
        }
        // Two ESCs are two Esc presses (crossterm: one).
        assert_eq!(all(b"\x1b\x1b").len(), 2);
        let Token::Event(Event::Key(k)) = &all(b"\x1b[97;5:3u")[0] else {
            panic!()
        };
        assert_eq!(k.kind, KeyEventKind::Release);
    }

    #[test]
    fn keyboard_flags_reply() {
        assert_eq!(
            all(b"\x1b[?1u\x1b[?62;22c"),
            vec![Token::Keyboard(1), Token::Reply(Reply::Da1(vec![62, 22]))]
        );
    }

    /// A lone ESC waits for more (it may start a sequence), and is the Esc key once no byte
    /// came for a while (the reader's flush).
    #[test]
    fn a_lone_esc_waits_then_is_esc() {
        let mut p = Parser::default();
        p.push(b"\x1b");
        assert_eq!(p.next(false), None);
        assert_eq!(p.next(true), Some(k(KeyCode::Esc, KeyModifiers::NONE)));
        p.push(b"\x1b");
        assert_eq!(p.next(false), None);
        p.push(b"[A");
        assert_eq!(p.next(false), Some(k(KeyCode::Up, KeyModifiers::NONE)));
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
