//! The interactive runtime: terminal events in, effects out. Everything that touches the
//! clock, the terminal, files or the clipboard lives here, never in the engine.

use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use caretline::protocol::Change;
use caretline::view::Role;
use caretline::{Effect, Frame, Key, KeyCode, Mods, Msg, Session, State, keymap_for};

use crate::hub::{self, Hub, Input};
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton, MouseEvent,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    BeginSynchronizedUpdate, EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement,
};
use crossterm::{execute, queue};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Modifier, Style};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Converts a crossterm key to the engine's key type.
fn to_key(ev: &event::KeyEvent) -> Option<Key> {
    use event::KeyCode as C;
    let code = match ev.code {
        C::Char(c) => KeyCode::Char(c),
        C::Enter => KeyCode::Enter,
        C::Backspace => KeyCode::Backspace,
        C::Delete => KeyCode::Delete,
        C::Left => KeyCode::Left,
        C::Right => KeyCode::Right,
        C::Up => KeyCode::Up,
        C::Down => KeyCode::Down,
        C::Home => KeyCode::Home,
        C::End => KeyCode::End,
        C::PageUp => KeyCode::PageUp,
        C::PageDown => KeyCode::PageDown,
        C::Tab => KeyCode::Tab,
        C::BackTab => KeyCode::BackTab,
        C::Esc => KeyCode::Esc,
        _ => return None,
    };
    let m = ev.modifiers;
    Some(Key {
        code,
        mods: Mods {
            shift: m.contains(KeyModifiers::SHIFT),
            ctrl: m.contains(KeyModifiers::CONTROL),
            alt: m.contains(KeyModifiers::ALT) || m.contains(KeyModifiers::META),
            cmd: m.contains(KeyModifiers::SUPER),
        },
    })
}

