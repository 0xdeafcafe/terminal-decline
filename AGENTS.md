# Notes for agents working on this repo

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <summary in the imperative, lower case, no full stop>

<body: what changed and why, wrapped at 72 columns>
```

- Types: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci`,
  `chore`. Use `perf` for changes whose point is speed or memory.
- Scopes are the modules: `core` (term-core), `pty` (term-pty), `font`
  (term-font), `render` (term-render), `app` (term-app, the `terminal-decline`
  binary), `pin` (Dock pins), plus `ci`, `deps`, `docs` where they fit.
  Leave the scope out for changes across many modules.
- Breaking changes: `feat(core)!: …` and a `BREAKING CHANGE:` footer.
- One logical change per commit. Before committing, run `cargo fmt --all`,
  `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace`, and don't commit if any of them fail.

## Working here

- Read `DESIGN.md` first; update it when behaviour or measurements change.
- Benchmarks before and after only for changes whose point is speed or
  memory, or that touch the parser, frame building or the GPU path; other
  changes need fmt, clippy and tests only. DESIGN.md is brought up to date
  at the end of each roadmap phase, not in every PR (design docs under
  docs/design still get their "Landed" notes).
- Performance claims need numbers: `cargo bench -p term-core`,
  `cargo bench -p term-render`, `scripts/pressure/run.py`, and
  `/usr/bin/footprint -p <pid>` for memory (not RSS).
- `term-core` must stay free of platform code.

## Dogfooding

Alex uses `~/Applications/Terminal Decline.app`, installed by `scripts/dogfood.sh`.
After merging changes to `main` that affect the app, run
`scripts/dogfood.sh`: the running copy restarts into the new build and
reopens its windows in the same folders and places. The bundle is the
launcher, `Frameworks/libterminal_decline.dylib` and the pin stub in
`Helpers/`; the restart watches the library. It then runs `pin --repair`,
which rebuilds clone pins as stub pins (asking first, since macOS resets a
rebuilt pin's permissions). Test pins go in a temp dir with a temp `HOME`,
never in `~/Applications`.

## Smoke tests open real windows

The app smoke tests (`--run-ignored only`, `binary(smoke)`) open real
windows on Alex's screen. Don't run them locally: CI runs them on every
push to main and on `ci/**` branches (push one to check a change). Run a
single smoke test locally only when debugging it, and say so.

## Agents in worktrees

Each worktree builds its own `target/` (several GB). When your work is
committed, run `rm -rf target` in your worktree before reporting, so the
disk doesn't fill up while several agents run.
