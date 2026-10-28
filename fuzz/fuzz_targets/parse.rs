//! Terminal output with interleaved resizes, view scrolling and selections,
//! applied to `term_core::Term`. After every step the terminal invariants
//! must hold and everything the renderer and clipboard read must be readable.
//!
//! Input format: see `fuzz/steps.rs`. Run with
//! `cargo +nightly fuzz run parse -- -dict=fuzz/parse.dict`; print a crash as
//! steps with `cargo +nightly fuzz fmt parse <file>`.

#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

#[path = "../steps.rs"]
mod steps;

use steps::Step;

/// The steps of one input. Decoded with our own framing rather than derived,
/// so seeds are plain terminal output and the corpus replay test can decode
/// the same files without depending on `arbitrary`.
struct Input(Vec<Step>);

impl std::fmt::Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(&self.0).finish()
    }
}

impl<'a> Arbitrary<'a> for Input {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let len = u.arbitrary_len::<u8>()?;
        Ok(Input(steps::decode(u.bytes(len)?)))
    }

    fn arbitrary_take_rest(u: Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Input(steps::decode(u.take_rest())))
    }
}

fuzz_target!(|input: Input| {
    let mut term = steps::new_term();
    for step in &input.0 {
        steps::apply(&mut term, step);
        if let Err(e) = term.state.check_invariants() {
            // The message comes first: the nightly job deduplicates crashes by
            // the first line of the panic message.
            panic!("invariant broken: {e}\nafter step {step:?}");
        }
        steps::read_back(&term);
    }
});