/// Reads the system clipboard, if a known tool is available.
fn read_system_clipboard() -> Option<String> {
    let candidates: &[(&str, &[&str])] = &[
        ("pbpaste", &[]),
        ("wl-paste", &["--no-newline"]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
    ];
    for (cmd, args) in candidates {
        if let Ok(out) = Command::new(cmd).args(*args).stderr(Stdio::null()).output()
            && out.status.success()
        {
            return String::from_utf8(out.stdout).ok();
        }
    }
    None
}

/// Writes the system clipboard with a known tool, else with the OSC 52 escape sequence.
fn write_system_clipboard(text: &str) {
    let candidates: &[(&str, &[&str])] = &[
        ("pbcopy", &[]),
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (cmd, args) in candidates {
        let child = Command::new(cmd)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return;
            }
        }
    }
    let encoded = base64(text.as_bytes());
    let mut out = io::stdout();
    let _ = write!(out, "\x1b]52;c;{encoded}\x07");
    let _ = out.flush();
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(T[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Writes a file through a temporary sibling and a rename, so a failed write never leaves
/// a half-written file.
fn write_file(path: &str, text: &str) -> io::Result<()> {
    let target = Path::new(path);
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let tmp = dir.join(format!(".{name}.caretline-tmp"));
    fs::write(&tmp, text)?;
    if let Ok(meta) = fs::metadata(target) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }
    fs::rename(&tmp, target).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Performs one effect for the local user. Returns the message that reports its result.
fn perform(effect: &Effect, quit: &mut bool) -> Option<Msg> {
    match effect {
        Effect::WriteFile { path, text } => Some(match write_file(path, text) {
            Ok(()) => Msg::Saved,
            Err(e) => Msg::SaveFailed { err: e.to_string() },
        }),
        Effect::ClipboardSet { text } => {
            write_system_clipboard(text);
            None
        }
        Effect::Quit => {
            *quit = true;
            None
        }
        // Notices only come with the status bar off; the rest (block_left, refused, host and
        // any later kinds) are for hosts that keep their own data.
        _ => None,
    }
}

fn no_color() -> bool {
    static NO_COLOR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NO_COLOR.get_or_init(|| std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()))
}

/// The style of a cell's role: built-in roles, and the layer roles the CLI's renderers use
/// (crate::layers). Other named roles draw plain.
fn style_in(frame: &Frame, role: Role) -> Style {
    use crate::layers::{ACCENT, DIM, INK, PANEL, RULE, VOID};
    let Role::Named(_) = role else {
        return style(role);
    };
    let rgb = |c: (u8, u8, u8)| Color::Rgb(c.0, c.1, c.2);
    let plain = no_color();
    match frame.role_name(role) {
        "layer.callout" if !plain => Style::default().fg(rgb(INK)).bg(rgb(PANEL)),
        "layer.border" | "layer.arrow" if plain => Style::default().add_modifier(Modifier::BOLD),
        "brand.selection" if plain => Style::default().add_modifier(Modifier::REVERSED),
        "brand.chrome" if plain => style(Role::Status),
        "brand.text" if !plain => Style::default().fg(rgb(INK)).bg(rgb(VOID)),
        "brand.selection" if !plain => Style::default().fg(rgb(VOID)).bg(rgb(ACCENT)),
        "brand.chrome" if !plain => Style::default().fg(rgb(DIM)).bg(rgb(PANEL)),
        "layer.border" => Style::default().fg(rgb(RULE)).bg(rgb(PANEL)),
        "layer.arrow" => Style::default()
            .fg(rgb(ACCENT))
            .add_modifier(Modifier::BOLD),
        "layer.title" if plain => Style::default().add_modifier(Modifier::BOLD),
        "layer.title" => Style::default()
            .fg(rgb(INK))
            .bg(rgb(PANEL))
            .add_modifier(Modifier::BOLD),
        "layer.title.px" if plain => Style::default().add_modifier(Modifier::BOLD),
        "layer.title.px" => Style::default().fg(rgb(INK)).add_modifier(Modifier::BOLD),
        "layer.ring" if plain => Style::default().add_modifier(Modifier::UNDERLINED),
        "layer.ring" => Style::default()
            .fg(rgb(INK))
            .bg(Color::Rgb(0x1c, 0x1b, 0x2c)),
        "layer.chip" if plain => Style::default().add_modifier(Modifier::REVERSED),
        "layer.chip" => Style::default().fg(rgb(INK)).bg(rgb(PANEL)),
        _ => Style::default(),
    }
}

fn style(role: Role) -> Style {
    match role {
        Role::Text => Style::default(),
        Role::Selection => Style::default().bg(Color::Blue).fg(Color::White),
        Role::Status => Style::default().add_modifier(Modifier::REVERSED),
        Role::StatusAccent => Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
        Role::Hang => Style::default().add_modifier(Modifier::DIM),
        Role::Named(_) => Style::default(),
    }
}

fn restore_terminal(kitty: bool, mouse: bool) {
    let mut out = io::stdout();
    if kitty {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    if mouse {
        let _ = execute!(out, DisableMouseCapture);
    }
    let _ = execute!(
        out,
        DisableBracketedPaste,
        SetCursorStyle::DefaultUserShape,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}

/// Options for the interactive editor.
pub struct Interactive<'a> {
    pub trace: Option<&'a str>,
    pub mouse: bool,
    /// Serve the state protocol on this socket.
    pub listen: Option<PathBuf>,
    /// The file name to advertise in the discovery file.
    pub file: Option<&'a str>,
    /// The in-memory trace limit (see `Session::set_trace_limit`).
    pub trace_limit: usize,
    /// Repaint at most this many times a second (0: no cap).
    pub max_fps: u32,
    /// The frame clock to start with, in frames per second (0: off).
    pub frame_clock: u16,
    /// Print repaint statistics on exit.
    pub stats: bool,
    /// A built-in demo driving this editor (`caretline demo`).
    pub demo: Option<Box<dyn Demo>>,
}

/// What a demo does with a key.
pub enum KeyAction {
    /// Not the demo's: the keymap gets it.
    Pass,
    /// Handled by the demo.
    Consumed,
    /// Quit the editor.
    Quit,
}

/// A built-in demo (`caretline demo`): hooks into the event loop for keys of its own, a
/// status hint, timed work (a replay), a second pane and frames of its own. Everything it
/// changes goes through the session like any other input, so the trace records it.
pub trait Demo {
    /// A terminal key, before the keymap.
    fn key(&mut self, _hub: &mut Hub, _key: &Key) -> KeyAction {
        KeyAction::Pass
    }
    /// A terminal paste, before the editor applies it. Read-only presentations can
    /// consume it without preventing scripted or socket edits to the engine.
    fn paste(&mut self, _hub: &mut Hub, _text: &str) -> KeyAction {
        KeyAction::Pass
    }
    /// Runs after every batch of input (and once at the start).
    fn after(&mut self, _hub: &mut Hub) {}
    /// Advances anything timed. Returns when it next wants to run.
    fn poll(&mut self, _hub: &mut Hub, _now: Instant) -> Option<Instant> {
        None
    }
    /// Rows at the bottom of a `height`-row terminal for a pane showing the first other view
    /// (0: no pane).
    fn pane_rows(&self, _hub: &Hub, _height: u16) -> u16 {
        0
    }
    /// A frame to draw instead of the editor's own (a replay). Paired with `generation`.
    fn overlay(&self) -> Option<Frame> {
        None
    }
    /// Goes up whenever `overlay` would draw something new.
    fn generation(&self) -> u64 {
        0
    }
    /// Whether the demo draws layers that may use pixels: the runtime then reads the
    /// terminal's input itself (so replies become `Input::Reply`, not keys) and probes it at
    /// startup.
    fn wants_pixels(&self) -> bool {
        false
    }
    /// Draws over the frame about to be painted (layers). `gfx` says whether pixels are on.
    fn decorate(&mut self, _hub: &Hub, _frame: &mut Frame, _gfx: &Gfx) -> Decor {
        Decor::default()
    }
    /// Bytes that take down whatever the demo put on the terminal outside the cells (images),
    /// written before the overlay or exit.
    fn hide(&mut self) -> Vec<u8> {
        Vec::new()
    }
}

/// What the probe found: pixels on (the cell size in device pixels) or not, and why.
///
/// `cell_px` here is what the probe found; the runtime hands it to the state as
/// `Msg::Resize { cell_px }` (and a later `CSI 16 t` answer the same way), and draws pixels
/// from the state's `View::cell_px`, so pixel output is a function of the state and replays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gfx {
    pub cell_px: Option<caretline_layers::kitty::CellPx>,
    /// What XTVERSION said.
    pub terminal: Option<String>,
    /// Why pixels are on or off, for the status bar.
    pub why: String,
    /// Not over SSH: `t=t` temporary files may be used.
    pub local: bool,
}

/// What a demo adds to a painted frame.
#[derive(Debug, Clone, Default)]
pub struct Decor {
    /// Cells to dim (row by row; empty for none).
    pub dim: Vec<bool>,
    /// Bytes to write after the cells, inside the same synchronized update (kitty graphics).
    pub bytes: Vec<u8>,
}

/// How layers draw: `CARETLINE_LAYERS=auto|pixels|cells` (default auto).
fn layers_mode() -> String {
    std::env::var("CARETLINE_LAYERS")
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Probes the terminal for pixels: the graphics query, XTVERSION and the cell size, fenced by
/// DA1, waiting at most 200 ms. Pixels are on when the query says OK, the cell size came
/// back, and the terminal is Ghostty or kitty (or `CARETLINE_LAYERS=pixels`).
fn probe_gfx(parser: &mut crate::rawin::Parser) -> Gfx {
    use caretline_layers::probe::Probe;
    let mode = layers_mode();
    let local =
        std::env::var_os("SSH_CONNECTION").is_none() && std::env::var_os("SSH_TTY").is_none();
    let off = |why: &str| Gfx {
        cell_px: None,
        terminal: None,
        why: why.to_string(),
        local,
    };
    if mode == "cells" {
        return off("CARETLINE_LAYERS=cells");
    }
    if mode != "pixels" && (std::env::var_os("TMUX").is_some() || std::env::var_os("STY").is_some())
    {
        return off("inside a multiplexer");
    }
    let replies = crate::rawin::probe(parser, &mut io::stdout(), Duration::from_millis(200));
    let mut p = Probe::default();
    for r in &replies {
        p.add(r);
    }
    let terminal = p.version.clone();
    let name = p.terminal().unwrap_or_default();
    let trusted = name == "ghostty" || name == "kitty" || mode == "pixels";
    let why = if !p.fenced {
        "no answer from the terminal".to_string()
    } else if p.graphics != Some(true) {
        "no kitty graphics".to_string()
    } else if p.cell.is_none() {
        "no cell size".to_string()
    } else if !trusted {
        format!(
            "{} untested (CARETLINE_LAYERS=pixels)",
            if name.is_empty() { "terminal" } else { &name }
        )
    } else {
        terminal.clone().unwrap_or_else(|| "pixels".into())
    };
    let on = p.fenced && p.graphics_ok() && trusted;
    Gfx {
        cell_px: if on { p.cell } else { None },
        terminal,
        why,
        local,
    }
}

/// Applies messages from a demo, performing their effects (a save, a quit).
pub fn dispatch_demo(hub: &mut Hub, msgs: Vec<Msg>) -> bool {
    let mut quit = false;
    dispatch_local(hub, msgs, &mut quit, "runtime");
    quit
}

/// The editor's frame with the pane below it: the first other view, its caret drawn as a
/// selected cell (the terminal has one cursor, the person's).
pub fn compose(hub: &Hub, pane_rows: u16) -> Frame {
    compose_state(hub.session.state(), hub.session.views(), pane_rows)
}

/// [`compose`] for a state and its other views.
pub fn compose_state(state: &State, views: &[(u32, caretline::View)], pane_rows: u16) -> Frame {
    let top = caretline::view(state);
    let Some((_, v)) = views.first().filter(|_| pane_rows > 0) else {
        return top;
    };
    let mut s = State::from_parts(state.doc.clone(), v.clone());
    if (s.view.viewport.width, s.view.viewport.height) != (top.width, pane_rows) {
        caretline::update(&mut s, Msg::resize(top.width, pane_rows));
    }
    let mut pane = caretline::view(&s);
    if let Some((x, y)) = pane.cursor.take() {
        let i = y as usize * pane.width as usize + x as usize;
        if let Some(cell) = pane.cells.get_mut(i) {
            cell.role = Role::Selection;
        }
    }
    stack(top, pane)
}

/// One frame above another (the same width).
pub fn stack(mut top: Frame, bottom: Frame) -> Frame {
    top.height += bottom.height;
    top.cells.extend(bottom.cells);
    top.rows.extend(bottom.rows);
    top
}

/// What the event loop counts, for `--stats`.
#[derive(Default)]
struct Stats {
    paints: u64,
    paint_time: Duration,
    frames: u64,
    inputs: u64,
}

pub fn run_interactive(state: State, opts: Interactive<'_>) -> Result<(), String> {
    let trace = match opts.trace {
        Some(p) => Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| format!("{p}: {e}"))?,
        ),
        None => None,
    };
    let mut session = Session::new(state);
    session.set_trace_limit(opts.trace_limit);
    let mut hub = Hub::new(session, trace);
    // Clients write through their own views: view 0 is the person's.
    hub.client_views = true;
    let (tx, rx) = mpsc::channel::<Input>();

    // Bind before touching the terminal, so a bad path is an ordinary error.
    let listening = match &opts.listen {
        Some(path) => {
            let mut l = hub::listen(path, tx.clone())?;
            l.advertise(opts.file)
                .map_err(|e| format!("discovery file: {e}"))?;
            Some(l)
        }
        None => None,
    };

    let mouse = opts.mouse;
    enable_raw_mode().map_err(|e| format!("terminal: {e}"))?;
    // A demo with layers reads input itself and probes for pixels. It leaves the keyboard
    // protocol alone: crossterm's query would read stdin from under the raw reader.
    let raw = opts.demo.as_ref().is_some_and(|d| d.wants_pixels());
    let mut parser = crate::rawin::Parser::default();
    let gfx = if raw {
        probe_gfx(&mut parser)
    } else {
        Gfx::default()
    };
    let kitty = !raw && supports_keyboard_enhancement().unwrap_or(false);
    let mut out = io::stdout();
    let _ = execute!(
        out,
        EnterAlternateScreen,
        EnableBracketedPaste,
        SetCursorStyle::SteadyBar
    );
    if mouse {
        let _ = execute!(out, EnableMouseCapture);
    }
    if kitty {
        let _ = execute!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal(kitty, mouse);
        previous_hook(info);
    }));

    // Terminal events join socket requests on one queue: one order, one trace. The reader
    // stops when the editor does, so a key typed next goes to whatever runs next (a demo's
    // next chapter), not to a reader nobody listens to.
    let term_tx = tx.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = if raw {
        crate::rawin::spawn(parser, term_tx, stop.clone())
    } else {
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match event::poll(Duration::from_millis(100)) {
                    Ok(true) if !stop.load(Ordering::SeqCst) => {}
                    Ok(_) => continue,
                    Err(_) => break,
                }
                let Ok(ev) = event::read() else { break };
                if term_tx.send(Input::Terminal(ev)).is_err() {
                    break;
                }
            }
        })
    };
    drop(tx);

    let status = listening
        .as_ref()
        .map(|l| format!("listening on {}", l.path.display()));
    let started = Instant::now();
    let mut stats = Stats::default();
    let pacing = Pacing {
        max_fps: opts.max_fps,
        frame_clock: opts.frame_clock,
    };
    let result = event_loop(&mut hub, rx, status, pacing, &mut stats, opts.demo, gfx);
    stop.store(true, Ordering::SeqCst);
    let _ = reader.join();
    restore_terminal(kitty, mouse);
    if opts.stats {
        let secs = started.elapsed().as_secs_f64();
        let mean = stats.paint_time.as_secs_f64() * 1e3 / stats.paints.max(1) as f64;
        eprintln!(
            "caretline: {} repaints in {secs:.1} s ({:.1}/s, {mean:.2} ms each), {} inputs, {} clock frames",
            stats.paints,
            stats.paints as f64 / secs,
            stats.inputs,
            stats.frames
        );
    }
    if let Some(l) = &listening {
        eprintln!(
            "caretline: served the state protocol on {}",
            l.path.display()
        );
    }
    drop(listening);
    result
}

