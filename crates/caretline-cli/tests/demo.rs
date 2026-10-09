//! `caretline demo`: each demo starts, draws its first frame headless, and runs live on a
//! pseudo-terminal; the agent's writes are guarded and the person's undo spares them.

mod common;

use std::path::Path;
use std::process::Command;

use common::{Pty, Scratch};

const ROWS: u16 = 30;
const COLS: u16 = 90;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_caretline"))
}

fn run(args: &[&str]) -> String {
    let out = bin().args(args).output().expect("caretline runs");
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// A scratch directory for one test (TMPDIR for sockets, and the demo's files).
fn scratch(name: &str) -> Scratch {
    Scratch::new(&format!("demo-{name}"))
}

#[test]
fn demo_help_lists_the_demos() {
    let help = run(&["demo", "--help"]);
    for d in ["welcome", "tour", "scenes", "agent", "layers", "showcase"] {
        assert!(help.contains(&format!("  {d} ")), "{d} missing:\n{help}");
    }
}

/// Compares with `tests/goldens/<name>`; `CARETLINE_GOLDENS=update` rewrites it.
fn golden(name: &str, actual: &str) {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/goldens")
        .join(name);
    if std::env::var("CARETLINE_GOLDENS").as_deref() == Ok("update") {
        std::fs::write(&p, actual).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&p).unwrap_or_else(|_| {
        panic!("no golden {name}: run with CARETLINE_GOLDENS=update and review it")
    });
    assert!(
        want == actual,
        "golden {name} differs.\n--- want\n{want}\n--- got\n{actual}"
    );
}

#[test]
fn the_layers_demo_draws_in_cells_headless() {
    let cases: &[(&str, &[&str])] = &[
        ("demo-layers.80x24.txt", &["--snapshot", "80x24"]),
        ("demo-layers.44x16.txt", &["--snapshot", "44x16"]),
        // Scrolled twelve rows with the spotlight off: the hint follows its word.
        (
            "demo-layers.scrolled.60x20.txt",
            &[
                "--snapshot",
                "60x20",
                "--keys",
                "<down><down><down><down><down><down><down><down><down><down><down><down>s",
            ],
        ),
    ];
    for (name, args) in cases {
        let mut all = vec!["demo", "layers"];
        all.extend_from_slice(args);
        golden(name, &run(&all));
    }
    let ansi = run(&["demo", "layers", "--snapshot", "80x24", "--format", "ansi"]);
    assert!(ansi.contains("Jump by word") && ansi.contains("\x1b["));
}

#[test]
fn each_demo_draws_its_first_frame_headless() {
    let welcome = run(&["demo", "--snapshot", "80x24"]);
    assert!(welcome.contains("Start here"), "{welcome}");
    assert_eq!(welcome, run(&["demo", "welcome", "--snapshot", "80x24"]));

    let tour = run(&["demo", "tour", "--snapshot", "80x24"]);
    assert!(tour.contains("Welcome to caretline"), "{tour}");
    assert!(
        tour.lines().last().unwrap().contains("1/11 · "),
        "the hint names step 1:\n{tour}"
    );
    assert!(!tour.contains("[ ]"), "no task syntax in the tour");

    let agent = run(&["demo", "agent", "--snapshot", "80x30"]);
    assert_eq!(agent.lines().count(), 30);
    assert!(
        agent.contains("## Yours") || agent.contains("Yours"),
        "{agent}"
    );
    assert!(
        agent.contains("agent · connecting"),
        "the pane's status bar:\n{agent}"
    );

    let scenes = run(&["demo", "scenes", "--snapshot", "80x24"]);
    assert!(scenes.contains("C A R E T L I N E"), "{scenes}");
    assert!(scenes.contains("warp · 1/6"), "{scenes}");
    let donut = run(&["demo", "scenes", "--scene", "donut", "--snapshot", "60x20"]);
    assert!(donut.contains("donut · 1/1"), "{donut}");
    assert!(run(&["demo", "--snapshot", "80x24", "--format", "ansi"]).contains("\x1b["));
}

#[test]
fn the_agent_writes_with_if_rev_and_undo_spares_it() {
    let tmp = scratch("headless");
    let out = bin()
        .args(["demo", "agent", "--headless"])
        .env("TMPDIR", &*tmp)
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stderr)));
    assert!(out.status.success(), "{report:#}");
    assert_eq!(report["ok"], true);
    assert!(
        report["agent"]["stale_retried"].as_u64().unwrap() > 0,
        "the person's keys forced stale writes"
    );
    assert_eq!(report["agent"]["writes"], report["agent"]["chars"]);
    assert_eq!(report["undo_kept_only_the_agents_text"], true);
    assert!(
        report["text"]
            .as_str()
            .unwrap()
            .contains("Draft the release notes")
    );
}

fn spawn_demo(args: &[&str], tmp: &Path) -> Pty {
    let mut c = bin();
    c.args(args).env("TMPDIR", tmp);
    Pty::spawn(c, ROWS, COLS)
}

