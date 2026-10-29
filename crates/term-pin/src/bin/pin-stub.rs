//! The pin stub, copied into every pin as `Contents/MacOS/<id>` (docs/
//! design/pins.md §3). Frozen: see crates/term-pin/FROZEN.md.

fn main() {
    std::process::exit(term_pin::stub::run());
}