/// Turns a terminal event into messages (none for events the editor ignores).
fn terminal_msgs(state: &State, ev: Event) -> Vec<Msg> {
    match ev {
        Event::Key(k) if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            match to_key(&k).and_then(|key| keymap_for(state.doc.outline.is_some(), &key)) {
                // The keymap is pure, so a paste key carries no text; fill it in here from
                // the system clipboard so the trace records exactly what was pasted.
                Some(Msg::Paste { text: None }) => vec![Msg::Paste {
                    text: read_system_clipboard(),
                }],
                Some(Msg::PastePlain { text: None }) => vec![Msg::PastePlain {
                    text: read_system_clipboard(),
                }],
                Some(msg) => vec![msg],
                None => vec![],
            }
        }
        Event::Paste(text) => vec![Msg::Paste { text: Some(text) }],
        Event::Resize(width, height) => vec![Msg::resize(width, height)],
        Event::Mouse(m) => {
            let text_rows = state.text_rows() as u16;
            let extend = m.modifiers.contains(KeyModifiers::SHIFT);
            match m.kind {
                MouseEventKind::Down(MouseButton::Left) if m.row < text_rows => {
                    vec![Msg::Click {
                        col: m.column,
                        row: m.row,
                        extend,
                    }]
                }
                // At the first or last text row (or past it) a drag scrolls a row.
                MouseEventKind::Drag(MouseButton::Left) => vec![Msg::Drag {
                    col: m.column,
                    row: m.row,
                }],
                MouseEventKind::ScrollUp => vec![Msg::Scroll { rows: -3 }],
                MouseEventKind::ScrollDown => vec![Msg::Scroll { rows: 3 }],
                _ => vec![],
            }
        }
        _ => vec![],
    }
}

