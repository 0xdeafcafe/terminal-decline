//! kitty graphics control keys (term-image): parsing never panics, and a
//! command that parses keeps its invariants.
//!
//! Run with `cargo +nightly fuzz run kitty_keys`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use term_image::kitty::{Action, Keys, MAX_KEYS_LEN};

fuzz_target!(|keys: &[u8]| {
    match Keys::parse(keys) {
        Ok(c) => {
            assert!(keys.len() <= MAX_KEYS_LEN);
            assert!(c.image_id == 0 || c.image_number == 0);
            assert!(c.quiet <= 2);
            assert_eq!(c.delete.is_some(), c.action == Action::Delete);
        }
        Err(e) => assert_eq!(e.code(), "EINVAL"),
    }
});
