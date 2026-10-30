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
Ghostty) is in [docs/roadmap.md](docs/roadmap.md). The
speed and memory goals are goals: published numbers will come with the
beta, not before.

## Install

**Homebrew** (once the beta is out; the cask installs
`Terminal Decline.app` and the command as both `terminal-decline` and
`decline`, Apple Silicon only):

```sh
brew install --cask 0xdeafcafe/tap/terminal-decline
```

**From source**, as the bare command, with no app bundle (so no Dock pins
made from the app, and no updater):

```sh
cargo install --locked --git https://github.com/0xdeafcafe/terminal-decline terminal-decline
```

**For development**, build the app bundle and install it as
`~/Applications/Terminal Decline.app`:

```sh
scripts/dogfood.sh --open
```

Run it again after a change: a running copy restarts into the new build
and reopens its windows in the same folders and places.

macOS 14 (Sonoma) or later. Releases are Apple Silicon only; source builds
are tested on Apple Silicon. MIT licensed.

## Using it

```sh
terminal-decline              # your login shell
terminal-decline -e htop      # a program instead of the shell
terminal-decline pin Rush --icon rush.png -- ./rush   # Rush.app with its own Dock icon
```

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
- **Shell integration**: OSC 133 snippets for zsh, bash and fish in
  [shell/](shell/) (sourced by you, not injected): jump between prompts,
  select a command's output, Option-click to move the cursor. See
  [docs/shell-integration.md](docs/shell-integration.md).
- **Settings**: `~/.config/terminal-decline/config.toml`, reloaded when it
  changes; Settings... (Cmd+,) opens a settings screen in a tab, and
  `terminal-decline prefs` opens it from any terminal.
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