/// Held-pointer auto-scroll. The engine scrolls a row for each `Msg::Drag` on an edge row,
/// but a terminal reports a drag only when the pointer moves, so a pointer held still at the
/// edge would stop. While the button is held and the last drag scrolled the view from an
/// edge row, the runtime sends that drag again on a timer, through the normal message path
/// (so the trace records each one and a replay scrolls the same). It stops when the button
/// is released, the pointer moves off the edge, or a drag no longer scrolls.
#[derive(Default)]
struct AutoScroll {
    /// The cell of the drag to repeat.
    at: Option<(u16, u16)>,
    /// When to send it next.
    next: Option<Instant>,
}

impl AutoScroll {
    /// About every 50 ms; faster the further past the last text row the pointer is (the
    /// status bar, or a pane below), up to four times as fast.
    fn interval(state: &State, row: u16) -> Duration {
        let last = (state.text_rows() as u16).saturating_sub(1);
        let past = row.saturating_sub(last).min(3) as u32;
        Duration::from_millis(50) / (1 + past)
    }

    /// The first text row, or the last one or past it.
    fn on_edge(state: &State, row: u16) -> bool {
        row == 0 || row + 1 >= state.text_rows() as u16
    }

    fn stop(&mut self) {
        *self = AutoScroll::default();
    }

