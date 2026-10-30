# Terminal Decline

![Terminal Decline — less overhead, more terminal](docs/brand/readme-header.png)

A GPU-accelerated terminal for macOS, written in Rust. It aims to use less
memory and CPU than Ghostty and Warp, has no UI of its own, and can turn any
program into its own Dock app.

## Status

Pre-release: there is no tagged release yet (the version is `0.0.1`), and
the repository and the Homebrew tap are private until the `0.1.0` beta.
The terminal itself is in daily use by its author; the Developer ID
signing, notarization and the in-app updater need the Apple and Sparkle
secrets, which are not set up yet, so builds are ad-hoc signed for now.
What `0.1.0` needs is in [docs/release-checklist.md](docs/release-checklist.md);
the road to 1.0 (performance work, the measured comparison with
Ghostty) is in [docs/roadmap.md](docs/roadmap.md). The first measured
comparison is below; the roadmap's targets against Ghostty are not all met
yet.

## Benchmarks

30 October 2026, Apple M1 Max, macOS 27.0. Terminal Decline at 6a38c0b
(release build), Ghostty 1.4.0, kitty 0.49.2, Warp 0.2026.10.21.08.25.02,
Terminal.app 2.15. Menlo 13, 100×30 cells, 10k lines of scrollback, one
window; memory is macOS physical footprint summed over all the terminal's
processes; medians of 3–4 interleaved runs on a Mac with other builds
running (load 1.7–10). Details, spreads and caveats:
[docs/comparison-2026-10-30.md](docs/comparison-2026-10-30.md); script:
[scripts/compare/run.py](scripts/compare/run.py).

| | idle memory | after 10k lines | 4 tabs | idle CPU | idle wakeups/s | `cat` 100 MB time | `cat` CPU (incl. WindowServer) |
|---|---|---|---|---|---|---|---|
| Terminal Decline | 31.5 MB | 39 MB | 63 MB | 0.02% | 0.4 | 0.61 s | 0.62 s |
| Ghostty | 67 MB | 68 MB | 98 MB | 0.64% | 16 | **0.53 s** | 1.68 s |
| kitty | 109 MB | 140 MB | 115 MB | 0.13% | 23 | 0.95 s | 1.31 s |
| Warp | 333 MB | 310 MB | 332 MB | 1.11% | 62 | 3.59 s | 4.38 s |
| Terminal.app | 48 MB | 64 MB | 139 MB | 0.03% | 0.7 | 3.97 s | 4.62 s |

Ghostty finishes the `cat` a little sooner. Idle CPU and wakeups are low
but not yet zero, and memory is 47–64% of Ghostty's, short of the 35%
target. Input latency was not measured (the probe needs Accessibility and
Screen Recording).

## Install

**Homebrew** (once the beta is out; the cask installs
`Terminal Decline.app` and the command as both `terminal-decline` and
`decline`, Apple Silicon only):

```sh
brew install --cask 0xdeafcafe/tap/terminal-decline
```

**From source, as a macOS app.** You need Rust (`rustup`, stable) and
the Xcode command line tools (`xcode-select --install`; the full Xcode
adds the Metal compiler, otherwise shaders compile at start). Then:

```sh
git clone https://github.com/0xdeafcafe/terminal-decline
cd terminal-decline
scripts/dogfood.sh --open
```

This builds a release, makes the app bundle (launcher, library and pin
stub), signs it and installs it as `~/Applications/Terminal Decline.app`,
then opens it. Drag it to the Dock to keep it there.

- To install somewhere else: `DECLINE_DOGFOOD_DIR=/Applications scripts/dogfood.sh --open`.
- To update: `git pull`, then `scripts/dogfood.sh` again. A running copy
  restarts into the new build and reopens its windows in the same folders
  and places.
- It is signed ad-hoc, so macOS asks again for permissions (files,
  notifications) after each rebuild. To keep them, sign with your own
  certificate: `DECLINE_DOGFOOD_IDENTITY="Developer ID Application: …" scripts/dogfood.sh`.
- The in-app updater stays off in these builds: updating is `git pull`.

**From source, as the bare command** (no app bundle, so no Dock pins made
from the app and no updater):

```sh
cargo install --locked --git https://github.com/0xdeafcafe/terminal-decline terminal-decline
```

macOS 14 (Sonoma) or later. Releases are Apple Silicon only; source builds
are tested on Apple Silicon. MIT licensed.

## Using it

