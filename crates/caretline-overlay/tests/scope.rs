//! caretline-overlay names no host concept either: what a hint says and what a line means
//! are the host's. The same list as the engine's `tests/scope.rs`.

const WORDS: &[&str] = &[
    "task",
    "tasks",
    "todo",
    "done",
    "doing",
    "waiting",
    "cancelled",
    "completed",
    "checkbox",
    "checkboxes",
    "journal",
    "vault",
    "thc",
    "priority",
    "due",
    "note",
    "notes",
];

fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries {
        let p = e.unwrap().path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn the_overlay_names_no_host_concept() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = Vec::new();
    files(&root.join("src"), &mut paths);
    files(&root.join("examples"), &mut paths);
    let mut found = Vec::new();
    for p in paths {
        let text = std::fs::read_to_string(&p).unwrap();
        for (n, line) in text.lines().enumerate() {
            let lower = line.to_lowercase();
            for w in lower.split(|c: char| !c.is_alphanumeric()) {
                if WORDS.contains(&w) {
                    found.push(format!(
                        "{}:{}: {w}: {}",
                        p.strip_prefix(root).unwrap().display(),
                        n + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "host words in caretline-overlay:\n{}",
        found.join("\n")
    );
}
