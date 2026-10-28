# decline

A GPU-accelerated terminal for macOS, written in Rust. It aims to use less
memory and CPU than Ghostty and Warp, has no UI of its own, and can turn any
program into its own Dock app.

```sh
cargo install --locked --git https://github.com/0xdeafcafe/terminal-decline decline
decline                      # your login shell
decline -e htop              # a program instead of the shell
decline pin Rush --icon rush.png -- ./rush   # Rush.app with its own Dock icon
```

Settings: `~/.config/decline/config.toml` (font, colours, opacity, padding,
scrollback, `option_as_meta`, …). Pin settings: `~/.config/decline/pins/`.

See [DESIGN.md](DESIGN.md) for how it works, measurements against Ghostty,
pressure tests and the roadmap.

## Development

```sh
cargo test --workspace
cargo bench -p term-core            # parser throughput
cargo bench -p term-render          # frame building
scripts/pressure/run.py target/release/decline "$TMPDIR"
```

macOS 14+, Apple Silicon or Intel. MIT licensed.
