//! Shared test helpers: the caretline binary, a live editor on a pseudo-terminal, and a
//! minimal MCP client over the server's stdio. The pseudo-terminal, child processes, scratch
//! directories and deadlines are caretline-cli's (its tests/common), which say how a child
//! is cleaned up even when the test process is killed.
#![allow(dead_code, unused_imports)]

#[path = "../../../caretline-cli/tests/common/mod.rs"]
mod proc;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Once;
use std::sync::mpsc::{Receiver, RecvTimeoutError};

pub use proc::{Captured, Proc, Pty, Scratch, alive, eventually, patience};
use serde_json::{Value, json};

pub const ROWS: u16 = 12;
pub const COLS: u16 = 60;

/// The `caretline` binary (another package's), built once per test run. `CARETLINE_BIN`
/// overrides it.
pub fn caretline() -> PathBuf {
    static BUILD: Once = Once::new();
    if let Ok(p) = std::env::var("CARETLINE_BIN") {
        return PathBuf::from(p);
    }
    let mcp = PathBuf::from(env!("CARGO_BIN_EXE_caretline-mcp"));
    let bin = mcp.with_file_name("caretline");
    BUILD.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut c = Command::new(cargo);
        c.args(["build", "-q", "-p", "caretline-cli", "--bin", "caretline"])
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        if mcp
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == "release")
        {
            c.arg("--release");
        }
        let st = c.status().expect("cargo build caretline");
        assert!(st.success(), "building caretline failed");
    });
    assert!(bin.exists(), "{} missing", bin.display());
    bin
}

/// A short scratch directory (socket paths must stay short), used as TMPDIR so the editors
/// a test starts are the only ones its server discovers. Removed when dropped.
pub fn scratch(name: &str) -> Scratch {
    Scratch::new(&format!("m-{name}"))
}

/// A live editor on `file` in a pseudo-terminal, listening on `dir/ed.sock`.
pub fn live_editor(dir: &Path, file: &Path) -> Pty {
    let mut c = Command::new(caretline());
    c.arg(file)
        .arg("--listen")
        .arg(dir.join("ed.sock"))
        .arg("--no-mouse")
        .env("TMPDIR", dir)
        .current_dir(dir);
    let pty = Pty::spawn(c, ROWS, COLS);
    pty.wait("the editor", |s| s.contains("listening on"));
    pty
}

/// An MCP client over a child's stdio: JSON-RPC requests, one per line. The server exits
/// when its stdin ends, so it goes with the test process however that ends; dropping this
/// kills and reaps it.
pub struct Mcp {
    pub child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: Captured,
    next: u64,
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking() {
            let err = self.stderr.text();
            if !err.is_empty() {
                eprintln!("caretline-mcp stderr:\n{err}");
            }
        }
    }
}

impl Mcp {
    /// Starts `caretline-mcp ARGS` with TMPDIR = `dir` and runs the initialize handshake.
    pub fn start(dir: &Path, args: &[&str]) -> Mcp {
        let mut child = Command::new(env!("CARGO_BIN_EXE_caretline-mcp"))
            .args(args)
            .env("TMPDIR", dir)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stderr = Captured::default();
        stderr.drain(child.stderr.take().unwrap());
        // Lines arrive on a channel, so a request can give up after a deadline instead of
        // blocking forever on a server that hangs.
        let (tx, lines) = std::sync::mpsc::channel();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut m = Mcp {
            child,
            stdin,
            lines,
            stderr,
            next: 1,
        };
        let init = m.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "test-agent", "version": "0"}}),
        );
        assert_eq!(
            init["result"]["serverInfo"]["name"], "caretline-mcp",
            "{init}"
        );
        m.notify("notifications/initialized", json!({}));
        m
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "method": method, "params": params})
        )
        .unwrap();
    }

    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        // A watch waits up to its own timeout_ms before answering; this is on top.
        let wait = patience(10) + std::time::Duration::from_millis(params_timeout(&params));
        let deadline = std::time::Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    panic!(
                        "no answer to {method} in {wait:?}; stderr:\n{}",
                        self.stderr.text()
                    )
                }
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("the server closed stdout; stderr:\n{}", self.stderr.text())
                }
            };
            let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    /// Calls a tool; returns (is_error, structured result).
    pub fn tool(&mut self, name: &str, args: Value) -> (bool, Value) {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        assert!(r.get("error").is_none(), "{name}: protocol error {r}");
        let res = &r["result"];
        (
            res["isError"] == json!(true),
            res["structuredContent"].clone(),
        )
    }

    /// Calls a tool that must succeed.
    pub fn ok(&mut self, name: &str, args: Value) -> Value {
        let (err, v) = self.tool(name, args.clone());
        assert!(!err, "{name} {args} failed: {v}");
        v
    }
}

/// A tool call's `timeout_ms`, if it has one.
fn params_timeout(params: &Value) -> u64 {
    params["arguments"]["timeout_ms"].as_u64().unwrap_or(0)
}

/// A `watch` timeout of `ms` nominal milliseconds, scaled: for a watch that should see a
/// change, not one that should time out.
pub fn watch_ms(ms: u64) -> u64 {
    patience(1).as_millis() as u64 * ms / 1000
}

/// `caretline serve FILE --socket SOCK`, tied to the test process (`--exit-with-parent`).
pub fn serve(file: &Path, sock: &Path) -> Proc {
    let p = Proc::spawn(
        Command::new(caretline())
            .arg("serve")
            .arg(file)
            .arg("--socket")
            .arg(sock)
            .arg("--exit-with-parent"),
    );
    eventually("the socket", || {
        std::os::unix::net::UnixStream::connect(sock).is_ok()
    });
    p
}