    /// Follows a mouse event the editor just applied; `scrolled` says whether it moved the
    /// view.
    fn mouse(&mut self, state: &State, m: MouseEvent, scrolled: bool, now: Instant) {
        match m.kind {
            MouseEventKind::Drag(MouseButton::Left) if scrolled && Self::on_edge(state, m.row) => {
                self.at = Some((m.column, m.row));
                self.next = Some(now + Self::interval(state, m.row));
            }
            MouseEventKind::Drag(_)
            | MouseEventKind::Down(_)
            | MouseEventKind::Up(_)
            | MouseEventKind::Moved => self.stop(),
            _ => {}
        }
    }

    /// Sends the held drag again if it is due.
    fn poll(&mut self, hub: &mut Hub, now: Instant, quit: &mut bool) {
        let (Some((col, row)), Some(due)) = (self.at, self.next) else {
            return;
        };
        if now < due {
            return;
        }
        if !Self::on_edge(hub.session.state(), row) {
            return self.stop();
        }
        let before = hub.session.state().view.scroll;
        // No `tick` before it: a tick isn't a pointer message, so the view would follow the
        // caret (at the edge row) by its scrolloff and jump more than a row.
        dispatch_local(hub, vec![Msg::Drag { col, row }], quit, "terminal");
        if hub.session.state().view.scroll == before {
            return self.stop();
        }
        // On a schedule from the last send; after a stall, from now rather than in a burst.
        let period = Self::interval(hub.session.state(), row);
        let next = due + period;
        self.next = Some(if next <= now { now + period } else { next });
    }
}

/// Applies messages from the local user (or the runtime itself), performing their effects,
/// and tells subscribers.
fn dispatch_local(hub: &mut Hub, msgs: Vec<Msg>, quit: &mut bool, source: &str) {
    if msgs.is_empty() {
        return;
    }
    let mut applied = Vec::new();
    for msg in msgs {
        let (_, m) = hub.session.apply_with(msg, &mut |e| perform(e, quit));
        applied.extend(m);
    }
    let change = Change {
        rev: hub.session.rev(),
        msgs: applied,
        state_set: false,
        view: None,
    };
    hub.changed(&change, source);
}

/// How the event loop paces itself.
#[derive(Clone, Copy)]
struct Pacing {
    max_fps: u32,
    frame_clock: u16,
}

type Term = Terminal<CrosstermBackend<BufWriter<io::Stdout>>>;

