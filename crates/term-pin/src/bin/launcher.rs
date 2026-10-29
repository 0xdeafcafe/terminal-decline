//! The app's `Contents/MacOS/terminal-decline` (docs/design/pins.md §2.2).

fn main() {
    std::process::exit(term_pin::launcher::run());
}
