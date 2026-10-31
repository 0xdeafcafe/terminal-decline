//! Parser for implementing virtual terminal emulators
//!
//! [`Parser`] is implemented according to [Paul Williams' ANSI parser state
//! machine]. The state machine doesn't assign meaning to the parsed data and is
//! thus not itself sufficient for writing a terminal emulator. Instead, it is
//! expected that an implementation of [`Perform`] is provided which does
//! something useful with the parsed data. The [`Parser`] handles the book
//! keeping, and the [`Perform`] gets to simply handle actions.
//!
//! # Examples
//!
//! For an example of using the [`Parser`] please see the examples folder. The
//! example included there simply logs all the actions [`Perform`] does. One
//! quick way to see it in action is to pipe `printf` into it
//!
//! ```sh
//! printf '\x1b[31mExample' | cargo run --example parselog
//! ```
//!
//! # Differences from original state machine description
//!
//! * UTF-8 Support for Input
//! * OSC Strings can be terminated by 0x07
//! * Only supports 7-bit codes
//!
//! [`Parser`]: struct.Parser.html
//! [`Perform`]: trait.Perform.html
//! [Paul Williams' ANSI parser state machine]: https://vt100.net/emu/dec_ansi_parser
#![deny(clippy::all, clippy::if_not_else, clippy::enum_glob_use)]
#![cfg_attr(not(feature = "std"), no_std)]

use core::mem::MaybeUninit;
use core::str;

#[cfg(not(feature = "std"))]
use arrayvec::ArrayVec;

mod params;

#[cfg(feature = "ansi")]
pub mod ansi;
pub use params::{Params, ParamsIter};

const MAX_INTERMEDIATES: usize = 2;
const MAX_OSC_PARAMS: usize = 16;
const MAX_OSC_RAW: usize = 1024;

/// An OSC string longer than this is reported to [`Perform::osc_large`].
const OSC_LARGE_THRESHOLD: usize = 4096;

/// Parser for raw _VTE_ protocol which delegates actions to a [`Perform`]
///
/// [`Perform`]: trait.Perform.html
///
/// Generic over the value for the size of the raw Operating System Command
/// buffer. Only used when the `std` feature is not enabled.
#[derive(Default)]
pub struct Parser<const OSC_RAW_BUF_SIZE: usize = MAX_OSC_RAW> {
    state: State,
    intermediates: [u8; MAX_INTERMEDIATES],
    intermediate_idx: usize,
    params: Params,
    param: u16,
    #[cfg(not(feature = "std"))]
    osc_raw: ArrayVec<u8, OSC_RAW_BUF_SIZE>,
    #[cfg(feature = "std")]
    osc_raw: Vec<u8>,
    osc_params: [(usize, usize); MAX_OSC_PARAMS],
    osc_num_params: usize,
    osc_mode: OscMode,
    osc_limit: usize,
    ignoring: bool,
    partial_utf8: [u8; 4],
    partial_utf8_len: usize,
}

impl Parser {
    /// Create a new Parser
    pub fn new() -> Parser {
        Default::default()
    }
}

impl<const OSC_RAW_BUF_SIZE: usize> Parser<OSC_RAW_BUF_SIZE> {
    /// Create a new Parser with a custom size for the Operating System Command
    /// buffer.
    ///
    /// Call with a const-generic param on `Parser`, like:
    ///
    /// ```rust
    /// let mut p = vte::Parser::<64>::new_with_size();
    /// ```
    #[cfg(not(feature = "std"))]
    pub fn new_with_size() -> Parser<OSC_RAW_BUF_SIZE> {
        Default::default()
    }

    #[inline]
    fn params(&self) -> &Params {
        &self.params
    }

    #[inline]
    fn intermediates(&self) -> &[u8] {
        &self.intermediates[..self.intermediate_idx]
    }

