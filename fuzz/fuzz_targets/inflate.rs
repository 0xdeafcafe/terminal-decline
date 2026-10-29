//! zlib inflate with a cap (term-image), fed in slices: never panics, never
//! hands on more than the cap, and slicing doesn't change the result.
//!
//! Run with `cargo +nightly fuzz run inflate`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use term_image::inflate::{Inflate, inflate_all};

const CAP: usize = 1 << 20;

fuzz_target!(|input: &[u8]| {
    let Some((&step, data)) = input.split_first() else { return };
    let whole = inflate_all(data, CAP);
    let mut z = Inflate::new(CAP);
    let mut out = Vec::new();
    let mut result = Ok(());
    for slice in data.chunks(step as usize + 1) {
        result = z.feed(slice, &mut out);
        if result.is_err() {
            break;
        }
    }
    let result = result.and_then(|()| z.finish());
    assert!(out.len() <= CAP);
    match (whole, result) {
        (Ok(a), Ok(())) => assert_eq!(a, out),
        (Err(a), Err(b)) => assert_eq!(a, b),
        (a, b) => panic!("whole {a:?}, sliced {b:?}"),
    }
});
