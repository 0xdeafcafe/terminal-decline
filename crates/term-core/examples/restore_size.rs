//! Saved history for a restart (`restore_scrollback`): bytes for 10k lines
//! and the time to save and restore them.
//! `cargo run --release -p term-core --example restore_size`

use std::time::Instant;

use term_core::Term;

fn measure(name: &str, line: impl Fn(usize) -> String) {
    let mut t = Term::new(160, 50, 10_000, term_core::ImagePool::process());
    for i in 0..10_050 {
        t.advance(line(i).as_bytes());
        t.advance(b"\r\n");
    }
    for compressed in [false, true] {
        if compressed {
            // As it is once history has been idle a while (R.7).
            t.state.shrink_history();
        }
        let mut save = f64::MAX;
        let mut saved = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            saved = t.state.save_history(10_000, 0).unwrap();
            save = save.min(start.elapsed().as_secs_f64() * 1e3);
        }
        let mut load = f64::MAX;
        for _ in 0..5 {
            let mut back = Term::new(160, 50, 10_000, term_core::ImagePool::process());
            let start = Instant::now();
            back.state.restore_history(&saved).unwrap();
            load = load.min(start.elapsed().as_secs_f64() * 1e3);
        }
        println!(
            "{name:<16} {}: {:>4} KB for 10k lines ({:>3} B a line), save {save:.2} ms, restore {load:.2} ms",
            if compressed { "idle-compressed" } else { "fresh          " },
            saved.len() / 1024,
            saved.len() / 10_000,
        );
    }
}

fn main() {
    measure("ls -l --color", |i| {
        format!(
            "-rw-r--r--  1 alex  staff  {:>8} Oct 31 12:{:02} \x1b[1;34mdir{i}\x1b[0m/\x1b[32mfile_{i}.rs\x1b[0m",
            i * 37 % 100_000,
            i % 60
        )
    });
    measure("log", |i| {
        format!(
            "2026-10-31T12:{:02}:{:02}.{:03}Z \x1b[32mINFO\x1b[0m server::http: request id={i:08x} method=GET path=/api/v1/items/{} status=200 took={}ms",
            i / 60 % 60,
            i % 60,
            i % 1000,
            i % 977,
            i % 53
        )
    });
}