```sh
terminal-decline              # your login shell
terminal-decline -e htop      # a program instead of the shell
terminal-decline pin Rush --icon rush.png -- ./rush   # Rush.app with its own Dock icon
terminal-decline settings     # the Settings screen, in any terminal (also `decline settings`)
```

## Driving it from agents and tests

`terminal-decline ctl` runs programs in real Terminal Decline terminals
without windows, for agents and tests to type into, read and measure:

```sh
terminal-decline ctl spawn --name top --size 100x30 -- htop
terminal-decline ctl wait-for top --text "Load average" --timeout 5000
terminal-decline ctl screen top            # the screen as text; --json for cells, colours, cursor, modes
terminal-decline ctl screenshot top top.png   # drawn on the GPU with your fonts and theme
terminal-decline ctl kill top
```

Each command prints a line of JSON and exits 0 when done (3 when
`wait-for` timed out); keys go through the app's own encoding, kitty
keyboard protocol included (`send top --key ctrl+c --text q`). It talks
to the running app, or starts a headless server with no Dock icon that
leaves once idle, over a socket only your user can reach;
`agent_api = false` turns it off. `terminal-decline ctl --help` lists
the commands; [docs/design/agent-api.md](docs/design/agent-api.md) has
the details.

## Features

What is in the code today:

- **Tabs and splits**: native macOS window tabs (Cmd+T, Cmd+1 to 9) and
  split panes with draggable dividers, each pane drawn in the window's one
  Metal layer.
- **Text**: ligatures (shaped with Core Text; `font_features` turns them
  off or picks features), font fallback and bold/italic faces, line height
  and cell width adjustment, true colour, underline styles, graphemes and
  emoji, box-drawing and Powerline glyphs drawn in code.
- **Links**: OSC 8 hyperlinks and plain URLs, opened with Cmd-click.
- **Find bar**: Cmd+F with a standard macOS search field over the whole
  scrollback; Cmd+G, Cmd+E and the shared find text work as in other apps.
- **Selection**: auto-scroll past the edge, rectangular selection
  (Option-drag), Shift-click, copy on select (off by default).
- **Keyboard**: the kitty keyboard protocol, the layout-independent
  Ctrl/Meta keys, and a `keybind` setting to remap, unbind or send text.
- **Shell integration**: OSC 133 prompt marks and OSC 7 for zsh, bash
  and fish, loaded into the login shell by the app (`shell_integration`;
  the snippets are in [shell/](shell/) to source by hand elsewhere): jump
  between prompts, select a command's output, Option-click to move the
  cursor, the title status and finish notifications. See
  [docs/shell-integration.md](docs/shell-integration.md).
- **Settings**: `~/.config/terminal-decline/config.toml`, reloaded when it
  changes; Settings... (Cmd+,) opens a settings screen in a tab, and
  `terminal-decline settings` (or `decline settings`; `prefs` works too)
  opens it in any terminal, over ssh or in Terminal.app, without the app
  running. Edit config.toml opens the file in a tab running your editor
  (the `editor` setting, else `$VISUAL`/`$EDITOR`, else nano, vim or vi).
- **Themes**: eight built in (Catppuccin, Gruvbox, Solarized, Tokyo Night,
  Slope) and your own in `~/.config/terminal-decline/themes/`.
- **Importers**: `terminal-decline import ghostty` turns a Ghostty config
  into ours; `terminal-decline import iterm FILE.itermcolors` turns an
  iTerm2 colour preset into a theme.
- **Dock pins**: `terminal-decline pin` makes a program its own Dock app
  with a generated icon; pins load the installed app, so they survive
  upgrades. Pin settings are in `~/.config/terminal-decline/pins/`.
- **macOS**: files and folders dropped on the Dock icon or sent from
  Finder and Services open a window there, notifications (OSC 9/777 and
  the bell) when unfocused, Dock progress, Secure Keyboard Entry.

Inline images: kitty graphics, iTerm2 (`imgcat`) and Sixel, on by default
(`images = false` turns them off). The in-app
updater (Sparkle) is built, but only offers releases that are signed and
notarized, so it has nothing to offer until the secrets are set. It was called Decline
before; the first run moves `~/.config/decline` to the new config folder.

See [DESIGN.md](DESIGN.md) for how it works, pressure tests and the
roadmap.

## Development

```sh
cargo test --workspace
cargo bench -p term-core            # parser throughput
cargo bench -p term-render          # frame building
scripts/pressure/run.py target/release/terminal-decline "$TMPDIR"
```

Commits follow Conventional Commits (see [CLAUDE.md](CLAUDE.md)); the
[changelog](CHANGELOG.md) is generated from them with `git cliff`.
