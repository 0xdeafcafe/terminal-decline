//! Completion and colouring at the prompt (docs/design/completion.md
//! §11): reading the command line into words, which runs on the main
//! thread on every key (budget: reading and colouring a 200-character
//! line in 20 µs); matching 3,000 commands against a prefix, on the main
//! thread while the list is open; and the man page and `--help` readers,
//! on the worker.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use term_complete::lex::{self, At};
use term_complete::list::{self, Listings};
use term_complete::sources::Commands;
use term_complete::{help, man};

/// A typical line, then the same kind of line grown to `len` bytes.
fn line(len: usize) -> String {
    const PIECES: &[&str] = &[
        "git commit -m \"fix the thing; again\"",
        " && ls -la ~/src/project",
        " | grep -v $HOME",
        " > ./out.txt 2>&1",
        " ; FOO=1 sudo -u root make -j8 all",
        " && echo \"${PATH}\" $(date +%s)",
    ];
    let mut s = String::new();
    while s.len() < len {
        s.push_str(PIECES[s.len() % PIECES.len()]);
    }
    let mut end = len.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s
}

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("lex");
    for len in [20, 200, 4000] {
        let text = line(len);
        g.throughput(Throughput::Bytes(text.len() as u64));
        g.bench_with_input(BenchmarkId::from_parameter(len), &text, |b, text| {
            b.iter(|| black_box(lex::read(black_box(text), text.len())));
        });
    }
    g.finish();

    // 3,000 programs, as on a Mac with Homebrew.
    let names: Vec<String> = (0..3000)
        .map(|i| format!("{}{i}", ["g", "git-", "go", "brew", "x"][i % 5]))
        .collect();
    let commands = Commands::from_names(names);
    let env = term_core::ShellEnv::default();
    let listings = Listings {
        env: Some(&env),
        commands: Some(&commands),
        folder: None,
        flags: None,
        subcommands: None,
        history: None,
    };
    for prefix in ["g", "git-1"] {
        let at = At::Command { prefix: prefix.into() };
        c.bench_function(&format!("offer 3000 commands {prefix:?}"), |b| {
            b.iter(|| black_box(list::offer(black_box(&at), &listings)))
        });
    }

    // Real pages where this Mac has them.
    for page in [
        "/usr/share/man/man1/ls.1",
        "/opt/homebrew/share/man/man1/git-fetch.1",
        "/usr/share/man/man1/curl.1",
    ] {
        let Ok(src) = std::fs::read_to_string(page) else { continue };
        let name = page.rsplit('/').next().unwrap_or(page);
        c.bench_function(&format!("man read {name}"), |b| b.iter(|| black_box(man::read(black_box(&src)))));
    }
    let cargo_help = "Rust's package manager\n\nOptions:\n".to_string()
        + &(0..40)
            .map(|i| {
                format!(
                    "  -{}, --option-{i} <VALUE>   What option {i} does, at some length\n",
                    (b'a' + (i % 26) as u8) as char
                )
            })
            .collect::<String>()
        + "\nCommands:\n"
        + &(0..30)
            .map(|i| format!("    command{i}   What command {i} does\n"))
            .collect::<String>();
    c.bench_function("help read 40 flags 30 commands", |b| {
        b.iter(|| black_box(help::read(black_box(&cargo_help))))
    });
}

/// Past command lines, the marks between commands and plain output
/// coloured (docs/design/command-marks.md), worked out after the
/// snapshot of a 120 × 40 view of eight commands with output: when
/// nothing changed (an idle frame), after a change, and from cold (new
/// caches).
fn marks(c: &mut Criterion) {
    use term_complete::marks::{Marks, Settings};
    let mut t = term_core::Term::new(120, 40, 1000, term_core::ImagePool::process());
    for i in 0..8 {
        let cmd = format!("git log --oneline -n 3 \"$HOME\" | grep -v x{i}");
        t.advance(format!("\x1b]133;A\x07~/src \x1b]133;B\x07{cmd}\r\n").as_bytes());
        t.state.set_clock(i * 1000);
        let enc = cmd.replace('%', "%25").replace(';', "%3B").replace(' ', "%20");
        t.advance(format!("\x1b]133;C;cmdline_url={enc}\x07").as_bytes());
        t.advance(b"3f2a9c1 fix: error when 12% done -> https://example.com/a\r\n");
        t.advance(b"\x1b[33mab12cd3\x1b[0m ok 4.5ms warning: deprecated\r\n");
        t.advance(b"plain words and nothing more to see here at all\r\n");
        t.state.set_clock(i * 1000 + 1500);
        t.advance(format!("\x1b]133;D;{}\x07", i % 2).as_bytes());
    }
    t.advance(b"\x1b]133;A\x07~/src \x1b]133;B\x07");
    let known = |name: &str| Some(name == "git");
    for (name, colour_output, failure_cause) in [
        ("marks", false, false),
        ("marks with output", true, false),
        ("marks with causes", false, true),
    ] {
        let s = Settings {
            past_commands: true,
            command_marks: true,
            colour_output,
            failure_cause,
        };
        let mut snap = term_core::Snapshot::default();
        let mut m = Marks::default();
        let mut frame = |t: &term_core::Term, m: &mut Marks| {
            snap.ink.clear();
            snap.separators.clear();
            snap.command_marks.clear();
            m.update(&t.state, s, &known, &mut snap);
        };
        c.bench_function(&format!("{name} idle"), |b| b.iter(|| frame(&t, &mut m)));
        c.bench_function(&format!("{name} changed"), |b| {
            b.iter(|| {
                t.advance(b" \x08");
                frame(&t, &mut m);
            })
        });
        c.bench_function(&format!("{name} cold"), |b| b.iter(|| frame(&t, &mut Marks::default())));
    }
}

criterion_group!(benches, bench, marks);
criterion_main!(benches);
