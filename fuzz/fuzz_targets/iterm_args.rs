//! iTerm2 `File=` arguments (term-image): never panics, and a name kept is
//! within its cap.
//!
//! Run with `cargo +nightly fuzz run iterm_args`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use term_image::iterm::{Args, MAX_NAME, split};

fuzz_target!(|body: &[u8]| {
    let args = match split(body) {
        Some((args, _)) => args,
        None => body,
    };
    let a = Args::parse(args);
    assert!(a.name.as_ref().is_none_or(|n| n.len() <= MAX_NAME));
});