    /// Advance the parser state.
    ///
    /// Requires a [`Perform`] implementation to handle the triggered actions.
    ///
    /// [`Perform`]: trait.Perform.html
    #[inline]
    pub fn advance<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) {
        let mut i = 0;

        // Handle partial codepoints from previous calls to `advance`.
        if self.partial_utf8_len != 0 {
            i += self.advance_partial_utf8(performer, bytes);
        }

        while i != bytes.len() {
            match self.state {
                State::Ground => {
                    i += self.advance_ground(performer, &bytes[i..]);
                    // Most escapes are CSIs: take one straight away
                    // rather than on the loop's next turn.
                    if self.state == State::Escape && bytes.get(i) == Some(&b'[') {
                        self.state = State::CsiEntry;
                        i += 1;
                        i += self.advance_csi_run(performer, &bytes[i..]);
                    }
                },
                State::ApcString | State::DcsPassthrough | State::OscString => {
                    i += self.advance_string(performer, &bytes[i..])
                },
                State::Escape if bytes[i] == b'[' => {
                    self.reset_params();
                    self.state = State::CsiEntry;
                    i += 1;
                    i += self.advance_csi_run(performer, &bytes[i..]);
                },
                State::CsiEntry | State::CsiParam if Self::csi_run_byte(bytes[i]) => {
                    i += self.advance_csi_run(performer, &bytes[i..]);
                },
                _ => {
                    // Inlining it results in worse codegen.
                    let byte = bytes[i];
                    self.change_state(performer, byte);
                    i += 1;
                },
            }
        }
    }

    /// [`Self::advance`] without the CSI fast path, for tests to compare
    /// against.
    #[cfg(test)]
    fn advance_reference<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) {
        let mut i = 0;
        if self.partial_utf8_len != 0 {
            i += self.advance_partial_utf8(performer, bytes);
        }
        while i != bytes.len() {
            match self.state {
                State::Ground => i += self.advance_ground(performer, &bytes[i..]),
                State::ApcString | State::DcsPassthrough | State::OscString => {
                    i += self.advance_string(performer, &bytes[i..])
                },
                _ => {
                    self.change_state(performer, bytes[i]);
                    i += 1;
                },
            }
        }
    }

    /// Whether [`Self::advance_csi_run`] takes `byte` in CSI entry or
    /// parameter state: digits, `;`, `:` and final bytes.
    #[inline(always)]
    fn csi_run_byte(byte: u8) -> bool {
        matches!(byte, b'0'..=b';' | 0x40..=0x7E)
    }

    /// The CSI fast path: in CSI entry or parameter state, take digits,
    /// `;`, `:` and a final byte in a tight loop, with the same actions the
    /// state machine would run for them, and stop at the first other byte
    /// (a private marker, an intermediate, a control) for the state
    /// machine to handle. Returns the bytes used; the state is left as the
    /// state machine would leave it.
    ///
    /// The parameter being read is kept in a local and the state written
    /// once at the end: stored to `self` on every byte, as the actions do,
    /// they made the loop a chain of stores.
    #[inline(always)]
    fn advance_csi_run<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        let mut param = self.param;
        let mut i = 0;
        while let Some(&byte) = bytes.get(i) {
            match byte {
                b'0'..=b'9' => {
                    if self.params.is_full() {
                        self.ignoring = true;
                    } else if let Some((n, v)) = bytes.get(i..i + 8).and_then(digit_run) {
                        // The whole number at once: a branch per digit
                        // mispredicted where each number ends.
                        param = (param as u64 * POW10[n] + v).min(u16::MAX as u64) as u16;
                        i += n;
                        continue;
                    } else {
                        param = param.saturating_mul(10).saturating_add((byte - b'0') as u16);
                    }
                },
                b':' | b';' => {
                    if self.params.is_full() {
                        self.ignoring = true;
                    } else {
                        if byte == b':' {
                            self.params.extend(param);
                        } else {
                            self.params.push(param);
                        }
                        param = 0;
                    }
                },
                0x40..=0x7E => {
                    self.param = param;
                    self.action_csi_dispatch(performer, byte);
                    return i + 1;
                },
                _ => break,
            }
            i += 1;
        }
        self.param = param;
        if i > 0 {
            self.state = State::CsiParam;
        }
        i
    }

    /// Partially advance the parser state.
    ///
    /// This is equivalent to [`Self::advance`], but stops when
    /// [`Perform::terminated`] is true after reading a byte.
    ///
    /// Returns the number of bytes read before termination.
    ///
    /// See [`Perform::advance`] for more details.
    #[inline]
    #[must_use = "Returned value should be used to processs the remaining bytes"]
    pub fn advance_until_terminated<P: Perform>(
        &mut self,
        performer: &mut P,
        bytes: &[u8],
    ) -> usize {
        let mut i = 0;

        // Handle partial codepoints from previous calls to `advance`.
        if self.partial_utf8_len != 0 {
            i += self.advance_partial_utf8(performer, bytes);
        }

        while i != bytes.len() && !performer.terminated() {
            match self.state {
                State::Ground => i += self.advance_ground(performer, &bytes[i..]),
                State::ApcString | State::DcsPassthrough | State::OscString => {
                    i += self.advance_string(performer, &bytes[i..])
                },
                State::Escape if bytes[i] == b'[' => {
                    self.reset_params();
                    self.state = State::CsiEntry;
                    i += 1;
                    i += self.advance_csi_run(performer, &bytes[i..]);
                },
                State::CsiEntry | State::CsiParam if Self::csi_run_byte(bytes[i]) => {
                    i += self.advance_csi_run(performer, &bytes[i..]);
                },
                _ => {
                    // Inlining it results in worse codegen.
                    let byte = bytes[i];
                    self.change_state(performer, byte);
                    i += 1;
                },
            }
        }

        i
    }

    #[inline(always)]
    fn change_state<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match self.state {
            State::CsiEntry => self.advance_csi_entry(performer, byte),
            State::CsiIgnore => self.advance_csi_ignore(performer, byte),
            State::CsiIntermediate => self.advance_csi_intermediate(performer, byte),
            State::CsiParam => self.advance_csi_param(performer, byte),
            State::DcsEntry => self.advance_dcs_entry(performer, byte),
            State::DcsIgnore => self.anywhere(performer, byte),
            State::DcsIntermediate => self.advance_dcs_intermediate(performer, byte),
            State::DcsParam => self.advance_dcs_param(performer, byte),
            State::DcsPassthrough => self.advance_dcs_passthrough(performer, byte),
            State::Escape => self.advance_esc(performer, byte),
            State::EscapeIntermediate => self.advance_esc_intermediate(performer, byte),
            State::OscString => self.advance_osc_string(performer, byte),
            State::SosPmApcString => self.anywhere(performer, byte),
            State::ApcString => {
                self.advance_apc(performer, &[byte]);
            },
            State::ApcEscape => self.advance_apc_escape(performer, byte),
            State::DcsEscape => self.advance_dcs_escape(performer, byte),
            State::Ground => unreachable!(),
        }
    }

    #[inline(always)]
    fn advance_csi_entry<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => {
                self.action_collect(byte);
                self.state = State::CsiIntermediate
            },
            0x30..=0x39 => {
                self.action_paramnext(byte);
                self.state = State::CsiParam
            },
            0x3A => {
                self.action_subparam();
                self.state = State::CsiParam
            },
            0x3B => {
                self.action_param();
                self.state = State::CsiParam
            },
            0x3C..=0x3F => {
                self.action_collect(byte);
                self.state = State::CsiParam
            },
            0x40..=0x7E => self.action_csi_dispatch(performer, byte),
            _ => self.anywhere(performer, byte),
        }
    }

    #[inline(always)]
    fn advance_csi_ignore<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x3F => (),
            0x40..=0x7E => self.state = State::Ground,
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    #[inline(always)]
    fn advance_csi_intermediate<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => self.action_collect(byte),
            0x30..=0x3F => self.state = State::CsiIgnore,
            0x40..=0x7E => self.action_csi_dispatch(performer, byte),
            _ => self.anywhere(performer, byte),
        }
    }

    #[inline(always)]
    fn advance_csi_param<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => {
                self.action_collect(byte);
                self.state = State::CsiIntermediate
            },
            0x30..=0x39 => self.action_paramnext(byte),
            0x3A => self.action_subparam(),
            0x3B => self.action_param(),
            0x3C..=0x3F => self.state = State::CsiIgnore,
            0x40..=0x7E => self.action_csi_dispatch(performer, byte),
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    #[inline(always)]
    fn advance_dcs_entry<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => (),
            0x20..=0x2F => {
                self.action_collect(byte);
                self.state = State::DcsIntermediate
            },
            0x30..=0x39 => {
                self.action_paramnext(byte);
                self.state = State::DcsParam
            },
            0x3A => {
                self.action_subparam();
                self.state = State::DcsParam
            },
            0x3B => {
                self.action_param();
                self.state = State::DcsParam
            },
            0x3C..=0x3F => {
                self.action_collect(byte);
                self.state = State::DcsParam
            },
            0x40..=0x7E => self.action_hook(performer, byte),
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    #[inline(always)]
    fn advance_dcs_intermediate<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => (),
            0x20..=0x2F => self.action_collect(byte),
            0x30..=0x3F => self.state = State::DcsIgnore,
            0x40..=0x7E => self.action_hook(performer, byte),
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    #[inline(always)]
    fn advance_dcs_param<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => (),
            0x20..=0x2F => {
                self.action_collect(byte);
                self.state = State::DcsIntermediate
            },
            0x30..=0x39 => self.action_paramnext(byte),
            0x3A => self.action_subparam(),
            0x3B => self.action_param(),
            0x3C..=0x3F => self.state = State::DcsIgnore,
            0x40..=0x7E => self.action_hook(performer, byte),
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    #[inline(always)]
    fn advance_dcs_passthrough<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x7E => performer.put_slice(&[byte]),
            0x18 | 0x1A => {
                performer.dcs_end(false);
                performer.execute(byte);
                self.state = State::Ground
            },
            // ST or not is known only at the next byte.
            0x1B => self.state = State::DcsEscape,
            0x7F => (),
            0x9C => {
                performer.dcs_end(true);
                self.state = State::Ground
            },
            _ => (),
        }
    }

    #[inline(always)]
    fn advance_esc<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => {
                self.action_collect(byte);
                self.state = State::EscapeIntermediate
            },
            0x30..=0x4F => {
                performer.esc_dispatch(self.intermediates(), self.ignoring, byte);
                self.state = State::Ground
            },
            0x50 => {
                self.reset_params();
                self.state = State::DcsEntry
            },
            0x51..=0x57 => {
                performer.esc_dispatch(self.intermediates(), self.ignoring, byte);
                self.state = State::Ground
            },
            0x58 | 0x5E => self.state = State::SosPmApcString,
            0x5F => {
                performer.apc_start();
                self.state = State::ApcString
            },
            0x59..=0x5A => {
                performer.esc_dispatch(self.intermediates(), self.ignoring, byte);
                self.state = State::Ground
            },
            0x5B => {
                self.reset_params();
                self.state = State::CsiEntry
            },
            0x5C => {
                performer.esc_dispatch(self.intermediates(), self.ignoring, byte);
                self.state = State::Ground
            },
            0x5D => {
                self.osc_raw.clear();
                self.osc_num_params = 0;
                self.osc_mode = OscMode::Normal;
                self.osc_limit = OSC_LARGE_THRESHOLD;
                self.state = State::OscString
            },
            0x60..=0x7E => {
                performer.esc_dispatch(self.intermediates(), self.ignoring, byte);
                self.state = State::Ground
            },
            // Anywhere.
            0x18 | 0x1A => {
                performer.execute(byte);
                self.state = State::Ground
            },
            0x1B => (),
            _ => (),
        }
    }

    #[inline(always)]
    fn advance_esc_intermediate<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => performer.execute(byte),
            0x20..=0x2F => self.action_collect(byte),
            0x30..=0x7E => {
                performer.esc_dispatch(self.intermediates(), self.ignoring, byte);
                self.state = State::Ground
            },
            0x7F => (),
            _ => self.anywhere(performer, byte),
        }
    }

    #[inline(always)]
    fn advance_osc_string<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1C..=0x1F => (),
            0x07 => {
                self.osc_finish(performer, byte);
                self.state = State::Ground
            },
            0x18 | 0x1A => {
                self.osc_finish(performer, byte);
                performer.execute(byte);
                self.state = State::Ground
            },
            0x1B => {
                self.osc_finish(performer, byte);
                self.reset_params();
                self.state = State::Escape
            },
            _ => self.action_osc_put(performer, byte),
        }
    }

    #[inline(always)]
    fn anywhere<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match byte {
            0x18 | 0x1A => {
                performer.execute(byte);
                self.state = State::Ground
            },
            0x1B => {
                self.reset_params();
                self.state = State::Escape
            },
            _ => (),
        }
    }

    #[inline]
    fn action_csi_dispatch<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        if self.params.is_full() {
            self.ignoring = true;
        } else {
            self.params.push(self.param);
        }
        performer.csi_dispatch(self.params(), self.intermediates(), self.ignoring, byte as char);

        self.state = State::Ground
    }

    #[inline]
    fn action_hook<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        if self.params.is_full() {
            self.ignoring = true;
        } else {
            self.params.push(self.param);
        }
        performer.hook(self.params(), self.intermediates(), self.ignoring, byte as char);
        self.state = State::DcsPassthrough;
    }

    #[inline]
    fn action_collect(&mut self, byte: u8) {
        if self.intermediate_idx == MAX_INTERMEDIATES {
            self.ignoring = true;
        } else {
            self.intermediates[self.intermediate_idx] = byte;
            self.intermediate_idx += 1;
        }
    }

    /// Advance to the next subparameter.
    #[inline]
    fn action_subparam(&mut self) {
        if self.params.is_full() {
            self.ignoring = true;
        } else {
            self.params.extend(self.param);
            self.param = 0;
        }
    }

    /// Advance to the next parameter.
    #[inline]
    fn action_param(&mut self) {
        if self.params.is_full() {
            self.ignoring = true;
        } else {
            self.params.push(self.param);
            self.param = 0;
        }
    }

    /// Advance inside the parameter without terminating it.
    #[inline]
    fn action_paramnext(&mut self, byte: u8) {
        if self.params.is_full() {
            self.ignoring = true;
        } else {
            // Continue collecting bytes into param.
            self.param = self.param.saturating_mul(10);
            self.param = self.param.saturating_add((byte - b'0') as u16);
        }
    }

    /// Add OSC param separator.
    #[inline]
    fn action_osc_put_param(&mut self) {
        let idx = self.osc_raw.len();

        let param_idx = self.osc_num_params;
        match param_idx {
            // First param is special - 0 to current byte index.
            0 => self.osc_params[param_idx] = (0, idx),

            // Only process up to MAX_OSC_PARAMS.
            MAX_OSC_PARAMS => return,

            // All other params depend on previous indexing.
            _ => {
                let prev = self.osc_params[param_idx - 1];
                // Skip the separator after the previous parameter.
                let begin = prev.1 + 1;
                self.osc_params[param_idx] = (begin, idx);
            },
        }

        self.osc_num_params += 1;
    }

    /// Store one byte of an OSC string; `;` is stored too, and also ends a
    /// parameter. Applies the size policy (see [`Perform::osc_large`]).
    #[inline(always)]
    fn action_osc_put<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match self.osc_mode {
            OscMode::Drop => return,
            OscMode::Stream => return performer.osc_put(&[byte]),
            OscMode::Normal | OscMode::Buffer => (),
        }
        if self.osc_raw.len() >= self.osc_limit && !self.osc_over_limit(performer, byte) {
            return;
        }
        #[cfg(not(feature = "std"))]
        {
            if self.osc_raw.is_full() {
                return;
            }
        }
        if byte == b';' {
            self.action_osc_put_param();
        }
        self.osc_raw.push(byte);
    }

    /// The buffered OSC string reached its limit. Returns whether `byte`
    /// should still be stored.
    #[cold]
    #[inline(never)]
    fn osc_over_limit<P: Perform>(&mut self, performer: &mut P, byte: u8) -> bool {
        if self.osc_mode == OscMode::Normal {
            match performer.osc_large(&self.osc_raw) {
                OscLarge::Buffer(cap) => {
                    self.osc_mode = OscMode::Buffer;
                    self.osc_limit = cap;
                },
                OscLarge::Stream => {
                    self.osc_mode = OscMode::Stream;
                    performer.osc_put(&[byte]);
                    return false;
                },
                OscLarge::Drop => {
                    self.osc_mode = OscMode::Drop;
                    return false;
                },
            }
        }
        if self.osc_raw.len() >= self.osc_limit {
            self.osc_mode = OscMode::Drop;
            return false;
        }
        true
    }

    fn osc_finish<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        match self.osc_mode {
            OscMode::Normal | OscMode::Buffer => {
                self.action_osc_put_param();
                self.osc_dispatch(performer, byte);
            },
            OscMode::Stream => performer.osc_end(byte != 0x18 && byte != 0x1A),
            OscMode::Drop => (),
        }
        self.osc_raw.clear();
        self.osc_num_params = 0;
        self.osc_mode = OscMode::Normal;
    }

    /// Advance inside a string state (APC, DCS passthrough, OSC), handing
    /// over whole runs where there are any. Returns the bytes used.
    #[inline(never)]
    fn advance_string<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        match self.state {
            State::ApcString => self.advance_apc(performer, bytes),
            State::DcsPassthrough => self.advance_dcs_run(performer, bytes),
            State::OscString if self.osc_mode == OscMode::Stream => {
                self.advance_osc_stream(performer, bytes)
            },
            _ => {
                self.advance_osc_string(performer, bytes[0]);
                1
            },
        }
    }

    /// The rest of a streamed OSC string: pass the run before the next
    /// control byte whole, and handle a control byte alone.
    #[inline]
    fn advance_osc_stream<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        let run = bytes.iter().position(|&b| b < 0x20).unwrap_or(bytes.len());
        if run > 0 {
            performer.osc_put(&bytes[..run]);
            return run;
        }
        self.advance_osc_string(performer, bytes[0]);
        1
    }

    /// Inside an APC string: pass everything up to ESC, CAN or SUB whole.
    #[inline]
    fn advance_apc<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        match memchr::memchr3(0x1B, 0x18, 0x1A, bytes) {
            None => {
                performer.apc_put(bytes);
                bytes.len()
            },
            Some(end) => {
                if end > 0 {
                    performer.apc_put(&bytes[..end]);
                }
                if bytes[end] == 0x1B {
                    self.state = State::ApcEscape;
                } else {
                    performer.apc_end(false);
                    performer.execute(bytes[end]);
                    self.state = State::Ground;
                }
                end + 1
            },
        }
    }

    /// The byte after an ESC inside an APC string.
    fn advance_apc_escape<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        if byte == b'\\' {
            performer.apc_end(true);
            self.state = State::Ground;
        } else {
            // Abandoned: the ESC starts a new escape sequence.
            performer.apc_end(false);
            self.reset_params();
            self.state = State::Escape;
            self.advance_esc(performer, byte);
        }
    }

    /// The byte after an ESC inside DCS passthrough.
    fn advance_dcs_escape<P: Perform>(&mut self, performer: &mut P, byte: u8) {
        if byte == b'\\' {
            performer.dcs_end(true);
            self.state = State::Ground;
        } else {
            // Abandoned: the ESC starts a new escape sequence.
            performer.dcs_end(false);
            self.reset_params();
            self.state = State::Escape;
            self.advance_esc(performer, byte);
        }
    }

    /// Inside DCS passthrough: pass the run of data bytes whole, and handle
    /// any other byte alone.
    #[inline]
    fn advance_dcs_run<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        let run = bytes
            .iter()
            .position(|&b| matches!(b, 0x18 | 0x1A | 0x1B | 0x7F..=0xFF))
            .unwrap_or(bytes.len());
        if run > 0 {
            performer.put_slice(&bytes[..run]);
            return run;
        }
        self.advance_dcs_passthrough(performer, bytes[0]);
        1
    }

    /// Reset escape sequence parameters and intermediates.
    #[inline]
    fn reset_params(&mut self) {
        self.intermediate_idx = 0;
        self.ignoring = false;
        self.param = 0;

        self.params.clear();
    }

    /// Separate method for osc_dispatch that borrows self as read-only
    ///
    /// The aliasing is needed here for multiple slices into self.osc_raw
    #[inline]
    fn osc_dispatch<P: Perform>(&self, performer: &mut P, byte: u8) {
        let mut slices: [MaybeUninit<&[u8]>; MAX_OSC_PARAMS] =
            unsafe { MaybeUninit::uninit().assume_init() };

        for (i, slice) in slices.iter_mut().enumerate().take(self.osc_num_params) {
            let indices = self.osc_params[i];
            *slice = MaybeUninit::new(&self.osc_raw[indices.0..indices.1]);
        }

        unsafe {
            let num_params = self.osc_num_params;
            let params = &slices[..num_params] as *const [MaybeUninit<&[u8]>] as *const [&[u8]];
            performer.osc_dispatch(&*params, byte == 0x07);
        }
    }

    /// Advance the parser state from ground.
    ///
    /// The ground state is handled separately since it can only be left using
    /// the escape character (`\x1b`). Runs of printable ASCII are handed to
    /// [`Perform::print_ascii`] whole and other ASCII bytes (C0 controls,
    /// DEL) are handled inline; only non-ASCII bytes go through UTF-8
    /// decoding.
    #[inline]
    fn advance_ground<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        let mut i = 0;
        while i < bytes.len() {
            let run = printable_ascii_len(&bytes[i..]);
            if run > 0 {
                performer.print_ascii(&bytes[i..i + run]);
                i += run;
                if i == bytes.len() {
                    break;
                }
            }
            match bytes[i] {
                0x1B => {
                    self.state = State::Escape;
                    self.reset_params();
                    return i + 1;
                },
                byte @ 0x00..=0x1F => performer.execute(byte),
                0x7F => performer.print('\x7f'),
                _ => break,
            }
            i += 1;
        }
        if i > 0 {
            return i;
        }
        self.advance_ground_utf8(performer, bytes)
    }

    /// Advance from ground over a stretch of non-ASCII bytes: `bytes` starts
    /// with a byte >= 0x80, and the stretch ends at the next ASCII byte.
    #[inline(never)]
    fn advance_ground_utf8<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        let num_bytes = bytes.len();
        let plain_chars = bytes.iter().position(|&b| b < 0x80).unwrap_or(num_bytes);

        match str::from_utf8(&bytes[..plain_chars]) {
            Ok(parsed) => {
                Self::ground_dispatch(performer, parsed);
                plain_chars
            },
            // Handle invalid and partial utf8.
            Err(err) => {
                // Dispatch all the valid bytes.
                let valid_bytes = err.valid_up_to();
                let parsed = unsafe { str::from_utf8_unchecked(&bytes[..valid_bytes]) };
                Self::ground_dispatch(performer, parsed);

                match err.error_len() {
                    Some(len) => {
                        // Execute C1 escapes or emit replacement character.
                        if len == 1 && bytes[valid_bytes] <= 0x9F {
                            performer.execute(bytes[valid_bytes]);
                        } else {
                            performer.print('�');
                        }

                        // Restart processing after the invalid bytes.
                        //
                        // While we could theoretically try to just re-parse
                        // `bytes[valid_bytes + len..plain_chars]`, it's easier
                        // to just skip it and invalid utf8 is pretty rare anyway.
                        valid_bytes + len
                    },
                    None => {
                        if plain_chars < num_bytes {
                            // A character cut off by an ASCII byte (an escape
                            // or anything else) is one invalid sequence; the
                            // ASCII byte is handled by the next call.
                            performer.print('�');
                            plain_chars
                        } else {
                            // Process bytes cut off by the buffer end.
                            let extra_bytes = num_bytes - valid_bytes;
                            let partial_len = self.partial_utf8_len + extra_bytes;
                            self.partial_utf8[self.partial_utf8_len..partial_len]
                                .copy_from_slice(&bytes[valid_bytes..valid_bytes + extra_bytes]);
                            self.partial_utf8_len = partial_len;
                            num_bytes
                        }
                    },
                }
            },
        }
    }

    /// Advance the parser while processing a partial utf8 codepoint.
    #[inline]
    fn advance_partial_utf8<P: Perform>(&mut self, performer: &mut P, bytes: &[u8]) -> usize {
        // Try to copy up to 3 more characters, to ensure the codepoint is complete.
        let old_bytes = self.partial_utf8_len;
        let to_copy = bytes.len().min(self.partial_utf8.len() - old_bytes);
        self.partial_utf8[old_bytes..old_bytes + to_copy].copy_from_slice(&bytes[..to_copy]);
        self.partial_utf8_len += to_copy;

        // Parse the unicode character.
        match str::from_utf8(&self.partial_utf8[..self.partial_utf8_len]) {
            // If the entire buffer is valid, use the first character and continue parsing.
            Ok(parsed) => {
                let c = unsafe { parsed.chars().next().unwrap_unchecked() };
                Self::dispatch_char(performer, c);

                self.partial_utf8_len = 0;
                c.len_utf8() - old_bytes
            },
            Err(err) => {
                let valid_bytes = err.valid_up_to();
                // If we have any valid bytes, that means we partially copied another
                // utf8 character into `partial_utf8`. Since we only care about the
                // first character, we just ignore the rest.
                if valid_bytes > 0 {
                    let c = unsafe {
                        let parsed = str::from_utf8_unchecked(&self.partial_utf8[..valid_bytes]);
                        parsed.chars().next().unwrap_unchecked()
                    };

                    Self::dispatch_char(performer, c);

                    // Only the first character is used, so consume just its
                    // bytes and parse whatever follows it normally. (Upstream
                    // returns `valid_bytes - old_bytes` here, which skips the
                    // valid text between the character and an invalid byte.)
                    self.partial_utf8_len = 0;
                    return c.len_utf8() - old_bytes;
                }

                match err.error_len() {
                    // If the partial character was also invalid, emit the replacement
                    // character.
                    Some(invalid_len) => {
                        performer.print('�');

                        self.partial_utf8_len = 0;
                        invalid_len - old_bytes
                    },
                    // If the character still isn't complete, wait for more data.
                    None => to_copy,
                }
            },
        }
    }

    /// Handle ground dispatch of print/execute for all characters in a string.
    #[inline]
    fn ground_dispatch<P: Perform>(performer: &mut P, text: &str) {
        for c in text.chars() {
            Self::dispatch_char(performer, c);
        }
    }

    /// Print a character, or execute it if it is a C0 or C1 control. (Upstream
    /// prints C1 controls that complete a character split across calls; this
    /// copy executes them, as it does when they arrive in one piece.)
    #[inline(always)]
    fn dispatch_char<P: Perform>(performer: &mut P, c: char) {
        match c {
            '\x00'..='\x1f' | '\u{80}'..='\u{9f}' => performer.execute(c as u8),
            _ => performer.print(c),
        }
    }
}

