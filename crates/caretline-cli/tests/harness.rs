//! The key-to-screen goldens: each file in `tests/goldens/harness/` is one bug seen by hand or
//! found by the harness fuzz. Its first line is the `caretline sim` command that shows it (paste
//! it to see the case), the rest is what that command prints: the state after each key and the
//! emulated screen, checked against the state after every key. `CARETLINE_GOLDENS=update`
//! rewrites the files from their first lines.

use std::path::Path;
use std::process::Command;

/// Splits a command line on spaces, keeping 'single-quoted' words whole.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut any) = (false, false);
    for c in line.chars() {
        match c {
            '\'' => (quoted, any) = (!quoted, true),
            ' ' if !quoted => {
                if any {
                    out.push(std::mem::take(&mut word));
                }
                any = false;
            }
            c => (word, any) = (word + &c.to_string(), true),
        }
    }
    if any {
        out.push(word);
    }
    out
}

#[test]
fn every_harness_golden_replays() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens/harness");
    let update = std::env::var("CARETLINE_GOLDENS").as_deref() == Ok("update");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let file = std::fs::read_to_string(&path).unwrap();
        let (cmd, want) = file.split_once('\n').unwrap_or((&file, ""));
        let args = words(cmd);
        assert_eq!(
            args[..2],
            ["caretline", "sim"],
            "{name}: starts with caretline sim"
        );
        let out = Command::new(env!("CARGO_BIN_EXE_caretline"))
            .args(&args[1..])
            .output()
            .unwrap();
        let got = String::from_utf8(out.stdout).unwrap();
        assert!(
            out.status.success(),
            "{name}: {}\n{got}",
            String::from_utf8_lossy(&out.stderr)
        );
        if update {
            std::fs::write(&path, format!("{cmd}\n{got}")).unwrap();
        } else {
            assert!(
                want == got,
                "golden {name} differs (CARETLINE_GOLDENS=update rewrites it).\n--- want\n{want}\n--- got\n{got}"
            );
        }
        seen += 1;
    }
    assert!(seen >= 8, "only {seen} harness goldens");
}