/// Sizes view 0 to the terminal less the demo's pane, and the pane's view to the pane.
fn fit_views(
    hub: &mut Hub,
    demo: &Option<Box<dyn Demo>>,
    term: Option<(u16, u16)>,
    quit: &mut bool,
) {
    let (Some(demo), Some((w, h))) = (demo, term) else {
        return;
    };
    let rows = demo.pane_rows(hub, h).min(h.saturating_sub(2));
    let v = hub.session.state().view.viewport;
    let top = h - rows;
    if (v.width, v.height) != (w, top) {
        dispatch_local(hub, vec![Msg::resize(w, top)], quit, "runtime");
    }
    if rows > 0
        && let Some((id, view)) = hub.session.views().first()
        && (view.viewport.width, view.viewport.height) != (w, rows)
    {
        let id = *id;
        hub.session.apply_on(id, Msg::resize(w, rows));
    }
}

fn event_loop(
    hub: &mut Hub,
    rx: Receiver<Input>,
    status: Option<String>,
    pacing: Pacing,
    stats: &mut Stats,
    mut demo: Option<Box<dyn Demo>>,
    mut gfx: Gfx,
) -> Result<(), String> {
    // One buffered write per repaint, wrapped in a synchronized update (below).
    let backend = CrosstermBackend::new(BufWriter::with_capacity(1 << 16, io::stdout()));
    let mut terminal = Terminal::new(backend).map_err(|e| format!("terminal: {e}"))?;
    let mut quit = false;
    let mut term = terminal.size().map(|s| (s.width, s.height)).ok();

    let mut start = vec![Msg::Tick { now_ms: now_ms() }];
    let v = hub.session.state().view.viewport;
    let (w, h) = term.unwrap_or((v.width, v.height));
    // The probe's cell size goes into the state, where pixels are drawn from.
    let cell_px = gfx.cell_px.map(|c| caretline::CellPx::new(c.w, c.h));
    if (w, h) != (v.width, v.height)
        || cell_px.is_some_and(|c| Some(c) != hub.session.state().view.cell_px)
    {
        start.push(Msg::Resize {
            width: w,
            height: h,
            cell_px,
        });
    }
    if let Some(text) = status {
        start.push(Msg::ShowStatus { text });
    }
    if pacing.frame_clock > 0 {
        start.push(Msg::FrameClock {
            fps: pacing.frame_clock,
        });
    }
    dispatch_local(hub, start, &mut quit, "runtime");
    fit_views(hub, &demo, term, &mut quit);
    if let Some(d) = &mut demo {
        d.after(hub);
    }

    // The keys overlay (F1 or Alt-?): its scroll offset while it is shown.
    let mut help: Option<usize> = None;
    let mut auto = AutoScroll::default();
    let process = |hub: &mut Hub,
                   auto: &mut AutoScroll,
                   demo: &mut Option<Box<dyn Demo>>,
                   term: &mut Option<(u16, u16)>,
                   help: &mut Option<usize>,
                   gfx: &mut Gfx,
                   input: Input,
                   quit: &mut bool| match input {
        // A reply read mid-session: the cell size after a font change. Others (a late probe
        // answer) change nothing. It goes in as a message, so the trace records it and pixel
        // output replays.
        Input::Reply(caretline_layers::probe::Reply::CellSize(c)) => {
            let v = &hub.session.state().view;
            let c = caretline::CellPx::new(c.w, c.h);
            if gfx.cell_px.is_some() && c.w > 0 && c.h > 0 && v.cell_px != Some(c) {
                let msg = Msg::Resize {
                    width: v.viewport.width,
                    height: v.viewport.height,
                    cell_px: Some(c),
                };
                dispatch_local(hub, vec![msg], quit, "runtime");
            }
        }
        Input::Reply(_) => {}
        Input::Connect { client, out } => hub.connect(client, out),
        Input::Disconnect { client } => hub.disconnect(client),
        Input::Line { client, line } => {
            // Pushed messages' effects are returned, not performed, unless the request
            // sets apply_effects.
            let change = hub.request(client, &line, Some(&mut |e| perform(e, quit)));
            // A replaced state keeps the terminal's size.
            if let (Some(c), Some((w, h))) = (change, *term) {
                let v = hub.session.state().view.viewport;
                if c.state_set && (w, h) != (v.width, v.height) {
                    dispatch_local(hub, vec![Msg::resize(w, h)], quit, "runtime");
                }
            }
        }
        Input::Terminal(ev) => {
            if let Event::Key(k) = &ev
                && matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat)
            {
                let asks = k.code == event::KeyCode::F(1)
                    || (k.code == event::KeyCode::Char('?')
                        && k.modifiers.contains(KeyModifiers::ALT));
                match (*help, k.code) {
                    (Some(o), event::KeyCode::Down | event::KeyCode::PageDown) => {
                        *help = Some(
                            o + if k.code == event::KeyCode::Down {
                                1
                            } else {
                                10
                            },
                        );
                        return;
                    }
                    (Some(o), event::KeyCode::Up | event::KeyCode::PageUp) => {
                        *help = Some(o.saturating_sub(if k.code == event::KeyCode::Up {
                            1
                        } else {
                            10
                        }));
                        return;
                    }
                    (Some(_), _) => {
                        *help = None;
                        return;
                    }
                    (None, _) if asks => {
                        *help = Some(0);
                        return;
                    }
                    _ => {}
                }
            }
            if let Event::Resize(w, h) = ev {
                *term = Some((w, h));
                // The window's pixels disagree with cells × cell size: the font size changed.
                // Ask again; the answer comes back as an `Input::Reply`.
                if let (Some(c), Ok(ws)) = (
                    hub.session
                        .state()
                        .view
                        .cell_px
                        .filter(|_| gfx.cell_px.is_some()),
                    crossterm::terminal::window_size(),
                ) && ws.width > 0
                    && (ws.width as u32 != ws.columns as u32 * c.w as u32
                        || ws.height as u32 != ws.rows as u32 * c.h as u32)
                {
                    let mut out = io::stdout();
                    let _ = out
                        .write_all(caretline_layers::probe::cell_size_request())
                        .and_then(|_| out.flush());
                }
                if demo.is_some() {
                    fit_views(hub, demo, *term, quit);
                    return;
                }
            }
            if let (Some(d), Event::Paste(text)) = (demo.as_mut(), &ev) {
                match d.paste(hub, text) {
                    KeyAction::Pass => {}
                    KeyAction::Consumed => return,
                    KeyAction::Quit => {
                        *quit = true;
                        return;
                    }
                }
            }
            if let (Some(d), Event::Key(k)) = (demo.as_mut(), &ev)
                && matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                && let Some(key) = to_key(k)
            {
                match d.key(hub, &key) {
                    KeyAction::Pass => {}
                    KeyAction::Consumed => return,
                    KeyAction::Quit => {
                        *quit = true;
                        return;
                    }
                }
            }
            let mouse = match &ev {
                Event::Mouse(m) => Some(*m),
                _ => None,
            };
            let scroll_before = hub.session.state().view.scroll;
            let msgs = terminal_msgs(hub.session.state(), ev);
            if !msgs.is_empty() {
                let mut all = vec![Msg::Tick { now_ms: now_ms() }];
                all.extend(msgs);
                dispatch_local(hub, all, quit, "terminal");
            }
            if let Some(m) = mouse {
                let state = hub.session.state();
                let scrolled = state.view.scroll != scroll_before;
                auto.mouse(state, m, scrolled, Instant::now());
            }
        }
    };

    // Repaints are coalesced to one per refresh: time is cut into slots of `gap` on a fixed
    // grid (like a display's refresh), and a change paints at most once a slot, at once if
    // this slot hasn't painted yet, else at the next slot's start. Input is applied as it
    // arrives, so a fast client never waits for the terminal. A fixed grid, rather than a gap
    // after each paint, keeps frames that arrive at the display rate with a little jitter
    // from colliding.
    let gap = if pacing.max_fps > 0 {
        Duration::from_secs_f64(1.0 / pacing.max_fps as f64)
    } else {
        Duration::ZERO
    };
    let epoch = Instant::now();
    let slot = |t: Instant| {
        if gap.is_zero() {
            0
        } else {
            (t - epoch).as_nanos() / gap.as_nanos()
        }
    };
    let mut drawn: Option<(
        u64,
        u64,
        Option<usize>,
        Option<caretline_layers::kitty::CellPx>,
    )> = None;
    let mut painted_slot: Option<u128> = None;
    // The frame clock's next deadline, on an absolute schedule so it doesn't drift.
    let mut next_frame: Option<Instant> = None;
    while !quit {
        let now = Instant::now();
        match hub.session.state().view.frame_rate() {
            Some(fps) => {
                let period = Duration::from_secs_f64(1.0 / fps as f64);
                let due = *next_frame.get_or_insert(now);
                if now >= due {
                    stats.frames += 1;
                    dispatch_local(
                        hub,
                        vec![Msg::Frame { now_ms: now_ms() }],
                        &mut quit,
                        "runtime",
                    );
                    // Behind by more than a frame (a stall): skip ahead instead of bursting.
                    let next = due + period;
                    next_frame = Some(if next <= now { now + period } else { next });
                }
            }
            None => next_frame = None,
        }
        auto.poll(hub, now, &mut quit);
        let demo_wake = demo.as_mut().and_then(|d| d.poll(hub, now));
        let key = |hub: &Hub, demo: &Option<Box<dyn Demo>>, help: Option<usize>| {
            (
                hub.session.rev(),
                demo.as_ref().map_or(0, |d| d.generation()),
                help,
                gfx.cell_px,
            )
        };
        let dirty = drawn != Some(key(hub, &demo, help));
        let free = gap.is_zero() || painted_slot != Some(slot(now));
        if dirty && free {
            let t = Instant::now();
            let mut frame = match &demo {
                Some(d) => match d.overlay() {
                    Some(f) => f,
                    None => compose(
                        hub,
                        term.map_or(0, |(_, h)| d.pane_rows(hub, h).min(h.saturating_sub(2))),
                    ),
                },
                None => hub.session.frame(),
            };
            let decor = match (&mut demo, help) {
                // The keys overlay hides the demo's images too.
                (Some(d), Some(_)) => Decor {
                    dim: Vec::new(),
                    bytes: d.hide(),
                },
                // Pixels when the probe allowed them, at the cell size in the state.
                (Some(d), None) => {
                    let shown = Gfx {
                        cell_px: gfx.cell_px.and(
                            frame
                                .cell_px
                                .map(|c| caretline_layers::kitty::CellPx::new(c.w, c.h)),
                        ),
                        ..gfx.clone()
                    };
                    d.decorate(hub, &mut frame, &shown)
                }
                (None, _) => Decor::default(),
            };
            if let Some(offset) = help {
                crate::keys::overlay(
                    &mut frame,
                    hub.session.state().doc.outline.is_some(),
                    offset,
                );
            }
            draw(&mut terminal, &frame, &decor)?;
            stats.paints += 1;
            stats.paint_time += t.elapsed();
            drawn = Some(key(hub, &demo, help));
            painted_slot = Some(slot(t));
        }
        let paint_at = painted_slot.map(|k| epoch + gap * (k + 1) as u32);
        // Sleep until input, the next repaint a pending change is waiting for, the next
        // clock frame or the demo's next step, whichever is first.
        let dirty = drawn != Some(key(hub, &demo, help));
        let wake = [
            dirty.then_some(paint_at).flatten(),
            next_frame,
            demo_wake,
            auto.next,
        ]
        .into_iter()
        .flatten()
        .min();
        let input = match wake {
            Some(t) => match rx.recv_timeout(t.saturating_duration_since(Instant::now())) {
                Ok(input) => Some(input),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(input) => Some(input),
                Err(_) => break,
            },
        };
        if let Some(input) = input {
            stats.inputs += 1;
            process(
                hub, &mut auto, &mut demo, &mut term, &mut help, &mut gfx, input, &mut quit,
            );
            // Apply everything already queued before drawing again.
            while !quit {
                match rx.try_recv() {
                    Ok(input) => {
                        stats.inputs += 1;
                        process(
                            hub, &mut auto, &mut demo, &mut term, &mut help, &mut gfx, input,
                            &mut quit,
                        )
                    }
                    Err(_) => break,
                }
            }
            if demo.is_some() {
                fit_views(hub, &demo, term, &mut quit);
            }
            if let Some(d) = &mut demo {
                d.after(hub);
            }
        }
    }
    // Take the demo's images down by id before the screen goes.
    if let Some(d) = &mut demo {
        let bytes = d.hide();
        if !bytes.is_empty() {
            let mut out = io::stdout();
            let _ = out.write_all(&bytes).and_then(|_| out.flush());
        }
    }
    Ok(())
}

