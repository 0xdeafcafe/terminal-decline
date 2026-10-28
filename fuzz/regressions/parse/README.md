# Fuzz regressions

Minimised inputs from crashes the `parse` fuzz target found. Each one is
replayed on every PR by `crates/term-core/tests/fuzz_corpus.rs`.

When the nightly fuzz run opens an issue, download the `minimized` input from
the run's artifacts, fix the bug, and commit the input here as
`<issue number>-<short description>` alongside the fix.
