//! Image header sniffing and sizes (term-image: PNG, JPEG, GIF, WebP):
//! never panics; a size it reports is non-zero, and a header that parses
//! still parses with more bytes after it.
//!
//! Run with `cargo +nightly fuzz run sniff`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use term_image::probe::{ProbeError, probe, sniff};

fuzz_target!(|head: &[u8]| {
    match probe(head) {
        Ok(p) => {
            assert!(p.width > 0 && p.height > 0);
            assert!((1..=8).contains(&p.orientation));
            assert_eq!(sniff(head), Some(p.format));
            let mut longer = head.to_vec();
            longer.extend_from_slice(&[0; 64]);
            assert_eq!(probe(&longer), Ok(p));
        }
        Err(ProbeError::Unsupported) => assert!(sniff(head).is_none()),
        Err(_) => {}
    }
});
