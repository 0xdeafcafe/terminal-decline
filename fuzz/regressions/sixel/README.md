# Fuzz regressions: `sixel`

Minimised inputs that crashed the `sixel` fuzz target (the first byte picks
P1 and P2, the rest is the data after `q`). Each one is replayed on every PR,
with the target's checks, by `crates/term-image/tests/fuzz_regressions.rs`.

Name new ones `<issue number>-<short description>`, as in
`fuzz/regressions/parse/README.md`.