/// Length of the run of printable ASCII (0x20..=0x7E) at the start of `bytes`.
///
/// Looks at 8 bytes at a time. In a little-endian `u64` `x`:
/// - `(x - 0x2020…) & !x & 0x8080…` marks bytes below 0x20. A borrow can mark
///   a byte above a marked one by mistake, but never below, so the lowest mark
///   is exact.
/// - `(((x & !0x8080…) + 0x0101…) | x) & 0x8080…` marks exactly the bytes at or
///   above 0x7F (the addition can't carry into the next byte).
///
/// The lowest mark of the two together is the first byte that ends the run.
#[inline(always)]
fn printable_ascii_len(bytes: &[u8]) -> usize {
    const LOW: u64 = 0x2020_2020_2020_2020;
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGH: u64 = 0x8080_8080_8080_8080;

    let mut i = 0;
    while let Some(word) = bytes.get(i..i + 8) {
        let x = u64::from_le_bytes([
            word[0], word[1], word[2], word[3], word[4], word[5], word[6], word[7],
        ]);
        let below = x.wrapping_sub(LOW) & !x & HIGH;
        let above = (((x & !HIGH) + ONES) | x) & HIGH;
        let stop = below | above;
        if stop != 0 {
            return i + (stop.trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    // The last few bytes, one at a time (never reading past the end).
    while i < bytes.len() && (0x20..0x7F).contains(&bytes[i]) {
        i += 1;
    }
    i
}

/// What the parser is doing with the OSC string it is reading.
#[derive(PartialEq, Eq, Debug, Default, Copy, Clone)]
enum OscMode {
    /// Buffering, below the 4 KB threshold.
    #[default]
    Normal,
    /// Buffering up to the limit the performer chose.
    Buffer,
    /// Passing the rest to the performer.
    Stream,
    /// Discarding the rest; nothing will be dispatched.
    Drop,
}

/// What to do with an OSC string that has passed 4 KB; the answer to
/// [`Perform::osc_large`].
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum OscLarge {
    /// Keep buffering up to this many bytes in all (separators included).
    /// Past it the string is dropped, and not dispatched.
    Buffer(usize),
    /// Pass the rest to [`Perform::osc_put`], then call [`Perform::osc_end`].
    /// [`Perform::osc_dispatch`] is not called.
    Stream,
    /// Throw the string away.
    Drop,
}

/// Powers of ten up to 10⁷, for [`digit_run`]'s numbers.
const POW10: [u64; 8] = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000];

/// The run of ASCII digits `bytes` (8 of them) starts with, with no branch
/// per digit: its length (1 to 7) and value, or `None` if all 8 are digits.
#[inline(always)]
fn digit_run(bytes: &[u8]) -> Option<(usize, u64)> {
    const ONES: u64 = 0x0101_0101_0101_0101;
    let word = u64::from_le_bytes(bytes.try_into().ok()?);
    // Digits become 0-9; a byte is flagged if it is above 9 (adding 0x76
    // sets its top bit) or has its top bit set already. Carries only go
    // into later bytes, so the first flagged byte is the first non-digit.
    let t = word ^ (0x30 * ONES);
    let flagged = (t.wrapping_add(0x76 * ONES) | t) & (0x80 * ONES);
    let n = (flagged.trailing_zeros() / 8) as usize;
    if n == 0 || n == 8 {
        return None;
    }
    // The digits to the top, the first one highest after the zeros left
    // in front; then pairs, fours and eights of digits combined (the
    // usual SWAR conversion).
    let v = t << (64 - 8 * n);
    let v = (v.wrapping_mul(10 << 8 | 1) >> 8) & 0x00FF_00FF_00FF_00FF;
    let v = (v.wrapping_mul(100 << 16 | 1) >> 16) & 0x0000_FFFF_0000_FFFF;
    let v = v.wrapping_mul(10_000 << 32 | 1) >> 32;
    Some((n, v))
}

#[derive(PartialEq, Eq, Debug, Default, Copy, Clone)]
enum State {
    CsiEntry,
    CsiIgnore,
    CsiIntermediate,
    CsiParam,
    DcsEntry,
    DcsIgnore,
    DcsIntermediate,
    DcsParam,
    DcsPassthrough,
    Escape,
    EscapeIntermediate,
    OscString,
    SosPmApcString,
    #[default]
    Ground,
    ApcString,
    ApcEscape,
    DcsEscape,
}

/// Performs actions requested by the Parser
///
/// Actions in this case mean, for example, handling a CSI escape sequence
/// describing cursor movement, or simply printing characters to the screen.
///
/// The methods on this type correspond to actions described in
/// <http://vt100.net/emu/dec_ansi_parser>. I've done my best to describe them in
/// a useful way in my own words for completeness, but the site should be
/// referenced if something isn't clear. If the site disappears at some point in
/// the future, consider checking archive.org.
pub trait Perform {
    /// Draw a character to the screen and update states.
    fn print(&mut self, _c: char) {}

    /// Draw a run of printable ASCII characters (0x20..=0x7E) in the ground
    /// state. Each run is passed whole, so an implementation can write it in
    /// bulk; the default calls [`Perform::print`] for each byte.
    #[inline]
    fn print_ascii(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.print(byte as char);
        }
    }

    /// Execute a C0 or C1 control function.
    fn execute(&mut self, _byte: u8) {}

    /// Invoked when a final character arrives in first part of device control
    /// string.
    ///
    /// The control function should be determined from the private marker, final
    /// character, and execute with a parameter list. A handler should be
    /// selected for remaining characters in the string; the handler
    /// function should subsequently be called by `put` for every character in
    /// the control string.
    ///
    /// The `ignore` flag indicates that more than two intermediates arrived and
    /// subsequent characters were ignored.
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}

    /// Pass bytes as part of a device control string to the handle chosen in
    /// `hook`. C0 controls will also be passed to the handler.
    fn put(&mut self, _byte: u8) {}

    /// Pass several bytes of a device control string at once, to the handler
    /// chosen in `hook`. The default calls [`Perform::put`] for each byte, so
    /// an implementation of `put` alone sees the same bytes in the same order.
    #[inline]
    fn put_slice(&mut self, data: &[u8]) {
        for &byte in data {
            self.put(byte);
        }
    }

    /// `ESC _`: an application program command starts (kitty graphics:
    /// `ESC _ G ...`).
    fn apc_start(&mut self) {}

    /// Part of the command, in order. The command may arrive in any number of
    /// calls, however it was split across `advance` calls.
    fn apc_put(&mut self, _data: &[u8]) {}

    /// The command ended. `complete` is true for ST (`ESC \`), false if it
    /// was abandoned (CAN, SUB, or ESC followed by anything else).
    fn apc_end(&mut self, _complete: bool) {}

    /// An OSC string has passed 4 KB. `prefix` is what has arrived, `;`
    /// separators included. The answer says what to do with the rest; the
    /// default buffers up to 64 KB. Called once per string, and only when it
    /// is longer than 4 KB.
    fn osc_large(&mut self, _prefix: &[u8]) -> OscLarge {
        OscLarge::Buffer(64 * 1024)
    }

    /// After [`OscLarge::Stream`]: the rest of the string, in pieces.
    fn osc_put(&mut self, _data: &[u8]) {}

    /// After [`OscLarge::Stream`]: the string ended. `complete` is false if
    /// it ended with CAN or SUB, true for BEL or ESC.
    fn osc_end(&mut self, _complete: bool) {}

    /// Called when a device control string is terminated.
    ///
    /// The previously selected handler should be notified that the DCS has
    /// terminated. Called by the default [`Perform::dcs_end`].
    fn unhook(&mut self) {}

    /// The device control string ended. `complete` is true for ST (`ESC \`
    /// or C1 `0x9C`), false if it was abandoned (CAN, SUB, or ESC followed by
    /// anything else). The ST is consumed, not reported as an
    /// `esc_dispatch`. The default calls [`Perform::unhook`].
    fn dcs_end(&mut self, _complete: bool) {
        self.unhook();
    }

    /// Dispatch an operating system command.
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}

    /// A final character has arrived for a CSI sequence
    ///
    /// The `ignore` flag indicates that either more than two intermediates
    /// arrived or the number of parameters exceeded the maximum supported
    /// length, and subsequent characters were ignored.
    fn csi_dispatch(
        &mut self,
        _params: &Params,
        _intermediates: &[u8],
        _ignore: bool,
        _action: char,
    ) {
    }

    /// The final character of an escape sequence has arrived.
    ///
    /// The `ignore` flag indicates that more than two intermediates arrived and
    /// subsequent characters were ignored.
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}

    /// Whether the parser should terminate prematurely.
    ///
    /// This can be used in conjunction with
    /// [`Parser::advance_until_terminated`] to terminate the parser after
    /// receiving certain escape sequences like synchronized updates.
    ///
    /// This is checked after every parsed byte, so no expensive computation
    /// should take place in this function.
    #[inline(always)]
    fn terminated(&self) -> bool {
        false
    }
}

