# Preferences (Cmd+,): a terminal program Terminal Decline runs itself

Status: draft for review. Owner decisions (29 Oct 2026): preferences are a
TUI shown in a new tab, opened with Cmd+,; sectioned-list layout; the product
is renamed Terminal Decline (app, command `terminal-decline`, config dir
`~/.config/terminal-decline`, bundle id `red.forbes.terminal-decline`).

## Why a TUI

Zero UI of our own: the app draws nothing but terminals. Preferences are a
program running in one, built into the same binary (`terminal-decline
prefs`), so it costs no AppKit code, works over SSH-free local config only,
and doubles as a dogfood test of the terminal itself (mouse, colours, kitty
keys, resize).

## Opening it

- Cmd+, (menu: Terminal Decline > Settings…, the standard macOS place) opens
  a new tab in the front window running `<own executable> prefs --config
  <path>`; if a prefs tab is already open, it is brought to the front
  instead. A pin's Cmd+, edits that pin's settings file (with the user file
  shown read-only underneath as "inherited").
- The tab runs with a fixed title ("Settings") and closes when the program
  exits (q / Esc at the top level / Cmd+W).

## Layout (sectioned list)

```
┌ Sections ─────┐┌ Appearance ───────────────────────────────────────┐
│ ▸ Appearance  ││ Theme              Slope Dark            ▸        │
│   Font        ││ Follow light/dark  on                             │
│   Keys        ││ Opacity            1.00                           │
│   Window      ││ Blur               off                            │
│   Pins        ││ Padding            6                              │
│   Advanced    ││ Cursor style       block, no blink                │
└───────────────┘└───────────────────────────────────────────────────┘
 ↑↓ move  ←→ section  enter edit  / search  r reset  ? help  q quit
```

- Sections: Appearance (theme, light/dark pair, opacity, blur, padding,
  cursor), Font (family picker from installed monospace fonts, size,
  fallback list, bold/italic faces, line height, cell width), Keys
  (option-as-meta, keybind list with add/remove/rebind by pressing the
  chord), Window (columns, lines, scrollback, copy on select, confirm on
  close, ambiguous width), Pins (list pins: command, icon, open its
  settings, remove), Advanced (shell, command, env, image memory budget).
- Editors by type: toggle (space), choice list (enter → list, type to
  filter), number (←→ steps, or type), text, colour (hex with a swatch),
  list (add/remove/reorder), chord (press it).
- Mouse: click sections/rows, scroll; it's a terminal app using SGR mouse.
- Search: `/` filters all settings across sections by name and description.
- Every setting shows a one-line description at the bottom.

## Saving and live preview

- Edits write `config.toml` immediately (debounced ~150 ms) with
  `toml_edit`, preserving the user's comments, order and formatting; a
  value equal to the default is removed rather than written.
- The running app's live reload (E.9) applies the change to every window
  within a frame, so changing the theme or font previews instantly in the
  other tabs — and in the prefs tab itself.
- Settings that only apply to new windows are marked "new windows".
- Errors (e.g. a font that isn't installed) are shown inline, the same
  messages the app shows as notices.
- Undo: `u` reverts the last change; `r` resets the selected setting to
  default.

## Where the code lives

- `crates/term-prefs` (new library): the settings schema (one table:
  key, section, type, default, description, live or new-windows), the
  TOML read/write with `toml_edit`, and the TUI (ratatui + crossterm,
  or a small hand-rolled renderer if the dependency weight is a concern —
  reviewers to advise). No AppKit.
- The schema is the single source of truth: term-app's `Config` is
  generated from or checked against it (a test asserts every `Config`
  field has a schema entry and vice versa), and the generated config
  reference doc (E.9/Z.3) comes from it too.
- term-app: `prefs` subcommand → `term_prefs::run(path)`; Cmd+, opens the
  tab with that command; the font family picker needs the installed
  monospace font list (Core Text) — passed in via a small trait so
  term-prefs stays platform-free and testable.

## Testing

- Schema ↔ `Config` consistency test.
- `toml_edit` round trips: editing one value keeps comments and the rest
  byte-for-byte.
- TUI logic tests on a virtual buffer (ratatui's TestBackend): navigation,
  each editor type, search, undo/reset.
- Pipeline test: run `terminal-decline prefs` inside a headless session,
  send keys, check the file changed and the screen shows the new value.
- Smoke test: Cmd+, through a control command opens one prefs tab; a
  second Cmd+, focuses it.

## Rename to Terminal Decline (done before this)

- Bundle `Terminal Decline.app`, id `red.forbes.terminal-decline`; pin ids
  `red.forbes.terminal-decline.pin.<id>`; command `terminal-decline` with
  a `decline` alias (cask installs both; bare `cargo install` installs
  `terminal-decline`); config `~/.config/terminal-decline` (moved from
  `~/.config/decline` on first run, leaving a note), logs
  `~/Library/Logs/terminal-decline`, `TERM_PROGRAM=terminal-decline`,
  defaults domain follows the bundle id.
- Existing pins: `terminal-decline pin --update` re-creates each with the
  new id prefix (macOS sees new apps: permissions are asked again).
- The dogfood script installs `~/Applications/Terminal Decline.app` and
  removes the old `Decline.app`.

## Open questions for review

1. ratatui + crossterm vs a minimal hand-rolled TUI (binary size, compile
   time; the main binary is ~1.5 MB today).
2. Should prefs run in-process (a session whose "program" is a Rust
   function writing to the PTY) instead of spawning `<exe> prefs`? Saves a
   process (~10 MB) but couples crash domains.
3. Font picker: list all monospace fonts, or all fonts with a "not
   monospace" warning?