#[test]
fn the_tour_runs_live_dumps_and_replays() {
    let tmp = scratch("tour");
    let dir = tmp.join("files");
    let mut pty = spawn_demo(&["demo", "tour", "--dir", dir.to_str().unwrap()], &tmp);
    pty.wait_for(20, "the first hint", |s| s.contains("1/11 · "));
    pty.send(b" and more");
    pty.wait_for(20, "typing", |s| s.contains("and more"));
    pty.send(b"\x04"); // Ctrl-D
    pty.wait_for(20, "the dump", |s| s.contains("state.json · "));
    let dumped = std::fs::read_to_string(dir.join("state.json")).unwrap();
    assert!(dumped.contains("and more"));
    pty.send(b"\x10"); // Ctrl-P
    pty.wait_for(20, "the replay to finish", |s| {
        s.contains("replayed") && s.contains("identical state")
    });
    pty.send(b"\x11\x11"); // Ctrl-Q twice: leave without saving
    assert!(pty.exited(), "quit");
}

/// The welcome page: one golden per kind of line (a chapter, a command to take away).
#[test]
fn the_welcome_page_draws_in_cells_headless() {
    golden(
        "demo-welcome.80x24.txt",
        &run(&["demo", "--snapshot", "80x24"]),
    );
    golden(
        "demo-welcome.command.100x30.txt",
        &run(&["demo", "--snapshot", "100x30", "--keys", "<up><up><up>"]),
    );
}

/// Enter on a chapter runs it, and quitting the chapter comes back to the page, ticked.
#[test]
fn the_welcome_runs_a_chapter_and_comes_back() {
    let tmp = scratch("welcome");
    let dir = tmp.join("files");
    let mut pty = spawn_demo(&["demo", "--dir", dir.to_str().unwrap()], &tmp);
    pty.wait_for(20, "the page", |s| {
        s.contains("Start here") && s.contains("Watch it work")
    });
    pty.send(b"\x1b[B"); // Down: the tour
    pty.wait_for(20, "the tour chosen", |s| s.contains("Hands on"));
    pty.send(b"\r");
    pty.wait_for(20, "the tour", |s| s.contains("1/11 · "));
    pty.send(b"\x11"); // Ctrl-Q: nothing typed, so it quits at once
    // Back on the page with the next chapter chosen (the tick is a unit test: the
    // callout may cover it at this size).
    pty.wait_for(20, "the page again, the next chapter chosen", |s| {
        s.contains("Start here") && s.contains("You and an agent")
    });
    pty.send(b"q");
    assert!(pty.exited(), "quit");
}

#[test]
fn the_agent_demo_types_beside_the_person() {
    let tmp = scratch("agent");
    let dir = tmp.join("files");
    let mut pty = spawn_demo(&["demo", "agent", "--dir", dir.to_str().unwrap()], &tmp);
    pty.wait_for(20, "the editor", |s| s.contains("Yours"));
    pty.send(b"mine");
    pty.wait_for(20, "the agent's typing in its pane", |s| {
        s.contains("Hello! You're in")
    });
    let s = pty.wait_for(20, "the person's text", |s| s.contains("mine"));
    assert!(
        s.contains("agent · "),
        "the pane's status names the agent:\n{s}"
    );
    pty.send(b"\x11\x11");
    assert!(pty.exited(), "quit");
}

#[test]
fn the_layers_demo_runs_live_with_visible_controls_and_help() {
    let tmp = scratch("layers");
    let mut c = bin();
    c.args(["demo", "layers"])
        .env("TMPDIR", tmp.as_ref())
        .env("CARETLINE_LAYERS", "cells");
    let mut pty = Pty::spawn(c, 24, 80);
    let first = pty.wait("the hint and controls", |s| {
        s.contains("Jump by word") && s.contains("t transport")
    });
    assert!(
        first.contains("q quit") && first.contains("p mode"),
        "{first}"
    );
    assert!(first.contains("spot:on"), "{first}");

    pty.send(b"s");
    pty.wait("spotlight off", |s| s.contains("spot:off"));
    pty.send(b"\x1bOP"); // F1: the keys overlay covers the layers.
    pty.wait("keys overlay", |s| {
        s.contains("Keys") && s.contains("closes")
    });
    pty.send(b"\x1b"); // Esc closes help, not the demo.
    pty.wait("the layers restored", |s| {
        s.contains("Jump by word") && s.contains("spot:off") && !s.contains("closes")
    });
    pty.send(b"\x1b[B\x1b[B"); // Scroll: the hint follows its anchor.
    pty.wait("the view scrolled", |s| {
        !s.contains("Welcome to caretline") && s.contains("Jump by word")
    });
    pty.send(b"s");
    pty.wait("spotlight back on", |s| s.contains("spot:on"));
    pty.send(b"q");
    assert!(pty.exited(), "q quits");
}