/// Draws a frame. ratatui diffs it against the last one and writes only the cells that
/// changed; the writes go out as one buffered flush inside a synchronized update (DEC mode
/// 2026), so a terminal that supports it shows the whole frame at once, never half of one.
///
/// A demo's decor dims cells and adds bytes (kitty graphics) after the cells, inside the same
/// update, so text and pixels change together.
fn draw(terminal: &mut Term, frame: &Frame, decor: &Decor) -> Result<(), String> {
    let _ = queue!(terminal.backend_mut(), BeginSynchronizedUpdate);
    let drawn = terminal
        .draw(|f| {
            let area = f.area();
            let buf = f.buffer_mut();
            for y in 0..frame.height.min(area.height) {
                for x in 0..frame.width.min(area.width) {
                    let cell = frame.cell(x, y);
                    if cell.symbol.is_empty() {
                        continue;
                    }
                    let w = caretline::view::display_width(&cell.symbol).max(1);
                    let mut st = style_in(frame, cell.role);
                    if decor
                        .dim
                        .get(y as usize * frame.width as usize + x as usize)
                        == Some(&true)
                    {
                        st = st.add_modifier(Modifier::DIM);
                    }
                    buf.set_stringn(x, y, &cell.symbol, w, st);
                }
            }
            if let Some((x, y)) = frame.cursor
                && x < area.width
                && y < area.height
            {
                f.set_cursor_position((x, y));
            }
        })
        .map(|_| ())
        .map_err(|e| format!("draw: {e}"));
    if !decor.bytes.is_empty() {
        let _ = terminal.backend_mut().write_all(&decor.bytes);
    }
    let _ = execute!(terminal.backend_mut(), EndSynchronizedUpdate);
    drawn
}
