//! Bytes per scrollback line for typical output: `cargo run --release -p term-core --example scrollback_mem`

use term_core::Term;

fn measure(name: &str, line: impl Fn(usize) -> String) {
    let mut t = Term::new(160, 50, 10_000);
    for i in 0..20_000 {
        t.advance(line(i).as_bytes());
        t.advance(b"\r\n");
    }
    let bytes = t.state.scrollback_bytes();
    println!(
        "{name:<28} {:>6} bytes/line  ({:.1} MB for 10k lines)",
        bytes / t.state.scrollback_len(),
        bytes as f64 / 1e6
    );
}

fn main() {
    measure("plain 40 chars", |i| format!("{i:08} the quick brown fox jumps over"));
    measure("ls --color", |i| {
        format!("\x1b[1;34mdir{i}\x1b[0m  \x1b[32mbuild.sh\x1b[0m  notes.txt  \x1b[1;34msrc\x1b[0m  \x1b[31marchive.tar\x1b[0m")
    });
    measure("full-width background", |i| format!("\x1b[48;5;22m+{i:<158}\x1b[0m"));
    measure("compiler error", |i| {
        format!("\x1b[1;31merror[E{i:04}]\x1b[0m\x1b[1m: mismatched types\x1b[0m --> src/main.rs:{i}:5")
    });
    measure("CJK", |i| format!("{i} 漢字かなカナ한글 混合テキスト 中文字符"));
}
