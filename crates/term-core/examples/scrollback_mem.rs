//! Bytes per scrollback line for typical output: `cargo run --release -p term-core --example scrollback_mem`
//!
//! The first figure counts everything history holds for 10k lines (the
//! open block's spare capacity included); the sealed one only the sealed
//! blocks, as §11.4 of the cells design counts. With `compressed`: 100k
//! lines of each kind before and after idle compression (R.7).

use term_core::Term;
use term_core::image::placeholder::{PLACEHOLDER, diacritic};

fn measure(name: &str, line: impl Fn(usize) -> String) {
    // Images on, so kitty placeholders are cells.
    let mut t = Term::new(160, 50, 10_000, term_core::ImagePool::new(32 << 20));
    for i in 0..20_000 {
        t.advance(line(i).as_bytes());
        t.advance(b"\r\n");
    }
    let bytes = t.state.scrollback_bytes();
    let h = t.state.history_stats();
    println!(
        "{name:<28} {:>6} bytes/line  ({:.1} MB for 10k lines; sealed blocks {} bytes/line)",
        bytes / t.state.scrollback_len(),
        bytes as f64 / 1e6,
        h.sealed_bytes / h.sealed_lines.max(1),
    );
}

/// 100k lines with a 100k-line limit, then history compressed (R.7):
/// bytes before and after, and the time to decompress a block.
fn compressed(name: &str, line: impl Fn(usize) -> String) {
    let mut t = Term::new(160, 50, 100_000, term_core::ImagePool::new(32 << 20));
    for i in 0..100_050 {
        t.advance(line(i).as_bytes());
        t.advance(b"\r\n");
    }
    let (before, after) = t.state.shrink_history();
    let mut best = f64::MAX;
    let mut blocks = 0;
    for _ in 0..10 {
        let start = std::time::Instant::now();
        blocks = t.state.unpack_history_for_bench();
        best = best.min(start.elapsed().as_secs_f64() * 1e6 / blocks.max(1) as f64);
    }
    println!(
        "{name:<28} 100k lines: {:.3} MB -> {:.3} MB compressed ({blocks} blocks, decode {best:.0} µs a block)",
        before as f64 / 1e6,
        after as f64 / 1e6,
    );
}

/// A line of `ls -l --color`.
fn ls_l(i: usize) -> String {
    let mut r = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        r
    };
    let names = [
        "main",
        "lib",
        "config",
        "README",
        "build",
        "test_utils",
        "mod",
        "parser",
        "index",
        "Cargo",
    ];
    let exts = ["rs", "toml", "md", "json", "lock", "sh", "txt", "o", "d"];
    let dir = next() % 5 == 0;
    let (perm, colour) = if dir { ("drwxr-xr-x", "1;34") } else { ("-rw-r--r--", "0") };
    let name = format!(
        "{}{}",
        names[(next() % 10) as usize],
        if next() % 3 == 0 {
            format!("_{}", next() % 100)
        } else {
            String::new()
        }
    );
    let name = if dir {
        name
    } else {
        format!("{name}.{}", exts[(next() % 9) as usize])
    };
    format!(
        "{perm}  {} lw  staff  {:>8} {} {:>2} {:02}:{:02} \x1b[{colour}m{name}\x1b[0m",
        1 + next() % 30,
        next() % 2_000_000,
        ["Jan", "Feb", "Mar", "Oct", "Nov"][(next() % 5) as usize],
        1 + next() % 28,
        next() % 24,
        next() % 60
    )
}

/// A line of a service's log.
fn log(i: usize) -> String {
    let mut r = (i as u64).wrapping_mul(0xD1B5_4A32_D192_ED03) | 1;
    let mut next = || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        r
    };
    let (level, colour) = [("INFO", "32"), ("INFO", "32"), ("DEBUG", "36"), ("WARN", "33"), ("INFO", "32")][(next() % 5) as usize];
    let module = ["http::server", "db::pool", "auth", "worker::queue", "cache"][(next() % 5) as usize];
    let msg = match next() % 4 {
        0 => format!("GET /api/v1/users/{} 200 {}ms", next() % 100_000, next() % 900),
        1 => format!(
            "connection {} acquired in {}µs (pool size {})",
            next() % 64,
            next() % 5000,
            next() % 32
        ),
        2 => format!("job {:016x} done after {} retries", next(), next() % 4),
        _ => format!("cache miss for key session:{:08x}", next() as u32),
    };
    let ms = i as u64 * 37 + next() % 30;
    format!(
        "2026-10-30T{:02}:{:02}:{:02}.{:03}Z \x1b[{colour}m{level:<5}\x1b[0m \x1b[2m{module}\x1b[0m: {msg}",
        (ms / 3_600_000) % 24,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "file" {
        // Real output: the file's lines, repeated to 100k.
        let text = std::fs::read_to_string(&args[2]).expect("a text file");
        let lines: Vec<&str> = text.lines().collect();
        compressed(&args[2], |i| lines[i % lines.len()].to_string());
        return;
    }
    if args.get(1).is_some_and(|a| a == "compressed") {
        compressed("ls --color (as above)", |i| {
            format!("\x1b[1;34mdir{i}\x1b[0m  \x1b[32mbuild.sh\x1b[0m  notes.txt  \x1b[1;34msrc\x1b[0m  \x1b[31marchive.tar\x1b[0m")
        });
        compressed("ls -l --color", ls_l);
        compressed("service log", log);
        compressed("plain 40 chars", |i| format!("{i:08} the quick brown fox jumps over"));
        return;
    }
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
    measure("placeholder image row", |i| {
        // 40 cells of one image row, as yazi or `kitten icat --unicode-placeholder`
        // send them: the row and column diacritics on the first cell only.
        let mut s = String::from("\x1b[38;2;0;0;42m");
        s.push(PLACEHOLDER);
        s.push(diacritic((i % 297) as u16).unwrap());
        s.push(diacritic(0).unwrap());
        for _ in 1..40 {
            s.push(PLACEHOLDER);
        }
        s.push_str("\x1b[0m");
        s
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
