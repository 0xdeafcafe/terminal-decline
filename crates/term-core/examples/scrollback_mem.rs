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
    measure("emoji-heavy", |i| {
        format!(
            "{i:05} \u{2705} done \u{1f44d}\u{1f3fd} ship \u{1f1ec}\u{1f1e7} \u{2764}\u{fe0f} \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} ok"
        )
    });
    measure("gradient 160 cells", |i| {
        let mut s = String::new();
        for c in 0..160 {
            let n = i * 160 + c;
            s.push_str(&format!("\x1b[38;2;{};{};{}m\u{2588}", (n >> 16) as u8, (n >> 8) as u8, n as u8));
        }
        s.push_str("\x1b[0m");
        s
    });
}
