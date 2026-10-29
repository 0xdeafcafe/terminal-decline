//! Today's app as a library: `main` is term-app's main.rs, included.

use std::ffi::{c_char, c_int};

use term_pin::abi::PinInfoV1;

include!(concat!(env!("OUT_DIR"), "/main.rs"));

/// As a pin: today's code already tells a pin from the app by the
/// executable's file name, which is the stub's.
#[unsafe(no_mangle)]
pub extern "C" fn td_pin_main_v1(info: *const PinInfoV1) -> c_int {
    // SAFETY: the stub passes a valid struct.
    if info.is_null() || unsafe { (*info).size } as usize != size_of::<PinInfoV1>() {
        return 70;
    }
    match std::panic::catch_unwind(main) {
        Ok(()) => 0,
        Err(_) => 70,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn td_app_main_v1(_argc: c_int, _argv: *const *const c_char) -> c_int {
    match std::panic::catch_unwind(main) {
        Ok(()) => 0,
        Err(_) => 70,
    }
}
