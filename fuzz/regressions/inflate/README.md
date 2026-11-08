# Fuzz regressions: `inflate`

Inputs that crashed the `inflate` fuzz target (the first byte is the slice
size less one, the rest the zlib data). Each one is replayed on every PR, with
the target's checks, by `crates/term-image/tests/fuzz_regressions.rs`.

Name new ones `<issue number>-<short description>`, as in
`fuzz/regressions/parse/README.md`.