#[cfg(all(test, not(feature = "std")))]
#[macro_use]
extern crate std;

#[cfg(test)]
mod tests {
    use std::vec::Vec;

    use super::*;

    const OSC_BYTES: &[u8] = &[
        0x1B, 0x5D, // Begin OSC
        b'2', b';', b'j', b'w', b'i', b'l', b'm', b'@', b'j', b'w', b'i', b'l', b'm', b'-', b'd',
        b'e', b's', b'k', b':', b' ', b'~', b'/', b'c', b'o', b'd', b'e', b'/', b'a', b'l', b'a',
        b'c', b'r', b'i', b't', b't', b'y', 0x07, // End OSC
    ];

    #[derive(Default)]
    struct Dispatcher {
        dispatched: Vec<Sequence>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Sequence {
        Osc(Vec<Vec<u8>>, bool),
        Csi(Vec<Vec<u16>>, Vec<u8>, bool, char),
        Esc(Vec<u8>, bool, u8),
        DcsHook(Vec<Vec<u16>>, Vec<u8>, bool, char),
        DcsPut(u8),
        Print(char),
        Execute(u8),
        DcsUnhook,
    }

    impl Perform for Dispatcher {
        fn osc_dispatch(&mut self, params: &[&[u8]], bell_terminated: bool) {
            let params = params.iter().map(|p| p.to_vec()).collect();
            self.dispatched.push(Sequence::Osc(params, bell_terminated));
        }

        fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, c: char) {
            let params = params.iter().map(|subparam| subparam.to_vec()).collect();
            let intermediates = intermediates.to_vec();
            self.dispatched.push(Sequence::Csi(params, intermediates, ignore, c));
        }

        fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
            let intermediates = intermediates.to_vec();
            self.dispatched.push(Sequence::Esc(intermediates, ignore, byte));
        }

        fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, c: char) {
            let params = params.iter().map(|subparam| subparam.to_vec()).collect();
            let intermediates = intermediates.to_vec();
            self.dispatched.push(Sequence::DcsHook(params, intermediates, ignore, c));
        }

        fn put(&mut self, byte: u8) {
            self.dispatched.push(Sequence::DcsPut(byte));
        }

        fn unhook(&mut self) {
            self.dispatched.push(Sequence::DcsUnhook);
        }

        fn print(&mut self, c: char) {
            self.dispatched.push(Sequence::Print(c));
        }

        fn execute(&mut self, byte: u8) {
            self.dispatched.push(Sequence::Execute(byte));
        }
    }

    #[test]
    fn parse_osc() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, OSC_BYTES);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(params, _) => {
                assert_eq!(params.len(), 2);
                assert_eq!(params[0], &OSC_BYTES[2..3]);
                assert_eq!(params[1], &OSC_BYTES[4..(OSC_BYTES.len() - 1)]);
            },
            _ => panic!("expected osc sequence"),
        }
    }

    #[test]
    fn parse_empty_osc() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, &[0x1B, 0x5D, 0x07]);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(..) => (),
            _ => panic!("expected osc sequence"),
        }
    }

    #[test]
    fn parse_osc_max_params() {
        let params = ";".repeat(params::MAX_PARAMS + 1);
        let input = format!("\x1b]{}\x1b", &params[..]).into_bytes();
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, &input);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(params, _) => {
                assert_eq!(params.len(), MAX_OSC_PARAMS);
                assert!(params.iter().all(Vec::is_empty));
            },
            _ => panic!("expected osc sequence"),
        }
    }

    #[test]
    fn osc_bell_terminated() {
        const INPUT: &[u8] = b"\x1b]11;ff/00/ff\x07";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(_, true) => (),
            _ => panic!("expected osc with bell terminator"),
        }
    }

    #[test]
    fn osc_c0_st_terminated() {
        const INPUT: &[u8] = b"\x1b]11;ff/00/ff\x1b\\";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 2);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(_, false) => (),
            _ => panic!("expected osc with ST terminator"),
        }
    }

    #[test]
    fn parse_osc_with_utf8_arguments() {
        const INPUT: &[u8] = &[
            0x0D, 0x1B, 0x5D, 0x32, 0x3B, 0x65, 0x63, 0x68, 0x6F, 0x20, 0x27, 0xC2, 0xAF, 0x5C,
            0x5F, 0x28, 0xE3, 0x83, 0x84, 0x29, 0x5F, 0x2F, 0xC2, 0xAF, 0x27, 0x20, 0x26, 0x26,
            0x20, 0x73, 0x6C, 0x65, 0x65, 0x70, 0x20, 0x31, 0x07,
        ];
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched[0], Sequence::Execute(b'\r'));
        let osc_data = INPUT[5..(INPUT.len() - 1)].into();
        assert_eq!(dispatcher.dispatched[1], Sequence::Osc(vec![vec![b'2'], osc_data], true));
        assert_eq!(dispatcher.dispatched.len(), 2);
    }

    #[test]
    fn osc_containing_string_terminator() {
        const INPUT: &[u8] = b"\x1b]2;\xe6\x9c\xab\x1b\\";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 2);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(params, _) => {
                assert_eq!(params[1], &INPUT[4..(INPUT.len() - 2)]);
            },
            _ => panic!("expected osc sequence"),
        }
    }

    #[test]
    fn exceed_max_buffer_size() {
        const NUM_BYTES: usize = MAX_OSC_RAW + 100;
        const INPUT_START: &[u8] = b"\x1b]52;s";
        const INPUT_END: &[u8] = b"\x07";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        // Create valid OSC escape
        parser.advance(&mut dispatcher, INPUT_START);

        // Exceed max buffer size
        parser.advance(&mut dispatcher, &[b'a'; NUM_BYTES]);

        // Terminate escape for dispatch
        parser.advance(&mut dispatcher, INPUT_END);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(params, _) => {
                assert_eq!(params.len(), 2);
                assert_eq!(params[0], b"52");

                #[cfg(feature = "std")]
                assert_eq!(params[1].len(), NUM_BYTES + INPUT_END.len());

                #[cfg(not(feature = "std"))]
                // The buffer holds the `;` separator too.
                assert_eq!(params[1].len(), MAX_OSC_RAW - params[0].len() - 1);
            },
            _ => panic!("expected osc sequence"),
        }
    }

    #[test]
    fn parse_csi_max_params() {
        // This will build a list of repeating '1;'s
        // The length is MAX_PARAMS - 1 because the last semicolon is interpreted
        // as an implicit zero, making the total number of parameters MAX_PARAMS
        let params = "1;".repeat(params::MAX_PARAMS - 1);
        let input = format!("\x1b[{}p", &params[..]).into_bytes();

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, &input);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, _, ignore, _) => {
                assert_eq!(params.len(), params::MAX_PARAMS);
                assert!(!ignore);
            },
            _ => panic!("expected csi sequence"),
        }
    }

    #[test]
    fn parse_csi_params_ignore_long_params() {
        // This will build a list of repeating '1;'s
        // The length is MAX_PARAMS because the last semicolon is interpreted
        // as an implicit zero, making the total number of parameters MAX_PARAMS + 1
        let params = "1;".repeat(params::MAX_PARAMS);
        let input = format!("\x1b[{}p", &params[..]).into_bytes();

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, &input);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, _, ignore, _) => {
                assert_eq!(params.len(), params::MAX_PARAMS);
                assert!(ignore);
            },
            _ => panic!("expected csi sequence"),
        }
    }

    #[test]
    fn parse_csi_params_trailing_semicolon() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, b"\x1b[4;m");

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, ..) => assert_eq!(params, &[[4], [0]]),
            _ => panic!("expected csi sequence"),
        }
    }

    #[test]
    fn parse_csi_params_leading_semicolon() {
        // Create dispatcher and check state
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, b"\x1b[;4m");

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, ..) => assert_eq!(params, &[[0], [4]]),
            _ => panic!("expected csi sequence"),
        }
    }

    #[test]
    fn parse_long_csi_param() {
        // The important part is the parameter, which is (i64::MAX + 1)
        const INPUT: &[u8] = b"\x1b[9223372036854775808m";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, ..) => assert_eq!(params, &[[u16::MAX]]),
            _ => panic!("expected csi sequence"),
        }
    }

    #[test]
    fn csi_reset() {
        const INPUT: &[u8] = b"\x1b[3;1\x1b[?1049h";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, intermediates, ignore, _) => {
                assert_eq!(intermediates, b"?");
                assert_eq!(params, &[[1049]]);
                assert!(!ignore);
            },
            _ => panic!("expected csi sequence"),
        }
    }

    #[test]
    fn csi_subparameters() {
        const INPUT: &[u8] = b"\x1b[38:2:255:0:255;1m";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, intermediates, ignore, _) => {
                assert_eq!(params, &[vec![38, 2, 255, 0, 255], vec![1]]);
                assert_eq!(intermediates, &[]);
                assert!(!ignore);
            },
            _ => panic!("expected csi sequence"),
        }
    }

    #[test]
    fn parse_dcs_max_params() {
        let params = "1;".repeat(params::MAX_PARAMS + 1);
        let input = format!("\x1bP{}p", &params[..]).into_bytes();
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, &input);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::DcsHook(params, _, ignore, _) => {
                assert_eq!(params.len(), params::MAX_PARAMS);
                assert!(params.iter().all(|param| param == &[1]));
                assert!(ignore);
            },
            _ => panic!("expected dcs sequence"),
        }
    }

    #[test]
    fn dcs_reset() {
        const INPUT: &[u8] = b"\x1b[3;1\x1bP1$tx\x9c";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 3);
        match &dispatcher.dispatched[0] {
            Sequence::DcsHook(params, intermediates, ignore, _) => {
                assert_eq!(intermediates, b"$");
                assert_eq!(params, &[[1]]);
                assert!(!ignore);
            },
            _ => panic!("expected dcs sequence"),
        }
        assert_eq!(dispatcher.dispatched[1], Sequence::DcsPut(b'x'));
        assert_eq!(dispatcher.dispatched[2], Sequence::DcsUnhook);
    }

    #[test]
    fn parse_dcs() {
        const INPUT: &[u8] = b"\x1bP0;1|17/ab\x9c";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 7);
        match &dispatcher.dispatched[0] {
            Sequence::DcsHook(params, _, _, c) => {
                assert_eq!(params, &[[0], [1]]);
                assert_eq!(c, &'|');
            },
            _ => panic!("expected dcs sequence"),
        }
        for (i, byte) in b"17/ab".iter().enumerate() {
            assert_eq!(dispatcher.dispatched[1 + i], Sequence::DcsPut(*byte));
        }
        assert_eq!(dispatcher.dispatched[6], Sequence::DcsUnhook);
    }

    #[test]
    fn intermediate_reset_on_dcs_exit() {
        const INPUT: &[u8] = b"\x1bP=1sZZZ\x1b+\x5c";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 6);
        match &dispatcher.dispatched[5] {
            Sequence::Esc(intermediates, ..) => assert_eq!(intermediates, b"+"),
            _ => panic!("expected esc sequence"),
        }
    }

    #[test]
    fn esc_reset() {
        const INPUT: &[u8] = b"\x1b[3;1\x1b(A";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Esc(intermediates, ignore, byte) => {
                assert_eq!(intermediates, b"(");
                assert_eq!(*byte, b'A');
                assert!(!ignore);
            },
            _ => panic!("expected esc sequence"),
        }
    }

    #[test]
    fn esc_reset_intermediates() {
        const INPUT: &[u8] = b"\x1b[?2004l\x1b#8";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 2);
        assert_eq!(dispatcher.dispatched[0], Sequence::Csi(vec![vec![2004]], vec![63], false, 'l'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Esc(vec![35], false, 56));
    }

    #[test]
    fn params_buffer_filled_with_subparam() {
        const INPUT: &[u8] = b"\x1b[::::::::::::::::::::::::::::::::x\x1b";
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, intermediates, ignore, c) => {
                assert_eq!(intermediates, &[]);
                assert_eq!(params, &[[0; 32]]);
                assert_eq!(c, &'x');
                assert!(ignore);
            },
            _ => panic!("expected csi sequence"),
        }
    }

    #[cfg(not(feature = "std"))]
    #[test]
    fn build_with_fixed_size() {
        const INPUT: &[u8] = b"\x1b[3;1\x1b[?1049h";
        let mut dispatcher = Dispatcher::default();
        let mut parser: Parser<30> = Parser::new_with_size();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Csi(params, intermediates, ignore, _) => {
                assert_eq!(intermediates, b"?");
                assert_eq!(params, &[[1049]]);
                assert!(!ignore);
            },
            _ => panic!("expected csi sequence"),
        }
    }

    #[cfg(not(feature = "std"))]
    #[test]
    fn exceed_fixed_osc_buffer_size() {
        const OSC_BUFFER_SIZE: usize = 32;
        const NUM_BYTES: usize = OSC_BUFFER_SIZE + 100;
        const INPUT_START: &[u8] = b"\x1b]52;";
        const INPUT_END: &[u8] = b"\x07";

        let mut dispatcher = Dispatcher::default();
        let mut parser: Parser<OSC_BUFFER_SIZE> = Parser::new_with_size();

        // Create valid OSC escape
        parser.advance(&mut dispatcher, INPUT_START);

        // Exceed max buffer size
        parser.advance(&mut dispatcher, &[b'a'; NUM_BYTES]);

        // Terminate escape for dispatch
        parser.advance(&mut dispatcher, INPUT_END);

        assert_eq!(dispatcher.dispatched.len(), 1);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(params, _) => {
                assert_eq!(params.len(), 2);
                assert_eq!(params[0], b"52");
                // The buffer holds the `;` separator too.
                assert_eq!(params[1].len(), OSC_BUFFER_SIZE - params[0].len() - 1);
                for item in params[1].iter() {
                    assert_eq!(*item, b'a');
                }
            },
            _ => panic!("expected osc sequence"),
        }
    }

    #[cfg(not(feature = "std"))]
    #[test]
    fn fixed_size_osc_containing_string_terminator() {
        const INPUT_START: &[u8] = b"\x1b]2;";
        const INPUT_MIDDLE: &[u8] = b"s\xe6\x9c\xab";
        const INPUT_END: &[u8] = b"\x1b\\";

        let mut dispatcher = Dispatcher::default();
        let mut parser: Parser<6> = Parser::new_with_size();

        parser.advance(&mut dispatcher, INPUT_START);
        parser.advance(&mut dispatcher, INPUT_MIDDLE);
        parser.advance(&mut dispatcher, INPUT_END);

        assert_eq!(dispatcher.dispatched.len(), 2);
        match &dispatcher.dispatched[0] {
            Sequence::Osc(params, false) => {
                assert_eq!(params[0], b"2");
                assert_eq!(params[1], INPUT_MIDDLE);
            },
            _ => panic!("expected osc sequence"),
        }
    }

    #[test]
    fn unicode() {
        const INPUT: &[u8] = b"\xF0\x9F\x8E\x89_\xF0\x9F\xA6\x80\xF0\x9F\xA6\x80_\xF0\x9F\x8E\x89";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 6);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('🎉'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('_'));
        assert_eq!(dispatcher.dispatched[2], Sequence::Print('🦀'));
        assert_eq!(dispatcher.dispatched[3], Sequence::Print('🦀'));
        assert_eq!(dispatcher.dispatched[4], Sequence::Print('_'));
        assert_eq!(dispatcher.dispatched[5], Sequence::Print('🎉'));
    }

    #[test]
    fn invalid_utf8() {
        const INPUT: &[u8] = b"a\xEF\xBCb";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 3);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('a'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('�'));
        assert_eq!(dispatcher.dispatched[2], Sequence::Print('b'));
    }

    #[test]
    fn partial_utf8() {
        const INPUT: &[u8] = b"\xF0\x9F\x9A\x80";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, &INPUT[..1]);
        parser.advance(&mut dispatcher, &INPUT[1..2]);
        parser.advance(&mut dispatcher, &INPUT[2..3]);
        parser.advance(&mut dispatcher, &INPUT[3..]);

        assert_eq!(dispatcher.dispatched.len(), 1);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('🚀'));
    }

    #[test]
    fn partial_utf8_separating_utf8() {
        // This is different from the `partial_utf8` test since it has a multi-byte UTF8
        // character after the partial UTF8 state, causing a partial byte to be present
        // in the `partial_utf8` buffer after the 2-byte codepoint.

        // "ĸ🎉"
        const INPUT: &[u8] = b"\xC4\xB8\xF0\x9F\x8E\x89";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, &INPUT[..1]);
        parser.advance(&mut dispatcher, &INPUT[1..]);

        assert_eq!(dispatcher.dispatched.len(), 2);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('ĸ'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('🎉'));
    }

    #[test]
    fn partial_invalid_utf8() {
        const INPUT: &[u8] = b"a\xEF\xBCb";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, &INPUT[..1]);
        parser.advance(&mut dispatcher, &INPUT[1..2]);
        parser.advance(&mut dispatcher, &INPUT[2..3]);
        parser.advance(&mut dispatcher, &INPUT[3..]);

        assert_eq!(dispatcher.dispatched.len(), 3);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('a'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('�'));
        assert_eq!(dispatcher.dispatched[2], Sequence::Print('b'));
    }

    #[test]
    fn partial_invalid_utf8_split() {
        const INPUT: &[u8] = b"\xE4\xBF\x99\xB5";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, &INPUT[..2]);
        parser.advance(&mut dispatcher, &INPUT[2..]);

        assert_eq!(dispatcher.dispatched[0], Sequence::Print('俙'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Print('�'));
    }

    #[test]
    fn partial_utf8_into_esc() {
        const INPUT: &[u8] = b"\xD8\x1b012";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 4);
        assert_eq!(dispatcher.dispatched[0], Sequence::Print('�'));
        assert_eq!(dispatcher.dispatched[1], Sequence::Esc(Vec::new(), false, b'0'));
        assert_eq!(dispatcher.dispatched[2], Sequence::Print('1'));
        assert_eq!(dispatcher.dispatched[3], Sequence::Print('2'));
    }

    #[test]
    fn c1s() {
        const INPUT: &[u8] = b"\x00\x1f\x80\x90\x98\x9b\x9c\x9d\x9e\x9fa";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 11);
        assert_eq!(dispatcher.dispatched[0], Sequence::Execute(0));
        assert_eq!(dispatcher.dispatched[1], Sequence::Execute(31));
        assert_eq!(dispatcher.dispatched[2], Sequence::Execute(128));
        assert_eq!(dispatcher.dispatched[3], Sequence::Execute(144));
        assert_eq!(dispatcher.dispatched[4], Sequence::Execute(152));
        assert_eq!(dispatcher.dispatched[5], Sequence::Execute(155));
        assert_eq!(dispatcher.dispatched[6], Sequence::Execute(156));
        assert_eq!(dispatcher.dispatched[7], Sequence::Execute(157));
        assert_eq!(dispatcher.dispatched[8], Sequence::Execute(158));
        assert_eq!(dispatcher.dispatched[9], Sequence::Execute(159));
        assert_eq!(dispatcher.dispatched[10], Sequence::Print('a'));
    }

    #[test]
    fn execute_anywhere() {
        const INPUT: &[u8] = b"\x18\x1a";

        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, INPUT);

        assert_eq!(dispatcher.dispatched.len(), 2);
        assert_eq!(dispatcher.dispatched[0], Sequence::Execute(0x18));
        assert_eq!(dispatcher.dispatched[1], Sequence::Execute(0x1A));
    }

    #[test]
    fn partial_utf8_then_invalid_keeps_the_text_between() {
        // "é" split after its first byte, then valid ASCII and an invalid byte.
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, b"\xC3");
        parser.advance(&mut dispatcher, b"\xA9a\xFF");

        assert_eq!(dispatcher.dispatched, vec![
            Sequence::Print('é'),
            Sequence::Print('a'),
            Sequence::Print('�'),
        ]);
    }

    #[test]
    fn partial_utf8_then_wide_char_and_invalid() {
        let mut dispatcher = Dispatcher::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, b"\xE6\xBC");
        parser.advance(&mut dispatcher, b"\xA2\xC3\xA9\xFF");

        assert_eq!(dispatcher.dispatched, vec![
            Sequence::Print('漢'),
            Sequence::Print('é'),
            Sequence::Print('�'),
        ]);
    }

    /// Records `print_ascii` runs separately from everything else.
    #[derive(Default)]
    struct AsciiRuns {
        inner: Dispatcher,
    }

    impl Perform for AsciiRuns {
        fn print_ascii(&mut self, bytes: &[u8]) {
            assert!(!bytes.is_empty());
            assert!(bytes.iter().all(|b| (0x20..0x7F).contains(b)), "{bytes:?}");
            let text = core::str::from_utf8(bytes).unwrap();
            self.inner.dispatched.push(Sequence::Osc(vec![b"ascii".to_vec(), text.into()], false));
        }

        fn print(&mut self, c: char) {
            self.inner.print(c);
        }

        fn execute(&mut self, byte: u8) {
            self.inner.execute(byte);
        }

        fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, c: char) {
            self.inner.csi_dispatch(params, intermediates, ignore, c);
        }
    }

    #[test]
    fn print_ascii_gets_whole_runs() {
        let run = |s: &str| Sequence::Osc(vec![b"ascii".to_vec(), s.into()], false);
        let mut dispatcher = AsciiRuns::default();
        let mut parser = Parser::new();

        parser.advance(&mut dispatcher, "ab\r\ncd\x1b[1mef\x7fgh é ij".as_bytes());

        assert_eq!(dispatcher.inner.dispatched, vec![
            run("ab"),
            Sequence::Execute(b'\r'),
            Sequence::Execute(b'\n'),
            run("cd"),
            Sequence::Csi(vec![vec![1]], Vec::new(), false, 'm'),
            run("ef"),
            Sequence::Print('\x7f'),
            run("gh "),
            Sequence::Print('é'),
            run(" ij"),
        ]);
    }

    /// Feeding input whole or one byte at a time gives the same actions, and
    /// so does the default `print_ascii`.
    #[test]
    fn ground_state_split_anywhere() {
        let inputs: &[&[u8]] = &[
            b"plain text\r\n",
            "mixed é漢😀 text\x1b[31m red ─┼─\x1b[0m".as_bytes(),
            b"bad \xFF utf8 \xC3( and \xE2\x82 cut \xE2\x82\x1b[m and \xF0\x9F\x98",
            b"\xC3\xA9a\xFF\xC3",
            b"c1 \x9b bytes \xC2\x9b too\x00\x07\x7f",
            b"\xE4\xBF\x99\xB5x\xED\xA0\x80y",
        ];
        for input in inputs {
            let mut whole = Dispatcher::default();
            Parser::new().advance(&mut whole, input);
            let mut split = Dispatcher::default();
            let mut parser = Parser::new();
            for b in input.iter() {
                parser.advance(&mut split, &[*b]);
            }
            assert_eq!(whole.dispatched, split.dispatched, "{input:?}");
        }
    }

    /// The CSI fast path gives the actions the state machine alone gives,
    /// whole or one byte at a time: private markers, intermediates,
    /// controls, ESC, CAN and too many parameters inside a CSI included.
    #[test]
    fn csi_fast_path_matches_the_state_machine() {
        let mut inputs: Vec<Vec<u8>> = [
            &b"\x1b[38;5;123mab\x1b[0m \x1b[48:2::1:2:3m\x1b[m\x1b[;m\x1b[1;;2H"[..],
            b"\x1b[?25h\x1b[?1049;2004l\x1b[>4;1m\x1b[1?2m\x1b[1 q\x1b[ 2q",
            b"\x1b[3\n1m\x1b[2\x1b[3m\x1b[4\x18m\x1b[5\x1am\x1b[\x7f6m\x1b[7\x7fm",
            b"\x1b[1;2;3;4;5;6;7;8;9;10;11;12;13;14;15;16;17;18;19;20;21;22;23;24;25;26;27;28;29;30;31;32;33m",
            b"\x1b[1:2:3:4:5:6:7:8:9:10:11:12:13:14:15:16:17:18:19:20:21:22:23:24:25:26:27:28:29:30:31:32:33m",
            b"\x1b[99999999999m\x1b[0\x9bm\x1b[1\xc3\xa9m\x1b[",
        ]
        .iter()
        .map(|s| s.to_vec())
        .collect();
        // Random strings over the bytes that matter in a CSI.
        let alphabet = b"\x1b[0123456789;:m?> !\x07\n\x18\x7fHJ\\]";
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..2000 {
            let mut s = Vec::new();
            for _ in 0..(x % 40) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                s.push(alphabet[(x % alphabet.len() as u64) as usize]);
            }
            inputs.push(s);
        }
        for input in &inputs {
            let mut reference = Dispatcher::default();
            let mut parser = Parser::new();
            parser.advance_reference(&mut reference, input);
            let mut whole = Dispatcher::default();
            Parser::new().advance(&mut whole, input);
            assert_eq!(reference.dispatched, whole.dispatched, "{input:?}");
            let mut split = Dispatcher::default();
            let mut parser = Parser::new();
            for b in input.iter() {
                parser.advance(&mut split, &[*b]);
            }
            assert_eq!(reference.dispatched, split.dispatched, "{input:?}");
        }
    }

    /// `digit_run` agrees with reading the digits one at a time, for every
    /// run length and digit, with any byte after them.
    #[test]
    fn digit_run_matches_a_byte_loop() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut cases: Vec<[u8; 8]> = vec![*b"0m......", *b"65535;..", *b"65536m..", *b"9999999m", *b"12345678", *b";1m....."];
        for _ in 0..20000 {
            let mut c = [0u8; 8];
            for b in c.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                // Mostly digits, with every other byte now and then.
                *b = if x % 4 != 0 { b'0' + (x >> 8) as u8 % 10 } else { (x >> 16) as u8 };
            }
            cases.push(c);
        }
        for c in &cases {
            let n = c.iter().position(|b| !b.is_ascii_digit()).unwrap_or(8);
            let v = c[..n].iter().fold(0u64, |v, &b| v * 10 + (b - b'0') as u64);
            let expected = (n != 0 && n != 8).then_some((n, v));
            assert_eq!(digit_run(c), expected, "{c:?}");
        }
    }

    #[test]
    fn printable_ascii_len_matches_a_byte_loop() {
        let naive =
            |b: &[u8]| b.iter().position(|&c| !(0x20..0x7F).contains(&c)).unwrap_or(b.len());
        // Every byte value at every position, in buffers of every length up
        // to 17, with printable filler around it.
        for len in 0..=17 {
            for pos in 0..len {
                for byte in 0..=255u8 {
                    let mut buf = [b'x'; 17];
                    buf[pos] = byte;
                    let got = printable_ascii_len(&buf[..len]);
                    assert_eq!(got, naive(&buf[..len]), "{byte} at {pos}");
                }
            }
        }
        // Two stop bytes in one word, where borrows could mislead.
        let stops = [0x00u8, 0x1F, 0x1B, 0x0A, 0x7F, 0x80, 0xC3, 0xFF, 0x20, 0x7E];
        for a in 0..8 {
            for b in 0..8 {
                for &x in &stops {
                    for &y in &stops {
                        let mut buf = [b'q'; 8];
                        buf[a] = x;
                        buf[b] = y;
                        assert_eq!(printable_ascii_len(&buf), naive(&buf), "{buf:?}");
                    }
                }
            }
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod string_tests;
