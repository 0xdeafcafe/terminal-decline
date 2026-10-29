# Terminal Decline

![Terminal Decline — less overhead, more terminal](docs/brand/readme-header.png)

A GPU-accelerated terminal for macOS, written in Rust. It aims to use less
memory and CPU than Ghostty and Warp, has no UI of its own, and can turn any
program into its own Dock app.

```sh
cargo install --locked --git https://github.com/0xdeafcafe/terminal-decline terminal-decline
terminal-decline              # your login shell
terminal-decline -e htop      # a program instead of the shell
terminal-decline pin Rush --icon rush.png -- ./rush   # Rush.app with its own Dock icon
```

The Homebrew cask installs `Terminal Decline.app` and the command as both
`terminal-decline` and `decline`.

Settings: `~/.config/terminal-decline/config.toml` (font, colours, opacity,
padding, scrollback, `option_as_meta`, …), edited by hand or with
Settings… (Cmd+,), which opens them in a tab (`terminal-decline prefs`
does the same in any terminal). Pin settings:
`~/.config/terminal-decline/pins/`. It was called Decline before; the first
run moves `~/.config/decline` there.

See [DESIGN.md](DESIGN.md) for how it works, measurements against Ghostty,
pressure tests and the roadmap.

## Development

```sh
cargo test --workspace
cargo bench -p term-core            # parser throughput
cargo bench -p term-render          # frame building
scripts/pressure/run.py target/release/terminal-decline "$TMPDIR"
```

macOS 14+, Apple Silicon or Intel. MIT licensed.
