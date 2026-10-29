vte
===

> **Vendored copy of vte 0.15.0 for decline**, used through
> `[patch.crates-io]` in the workspace `Cargo.toml`. Local changes, in
> `src/lib.rs` (tests for the string handling are in `src/string_tests.rs`):
>
> - `Perform::print_ascii(&[u8])`: the ground state hands each run of
>   printable ASCII (0x20..=0x7E) over whole; C0 controls and DEL are
>   handled inline, and only non-ASCII bytes go through UTF-8 decoding. The
>   default implementation calls `print` once per byte, so other `Perform`
>   implementations behave as before. The end of a run is found 8 bytes at
>   a time (`printable_ascii_len`).
> - Fix: completing a UTF-8 character split across `advance` calls consumed
>   up to three bytes after it when an invalid byte followed (`"é"` sent as
>   `[0xC3]`, `[0xA9, b'a', 0xFF]` lost the `a`). A C1 control completed
>   that way is now executed, as it is when it arrives in one piece, rather
>   than printed.
>
> - APC strings (`ESC _ ... ST`) have a state of their own, with
>   `Perform::apc_start`, `apc_put(&[u8])` and `apc_end(complete)`. Data
>   arrives in slices found with `memchr3` for ESC, CAN and SUB; `complete` is
>   false if the string was abandoned (CAN, SUB, or ESC and anything but `\`).
>   The ST is consumed, not reported as an `esc_dispatch`. SOS and PM are
>   still ignored.
> - `Perform::put_slice(&[u8])`: DCS passthrough hands over runs of data
>   bytes whole. The default calls `put` per byte.
> - `Perform::dcs_end(complete)`: DCS passthrough has an escape state, as
>   APC does, so the end of a string is reported once the byte after an ESC
>   is known: `complete` is true for ST (`ESC \` or C1 `0x9C`), false if it
>   was abandoned (CAN, SUB, or ESC and anything but `\`). The ST is
>   consumed, not reported as an `esc_dispatch` (before, `ESC \` called
>   `unhook` at the ESC and then `esc_dispatch` for the `\`). The default
>   calls `unhook`, so other `Perform` implementations need no change.
> - OSC keeps its `;` separators in the raw buffer, so a parameter can be
>   rejoined (`params[2..].join(";")`) without loss. `osc_dispatch` is
>   unchanged: at most 16 parts, the rest of the string cut off. With a
>   fixed-size buffer (no `std`) the separators count against its size.
> - OSC size policy: a string passing 4 KB calls `Perform::osc_large(prefix)`
>   once, which answers `OscLarge::Buffer(cap)` (the default, 64 KB: past the
>   cap the string is dropped and not dispatched), `Stream` (the rest goes to
>   `osc_put`, then `osc_end(complete)`; no `osc_dispatch`) or `Drop`. Before
>   this the buffer was unbounded. APC and DCS data are not capped by the
>   parser.
>
> Run its tests with `cargo test --manifest-path vendor/vte/Cargo.toml`.

[![Build Status](https://travis-ci.org/alacritty/vte.svg?branch=master)](https://travis-ci.org/alacritty/vte)
[![Crates.io Version](https://img.shields.io/crates/v/vte.svg)](https://crates.io/crates/vte/)

Parser for implementing virtual terminal emulators in Rust.

The parser is implemented according to [Paul Williams' ANSI parser state
machine]. The state machine doesn't assign meaning to the parsed data and is
thus not itself sufficient for writing a terminal emulator. Instead, it is
expected that an implementation of the `Perform` trait which does something
useful with the parsed data. The `Parser` handles the book keeping, and the
`Perform` gets to simply handle actions.

See the [docs] for more info.

[Paul Williams' ANSI parser state machine]: https://vt100.net/emu/dec_ansi_parser
[docs]: https://docs.rs/crate/vte/
