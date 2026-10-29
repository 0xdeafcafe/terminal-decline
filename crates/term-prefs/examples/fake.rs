//! The preferences TUI with the test host, to try it in any terminal:
//! `cargo run -p term-prefs --example fake -- /tmp/config.toml`.

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let file = if args.first().is_some_and(|a| !a.starts_with("--")) {
        args.remove(0)
    } else {
        "/tmp/term-prefs-config.toml".into()
    };
    let host = term_prefs::Fake::new(file.into());
    std::process::exit(term_prefs::run(&args, &host));
}
