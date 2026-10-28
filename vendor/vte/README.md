vte
===

> **Vendored copy of vte 0.15.0 for decline**, used through
> `[patch.crates-io]` in the workspace `Cargo.toml`. Local changes, all in
> `src/lib.rs`:
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
