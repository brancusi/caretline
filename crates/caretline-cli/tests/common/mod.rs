//! Shared test helpers for the binary: scratch directories and child processes that clean up
//! after themselves (on success, on a failed assertion and on an early return), a pseudo-
//! terminal that waits on output as it arrives, and deadlines that stretch under load.
//!
//! A child never inherits the test's stdout or stderr (a leftover child would hold a
//! `cargo test | …` pipe open), and every child is tied to the test process: an editor on a
//! pseudo-terminal gets SIGHUP when the test's end of it closes, a `serve` on stdio sees its
//! stdin end, and a `serve --socket` runs with `--exit-with-parent`. `Drop` only covers a
//! test that unwinds; those cover a test process that is killed.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::ops::Deref;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// How much longer than its nominal time a test waits: `CARETLINE_TEST_TIMEOUT_SCALE`
/// (default 6, so a nominal 10 s is a minute: room for a loaded machine, still a failure
/// when something hangs).
pub fn scale() -> f64 {
    std::env::var("CARETLINE_TEST_TIMEOUT_SCALE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| *s > 0.0)
        .unwrap_or(6.0)
}

/// `secs` of nominal time, scaled.
pub fn patience(secs: u64) -> Duration {
    Duration::from_secs_f64(secs as f64 * scale())
}

/// Waits until `f` holds, for a scaled 10 s.
pub fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + patience(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A directory removed when dropped. Short (under /tmp), since socket paths must be.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        static SWEEP: std::sync::Once = std::sync::Once::new();
        SWEEP.call_once(sweep);
        let dir = PathBuf::from("/tmp").join(format!("clt-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

/// Removes the scratch directories of test processes that are gone: `Drop` doesn't run when
/// a test process is killed. (`clm-` and `clp-` are these tests' earlier names.)
fn sweep() {
    let Ok(entries) = std::fs::read_dir("/tmp") else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(rest) = name.to_str().and_then(|n| {
            ["clt-", "clm-", "clp-"]
                .iter()
                .find_map(|p| n.strip_prefix(p))
        }) else {
            continue;
        };
        let Some(pid) = rest.split('-').next().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        if pid != std::process::id() && !alive(pid) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

impl Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Output a child wrote, collected by a thread.
#[derive(Clone, Default)]
pub struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub fn drain(&self, mut r: impl Read + Send + 'static) {
        let o = self.0.clone();
        std::thread::spawn(move || {
            let mut b = [0u8; 8192];
            while let Ok(n) = r.read(&mut b) {
                if n == 0 {
                    break;
                }
                o.lock().unwrap().extend_from_slice(&b[..n]);
            }
        });
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// A child killed and reaped when dropped. Its stdout and stderr go to pipes the test owns
/// (shown if the test fails); its stdin is null unless the command set it.
pub struct Proc {
    pub child: Child,
    pub stderr: Captured,
    pub stdout: Captured,
}

impl Proc {
    pub fn spawn(c: &mut Command) -> Proc {
        let mut child = c
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (stdout, stderr) = (Captured::default(), Captured::default());
        stdout.drain(child.stdout.take().unwrap());
        stderr.drain(child.stderr.take().unwrap());
        Proc {
            child,
            stderr,
            stdout,
        }
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            let err = self.stderr.text();
            if !err.is_empty() {
                eprintln!("child {} stderr:\n{err}", self.child.id());
            }
        }
    }
}

/// Whether a process with this pid exists (a zombie counts).
pub fn alive(pid: u32) -> bool {
    let r = unsafe { libc::kill(pid as i32, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Marks an fd close-on-exec, so children spawned later (by any test thread) don't inherit
/// it: a child holding a pseudo-terminal's master keeps the editor on it from ever seeing
/// the hang-up.
fn cloexec(fd: i32) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
    }
}

#[derive(Default)]
struct Out {
    bytes: Vec<u8>,
    closed: bool,
}

/// A child on a pseudo-terminal, in its own session. When the test's end closes (dropped,
/// or the test process dies), the child gets SIGHUP. Dropping also kills and reaps it.
pub struct Pty {
    pub master: std::fs::File,
    out: Arc<(Mutex<Out>, Condvar)>,
    pub child: Child,
    rows: u16,
    cols: u16,
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Pty {
    pub fn spawn(mut c: Command, rows: u16, cols: u16) -> Pty {
        let (mut m, mut s) = (0, 0);
        let mut ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut m,
                    &mut s,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut ws,
                )
            },
            0
        );
        cloexec(m);
        cloexec(s);
        let slave = unsafe { OwnedFd::from_raw_fd(s) };
        c.env("TERM", "xterm-256color");
        let sfd = slave.as_raw_fd();
        c.stdin(slave.try_clone().unwrap())
            .stdout(slave.try_clone().unwrap())
            .stderr(slave);
        unsafe {
            c.pre_exec(move || {
                libc::setsid();
                libc::ioctl(sfd, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        let child = c.spawn().unwrap();
        drop(c); // the slave's last copy in this process
        let master = unsafe { std::fs::File::from_raw_fd(m) };
        let out = Arc::new((Mutex::new(Out::default()), Condvar::new()));
        let (mut r, o) = (master.try_clone().unwrap(), out.clone());
        let mut w = master.try_clone().unwrap();
        std::thread::spawn(move || {
            let mut b = [0u8; 65536];
            loop {
                let n = r.read(&mut b).unwrap_or(0);
                let (lock, cv) = &*o;
                let mut g = lock.lock().unwrap();
                if n == 0 {
                    g.closed = true;
                    cv.notify_all();
                    break;
                }
                // Answer the keyboard-protocol query (no kitty support) so start-up is quick.
                if b[..n].windows(4).any(|x| x == b"\x1b[?u") {
                    let _ = w.write_all(b"\x1b[?62;22c");
                }
                g.bytes.extend_from_slice(&b[..n]);
                cv.notify_all();
            }
        });
        Pty {
            master,
            out,
            child,
            rows,
            cols,
        }
    }

    pub fn send(&mut self, b: &[u8]) {
        self.master.write_all(b).unwrap();
    }

    pub fn screen(&self) -> String {
        let g = self.out.0.lock().unwrap();
        screen(&g.bytes, self.rows as usize, self.cols as usize).join("\n")
    }

    /// Waits for the screen to satisfy `ok`, checking each time output arrives, for a scaled
    /// `secs`. Fails at once if the child closes the terminal first.
    pub fn wait_for(&self, secs: u64, what: &str, ok: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + patience(secs);
        let (lock, cv) = &*self.out;
        let mut g = lock.lock().unwrap();
        loop {
            let s = screen(&g.bytes, self.rows as usize, self.cols as usize).join("\n");
            if ok(&s) {
                return s;
            }
            assert!(
                !g.closed,
                "the terminal closed waiting for {what}; screen:\n{s}"
            );
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for {what}; screen:\n{s}");
            g = cv.wait_timeout(g, deadline - now).unwrap().0;
        }
    }

    pub fn wait(&self, what: &str, ok: impl Fn(&str) -> bool) -> String {
        self.wait_for(10, what, ok)
    }

    /// Waits for the child to exit; whether it exited successfully.
    pub fn exited(&mut self) -> bool {
        let deadline = Instant::now() + patience(10);
        while Instant::now() < deadline {
            if let Ok(Some(s)) = self.child.try_wait() {
                return s.success();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

/// The screen a byte stream draws: cursor moves (`CSI r;c H`), clears and text.
pub fn screen(out: &[u8], rows: usize, cols: usize) -> Vec<String> {
    let mut grid = vec![vec![' '; cols]; rows];
    let (mut r, mut c) = (0usize, 0usize);
    let s = String::from_utf8_lossy(out);
    let mut it = s.chars().peekable();
    while let Some(ch) = it.next() {
        match ch {
            '\x1b' => match it.next() {
                Some('[') => {
                    let mut params = String::new();
                    while let Some(&n) = it.peek() {
                        it.next();
                        if ('@'..='~').contains(&n) {
                            if n == 'H' {
                                let mut p = params
                                    .trim_start_matches('?')
                                    .split(';')
                                    .map(|x| x.parse::<usize>().unwrap_or(1));
                                r = p.next().unwrap_or(1).saturating_sub(1);
                                c = p.next().unwrap_or(1).saturating_sub(1);
                            } else if n == 'J' && params == "2" {
                                grid = vec![vec![' '; cols]; rows];
                            }
                            break;
                        }
                        params.push(n);
                    }
                }
                Some(']') => {
                    while let Some(n) = it.next() {
                        if n == '\x07' || (n == '\x1b' && it.peek() == Some(&'\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => c = 0,
            '\n' => r += 1,
            ch if ch >= ' ' => {
                if r < rows && c < cols {
                    grid[r][c] = ch;
                }
                c += 1;
            }
            _ => {}
        }
    }
    grid.into_iter().map(|l| l.into_iter().collect()).collect()
}
