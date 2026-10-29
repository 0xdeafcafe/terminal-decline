//! The static `terminal-decline` binary (`cargo run`, `cargo install`):
//! everything is in the library, started in the bare role. The app bundle
//! runs the same library through its launcher instead (crates/term-dylib).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(terminal_decline_app::run(terminal_decline_app::Role::Bare, args));
}
