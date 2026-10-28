//! Replays the committed fuzz inputs (`fuzz/corpus/parse`, the seed corpus,
//! and `fuzz/regressions/parse`, crashes the fuzzer found) through `Term` on
//! stable Rust, so every PR runs them without cargo-fuzz.
//!
//! The input format is shared with the fuzz target (`fuzz/steps.rs`). The
//! terminal invariants are checked by `Term::advance` itself in debug builds.

use std::fs;
use std::path::{Path, PathBuf};

#[path = "../../../fuzz/steps.rs"]
mod steps;

fn inputs(dir: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz").join(dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file() && !p.file_name().unwrap().to_string_lossy().starts_with('.'))
        .filter(|p| p.extension().is_none_or(|e| e != "md"))
        .collect();
    files.sort();
    files
}

fn replay(path: &Path) {
    let data = fs::read(path).unwrap();
    let mut term = steps::new_term();
    for step in steps::decode(&data) {
        steps::apply(&mut term, &step);
        steps::read_back(&term);
    }
}

fn replay_all(files: &[PathBuf]) {
    let mut failed = Vec::new();
    for path in files {
        if std::panic::catch_unwind(|| replay(path)).is_err() {
            failed.push(path.display().to_string());
        }
    }
    assert!(failed.is_empty(), "inputs that panicked:\n  {}", failed.join("\n  "));
}

#[test]
fn seed_corpus() {
    let files = inputs("corpus/parse");
    assert!(!files.is_empty(), "no seed corpus found in fuzz/corpus/parse");
    replay_all(&files);
}

#[test]
fn regressions() {
    replay_all(&inputs("regressions/parse"));
}

#[test]
fn decode_format() {
    use steps::Step;
    let steps = steps::decode(b"ab\xffR\x4f\x17cd\xffV\xfe\xffC\xffS\x01\x02\x03\x04\xff\xff\xffX\xffR\x01");
    let shown: Vec<String> = steps.iter().map(|s| format!("{s:?}")).collect();
    assert_eq!(
        shown,
        [
            "Bytes(b\"ab\")",
            "Resize(80, 24)",
            "Bytes(b\"cd\")",
            "ScrollView(-2)",
            "ClearSelection",
            "Select(1, 2, 3, 4)",
            "Bytes(b\"\\xff\\xff\\xffX\")",
        ]
    );
    // Long runs are split.
    let steps = steps::decode(&[b'x'; steps::MAX_CHUNK * 2 + 1]);
    assert_eq!(steps.len(), 3);
    assert!(matches!(&steps[2], Step::Bytes(b) if b.len() == 1));
}