#[test]
fn the_showcase_draws_every_slide_and_checks_its_real_state() {
    golden(
        "demo-showcase.intro.80x24.txt",
        &run(&["demo", "showcase", "--snapshot", "80x24"]),
    );
    for slide in 1..=12 {
        golden(
            &format!("demo-showcase.{slide}.100x30.txt"),
            &run(&[
                "demo",
                "showcase",
                "--snapshot",
                "100x30",
                "--keys",
                &format!("1{}<wait:14000>", "<right>".repeat(slide - 1)),
            ]),
        );
    }
    golden(
        "demo-showcase.compact.44x16.txt",
        &run(&[
            "demo",
            "showcase",
            "--snapshot",
            "44x16",
            "--keys",
            "4<wait:7000>",
        ]),
    );
    let report: serde_json::Value =
        serde_json::from_str(&run(&["demo", "showcase", "--headless"])).unwrap();
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["slides"], 12);
    assert_eq!(report["checks"].as_object().unwrap().len(), 13);
    assert!(
        report["checks"]
            .as_object()
            .unwrap()
            .values()
            .all(|v| *v == true)
    );
    for seconds in ["0", "nan", "301"] {
        assert!(
            !bin()
                .args(["demo", "showcase", "--seconds", seconds, "--headless"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}

#[test]
fn the_showcase_advances_itself_and_holds_the_verified_final_slide() {
    let tmp = scratch("showcase-auto");
    let socket = tmp.join("show.sock");
    let mut c = bin();
    c.args([
        "demo",
        "showcase",
        "--seconds",
        "2",
        "--socket",
        socket.to_str().unwrap(),
    ])
    .env("TMPDIR", tmp.as_ref())
    .env("CARETLINE_LAYERS", "cells");
    let mut pty = Pty::spawn(c, 30, 100);
    pty.wait("the complete timed presentation", |s| {
        s.contains("DONE") && s.contains("13/13 checks verified") && s.contains("C A R E T L I N E")
    });
    pty.send(b"q");
    assert!(pty.exited());
}

#[test]
fn the_showcase_navigates_pauses_and_accepts_real_socket_updates() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let tmp = scratch("showcase");
    let socket = tmp.join("show.sock");
    let mut c = bin();
    c.args([
        "demo",
        "showcase",
        "--seconds",
        "2",
        "--socket",
        socket.to_str().unwrap(),
    ])
    .env("TMPDIR", tmp.as_ref())
    .env("CARETLINE_LAYERS", "cells");
    let mut pty = Pty::spawn(c, 30, 100);
    pty.wait("the intro", |s| {
        s.contains("Meet Caretline") && s.contains("AUTO")
    });
    pty.send(b" ");
    pty.wait("paused intro", |s| s.contains("PAUSED"));
    pty.send(b"\x1b[200~\nCORRUPTED SLIDE\n\x1b[201~a");
    let intact = pty.wait("paste consumed before manual playback resumes", |s| {
        s.contains("MANUAL")
    });
    assert!(!intact.contains("CORRUPTED SLIDE"), "{intact}");
    let mut stream = UnixStream::connect(&socket).unwrap();
    stream.set_read_timeout(Some(common::patience(5))).unwrap();
    stream.write_all(b"{\"op\":\"subscribe\"}\n").unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("subscribed"), "{line}");

    pty.send(b"\x1b[C"); // Right: slide 2, manual transitions, live animation.
    pty.wait("the message stream", |s| {
        s.contains("Live message stream") && s.contains("Hello")
    });
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        if event["state_set"] == true {
            assert_eq!(event["source"], "showcase");
            break;
        }
    }
    pty.send(b" ");
    pty.wait("paused", |s| s.contains("PAUSED"));
    let reply = bin()
        .args([
            "send",
            "--socket",
            socket.to_str().unwrap(),
            "--view",
            "0",
            "keys",
            " + outside",
        ])
        .output()
        .unwrap();
    assert!(
        reply.status.success(),
        "{}",
        String::from_utf8_lossy(&reply.stderr)
    );
    pty.wait("a write from the command line", |s| s.contains("outside"));
    pty.send(b"r");
    pty.wait("restarted current slide", |s| {
        s.contains("MANUAL") && !s.contains("outside")
    });
    pty.send(b"4");
    pty.wait("multiple overlays", |s| {
        s.contains("Overlay choreography") && s.contains("LATENCY")
    });
    pty.send(b"\x1b[D");
    pty.wait("previous slide", |s| s.contains("Many carets, exact undo"));
    pty.send(b"q");
    assert!(pty.exited(), "q quits and closes the listener");
    common::eventually("socket cleanup", || !socket.exists());
}

#[test]
fn the_scenes_play_in_the_editor() {
    let tmp = scratch("scenes");
    let mut pty = spawn_demo(&["demo", "scenes"], &tmp);
    pty.wait_for(20, "the warp field", |s| {
        s.contains("C A R E T L I N E") && s.contains("warp · 1/6")
    });
    pty.send(b"\x1b[C"); // right: the next scene
    pty.wait_for(20, "the donut", |s| s.contains("donut · 2/6"));
    pty.send(b"q");
    assert!(pty.exited(), "q quits");
}
