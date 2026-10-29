# Preferences (Cmd+,): a terminal program Terminal Decline runs itself

Status: reviewed (Opus, 29 Oct 2026) and revised; build after the rename
(below), and the Pins section after P.2. Owner decisions (29 Oct 2026):
preferences are a TUI shown in a new tab, opened with Cmd+,; sectioned-list
layout; the product is renamed Terminal Decline (app, command
`terminal-decline`, config dir `~/.config/terminal-decline`, bundle id
`red.forbes.terminal-decline`). From the review: a hand-rolled TUI, run as
its own process, and a font picker that lists monospace fonts first (see
"Decisions from the review").

## Why a TUI

Zero UI of our own: the app draws nothing but terminals. Preferences are a
program running in one, built into the same binary (`terminal-decline
prefs`), so it costs no AppKit code and doubles as a dogfood test of the
terminal itself (mouse, colours, kitty keys, resize). `config.toml` stays
the source of truth: everything the TUI does is an edit to it, and the file
can always be edited by hand.

Two things follow from living in a tab:

- While the prefs tab is in front, the other tabs of its window are hidden
  and not drawn, so the preview most people see is the prefs tab itself
  (other windows show it too). The tab must stay usable whatever is set
  (see "Keeping the prefs tab usable").
- VoiceOver can't read the terminal in 1.0 (roadmap decision 6), so it
  can't read this TUI either. The accessible way to change settings is the
  file in any editor, one menu item away (see "Accessibility").

## Opening it

- Terminal Decline > Settings… (Cmd+,; a new keybind action `settings`,
  default `cmd+,`) opens a new tab in the front window running the prefs
  program. If this process already has a prefs tab, that tab is brought to
  the front instead (the app remembers which session it opened for
  Settings). In a pin, the pin's menu item edits the pin's own file, with
  the user's file underneath as "inherited". Beside it, "Edit config.toml"
  opens the file in the default text editor (`open -t`), for anyone who
  prefers the file or can't use the TUI.
- The program is started directly, not through the login shell: term-pty's
  `Command` gets a `direct` field (exec the absolute path, instead of
  `$SHELL -l -c 'exec "$0" "$@"'`). That skips the user's `.zprofile`
  (30–50 ms here, more with heavy profiles), and Settings opens even when
  the login shell is broken or doesn't accept `$0` / `$@` (to check with
  fish, which is on the daily-driver list).
- It runs the main app's binary, not `current_exe()`: in a pin that is the
  pin's own clone today and a launcher that never changes after P.2. The
  app passes everything as arguments:
  `terminal-decline prefs --layer <file>... --edit <n> --in-app`. The
  layers are the files the window's config was read from
  (`config::Source`: the user's config, a pin's file, `--config`), `--edit`
  names the one being edited, and `--in-app` says the app is there to
  record chords (below). Run by hand in another terminal,
  `terminal-decline prefs` edits the user's file and chords are typed.
- The app gives the tab a fixed title ("Settings"; titles the program sets
  are ignored) and keeps it opaque. The tab closes when the program exits:
  `q`, Esc at the top level, or Cmd+W (the program gets SIGHUP; nothing is
  waiting to be written, see "Saving", so nothing is lost). With
  `confirm_close` on, the prefs tab is never the reason to ask: closing it
  or quitting doesn't count it as a running program.
- If the program panics, it restores the terminal (leaves the alternate
  screen, mouse reporting and kitty flags), prints the message and waits
  for a key, so the tab doesn't just vanish. The crash log is written as
  for the app.

## Layout (sectioned list)

```
┌ Sections ─────┐┌ Appearance ───────────────────────────────────────┐
│ ▸ Appearance  ││ Theme             slope-dark          ▸           │
│   Font        ││ Light/dark pair   off                             │
│   Keys        ││ Colours           from the theme      ▸           │
│   Window      ││ Opacity           1.00                            │
│   Pins        ││ Blur              off                 new windows │
│   Advanced    ││ Padding           6                               │
└───────────────┘└───────────────────────────────────────────────────┘
 ↑↓ move  ←→ section  enter edit  / search  r reset  ? help  q quit
```

- Sections hold only settings that exist (the schema test keeps it so;
  later settings get entries when they are built):
  - Appearance: theme (a name, or a light/dark pair), colours (foreground,
    background, cursor, selection and the 16-colour palette, over the
    theme's), opacity, blur (new windows), padding.
  - Font: family (picker, see "Fonts"), size, fallback list, bold / italic
    / bold italic faces, line height, cell width.
  - Keys: option as meta, and the keybind list (add, remove, rebind by
    pressing the chord or typing it).
  - Window: columns, lines, scrollback (new windows), copy on select,
    confirm before closing.
  - Pins: see "Pins". Not shown while editing a pin's own file.
  - Advanced: command, cwd, env (new windows); in a pin, its title.
- Editors by type: toggle (space), choice list (enter → list, type to
  filter), number (←→ steps within the schema's range, or type), text
  (committed with Enter), colour (hex with a swatch), list
  (add/remove/reorder), chord (press it, or type it).
- Each row says where its value comes from: set in this file, "default",
  "inherited from config.toml" (a pin's file over the user's), or
  "overridden by <file>" (a `--config` layer on top); and it marks "new
  windows" settings.
- Mouse: click sections and rows, scroll; SGR mouse. The program pushes
  kitty flag 1 (disambiguate), so Esc, Alt+key and Ctrl+Enter are
  unambiguous.
- Search: `/` filters all settings across sections by name and
  description. Every setting shows a one-line description at the bottom.
- `e` opens the file being edited in `$EDITOR` in this tab; the TUI comes
  back when the editor exits.
- The program blocks on input: no timers and no polling loops, so an open
  prefs tab costs 0 wakeups like any idle tab. It sends no startup queries
  that wait for a reply.
- Ctrl+L repaints everything (Cmd+K clears the screen and sends ^L), as
  does a resize (a font change resizes the tab).

## Keeping the prefs tab usable

When the tab is in front it is the preview, so settings that break text
must not lock the user out:

- Numbers have ranges in the schema (for example font size 6–72, line
  height 0.5–3).
- The app keeps the prefs view opaque whatever `opacity` says.
- The TUI draws its frame, labels and selection in colours it picks to
  contrast with the configured background, so foreground = background or a
  dark-on-dark palette can't hide it. A preview strip shows sample text in
  the default foreground and background and the 16 palette colours: that
  is the live preview.
- `u` (undo) works without reading the screen; Esc cancels an edit in
  progress.

## Saving

Each committed edit is written at once; there is no debounce. The app's
watcher already waits 40 ms after an event and compares contents, so a
debounce would only delay the preview (to about 190 ms) and lose the last
edit when Cmd+W closes the tab. While a key repeats on a number, the
program writes at most 10 times a second, and always the final value.

Every write:

1. Reads the file again and applies only this edit to what it finds, so
   edits made meanwhile in another editor are kept.
2. Edits with `toml_edit`, keeping comments, order and formatting
   (behaviour checked with toml_edit 0.25.17):
   - the new value takes over the old one's decor (its inline comment and
     spacing); plain assignment drops it, so `font_size = 13 # big` would
     become `font_size = 14`;
   - a new table is made as a standard `[table]` (`toml_edit::table()`);
     indexing a missing one would write an inline `keybind = { … }`;
   - a file with CRLF line endings is written back with CRLF (toml_edit
     writes LF);
   - integers stay integers (`13` → `14`, not `14.0`).
3. Writes atomically: a temp file in the same folder, renamed over the
   target, keeping its permissions. A config.toml that is a symlink (into a
   dotfiles repo, say) is written at its target (`canonicalize`), so the
   link stays a link.

Never written:

- A file that doesn't parse. The TUI opens it read-only, shows toml_edit's
  message with the line and column, and offers `e`.
- Anything on load: opening the TUI changes nothing (no reformatting, no
  respelling of chords).

Removing:

- A value is never removed just because it equals the default. That would
  drop the user's comments with it, and in a pin's file it changes the
  meaning: removing `font_size = 13` there makes the pin inherit the user's
  16.
- `r` resets: it removes the key from the file being edited, so the value
  comes from the layers below (the defaults for the user's file; the
  defaults and the user's file for a pin). The status line says that the
  key's comment goes with it. In a pin's file `command` can't be reset (a
  pin with no command opens a login shell), and `pin_app` isn't shown.
- `u` undoes the last change by putting back the previous item with its
  comments (toml_edit items carry them), applied to the file as it is now.

The program also watches the file it edits, and the layers below, with the
app's `watch` module, and refreshes when they change; it ignores a change
whose contents equal its own last write.

## Live preview and the app's reload

The app's reload (`watch`, `AppDelegate::reload_config`) applies each
write: colours, fonts, padding, opacity, option as meta, copy on select and
keybinds at once in every window (the menus are rebuilt), the rest to new
windows. There is no loop: the app never writes config files, and a write
that changes nothing reloads nothing (the watcher compares contents). A
grid that changes size sends SIGWINCH; one that doesn't sends nothing (the
tty only signals when the size changes).

The reload's notices need one change first. Today `reload_config` hands
every error in the new config, plus "X changed: it applies to new
windows", to every window, and `session::write_late_notice` writes them
into the terminal as if the program had printed them. With the TUI, each
step on `columns`, and every edit while the file has any old error, would
write red lines into every window: Claude Code, vim and the prefs tab
itself have their cursor moved and their colours reset underneath them,
and stay garbled until they redraw. The change:

- While a prefs session is open, reloads write no notices into windows;
  the TUI shows the same messages, computed with the same loader.
- Otherwise: only errors that are new since the last load; never the "new
  windows" line (for hand edits too); and nothing into a terminal that is
  in the alternate screen.

The TUI checks each edit with the app's own loader before writing (see
"Where the code lives"), so its writes add no errors; fonts that aren't
installed are caught by the picker.

## Keybinds

- `[keybind]` holds changes against `keybind::DEFAULTS`: removing a
  default writes `"chord" = "none"`, and restoring one deletes the entry.
- Entries are found by their parsed chord, not their text: `"command+K"`
  and `"cmd+k"` are the same key, and when both are in a file the later
  spelling wins (`keybind::canonicalize`). An edit rewrites the existing
  entry in place and never adds a second spelling.
- Conflicts are shown from the bindings in effect (`Keybinds`): "Cmd+T is
  New Tab: replace?". Binding the chord of a fixed item (Quit, Hide,
  Minimize, Cmd+1…9) takes it from that item (`keeps_fixed`), and the TUI
  says so.

### Recording a chord

A program can't receive Cmd chords in Terminal Decline: Cmd stays with the
menus whatever the kitty flags, bound Cmd chords are matched by the menus
before the view sees them, and unbound ones are dropped
(`KeyAction::ToMenu`). Option chords go to the input method (Option+E is a
dead key) unless option as meta is on. So recording goes through the app:

1. The prefs program sends `OSC 7771 ; record-chord ST`. The app honours it
   only in the session it opened for Settings (a flag on that session, set
   by the Cmd+, path, which programs can't set), so a program in another
   tab can't swallow Cmd+W or learn which keys are pressed. Anywhere else
   it is ignored like any unknown OSC.
2. The app adds a local key-down event monitor
   (`NSEvent addLocalMonitorForEventsMatchingMask:handler:`), which sees
   keys before the menus, the input method and `[keybind]`. It takes the
   first key-down in that window that isn't a lone modifier, swallows it,
   and removes the monitor. Plain Esc, the window losing focus, a tab
   switch or 10 seconds cancel.
3. The app replies through the PTY input, like a query reply:
   `OSC 7771 ; chord ; <chord> ST`, spelled by `Chord::of_event` and its
   `Display` (`ctrl+alt+shift+cmd+k`), or `OSC 7771 ; cancelled ST`. Using
   the app's own chord code means the recorded chord is exactly the one
   `Keybinds::resolve` matches, on non-Latin layouts (the ASCII-capable
   layout's character) and for shifted characters (`cmd+}`).
4. Shortcuts macOS keeps (Cmd+Tab, Cmd+Space, the screenshot keys, input
   source switching) never reach the app: the TUI says so, and a chord can
   always be typed instead (`cmd+shift+k`), checked by the same parser.
   Typing is also how chords are entered without `--in-app`.

Not the kitty protocol: the app never sends Cmd to programs, and doing it
for one session would still need the menus bypassed and kitty's codes
turned back into the app's chord spelling, which `Chord::of_event` already
does.

## Pins

- The section lists the pins (`pin::list`): name, command, icon. Enter
  opens that pin's file in the same editor, layered over the user's file.
- It is built on the pin layout P.2 makes (a launcher plus the app's
  library), which comes first.
- Remove calls `pin::remove`, asks to confirm, refuses while that pin is
  running (its executable among the running processes), and says that its
  Dock tile stays.
- Changing the icon re-signs the pin; until P.2 its permissions are asked
  again, and the TUI says so.
- A running pin watches its own files, so edits apply in that pin's
  windows at once.
- `pin_app` is hidden, `command` can't be reset, and `title` is under
  Advanced. Making pins stays with `terminal-decline pin`.

## Fonts

On Alex's Mac 187 font families are installed, and only 5 carry Core
Text's monospace flag (Andale Mono, Courier New, Menlo, Monaco, PT Mono);
one more (Ayuthaya) has equal ASCII advances without it. A monospace-only
list would hide fonts that don't set the flag. So the family picker shows
first the families with the flag or with equal advances for `iMW0.`, and a
"show all fonts" toggle adds the rest, marked "not monospace: text will
look uneven". The fallback list picks from all fonts (fallbacks such as
Apple Color Emoji or a Nerd Font symbols font aren't monospace). Typed
names are accepted and checked with `Fonts::missing`. The scan takes
90–280 ms, so it runs when the picker opens, not at start.

## Accessibility

VoiceOver isn't in 1.0 (roadmap decision 6): it can't read the terminal,
so it can't read this TUI. What the TUI does meanwhile:

- "Edit config.toml" sits beside Settings… in the menu, and `e` in the
  TUI: the file in any editor is the accessible way.
- The terminal's real cursor sits on the selected row, so Zoom (and
  VoiceOver, later) can follow it.
- The selection is marked with ▸ and reverse video, and errors with the
  word "error:", never with colour alone.
- Everything works from the keyboard; the mouse is optional.

## Where the code lives

- `crates/term-prefs` (new library, no AppKit or other platform code): the
  settings schema, the TOML edits with `toml_edit`, and the TUI. term-app
  depends on it; it can't depend on term-app, so what it needs from the
  app comes through a trait the app implements:
  - check a file's text with the app's own loader (`config::load` with the
    layers) and return the notices, so the TUI's messages are exactly the
    app's;
  - the values from the layers below the edited file (from
    `Config::default()` and `config::load`, as TOML);
  - the installed fonts with their monospace flag, and the theme names
    (built-in and the themes folder);
  - the pins: list and remove (the pin module).

  term-prefs' tests use a fake.
- The TUI is hand-rolled (decision 1): a cell grid compared with the last
  frame, box drawing, a list, a table, a text field and the editors above;
  an input decoder for CSI, SS3, SGR mouse, kitty `CSI … u` and bracketed
  paste; raw mode through termios. Character widths come from term-core's
  table, the same the terminal draws with. About 1–1.5k lines.
- term-app: the `prefs` subcommand (handled in `main` before the config is
  read, like `pin` and `import`) calls `term_prefs::run` with its
  arguments and the trait; the Settings… and Edit config.toml menu items;
  the chord recording; the change to the reload's notices.

### The schema

The schema checks `Config` rather than generating it: `Config` has custom
deserializers (`line_height`, `cell_width`), a computed field
(`palettes`), an internal one (`pin_app`), maps (`env`, `[keybind]`) and a
value that is a string or a table (`theme`), which a generator would have
to special-case for little gain.

- An entry: key, section, editor kind (with its range or choices), live or
  new windows, description, hidden.
- Defaults aren't repeated in the schema: they come from `Config::default()`
  (Config derives Serialize, skipping `palettes`).
- `config::new_windows_only`, a hand-written list today, reads the schema's
  live / new-windows flag.
- The generated config reference comes from the schema.

## Cost

Measured 29 Oct 2026 on an M1 Max, release profile (thin LTO, one codegen
unit, unwinding), with a stub `prefs` that draws this layout and edits a
file:

| added to the app binary (2.16 MB today) | size |
|---|---|
| toml_edit | +111 KB |
| toml_edit + hand-rolled TUI | +151 KB |
| toml_edit + ratatui-core and ratatui-widgets, own backend | +381 KB |
| toml_edit + ratatui + crossterm | +516 KB |

- Clean release build: the same wall time within noise (52 s); CPU time
  88 → 118 s with ratatui + crossterm, which add 77 crates to Cargo.lock.
- Footprint of a running prefs program: 2.8 MB (ratatui + crossterm) or
  2.1 MB (hand-rolled) from the app binary; 1.9 / 1.3 MB from a binary
  that doesn't link AppKit. For comparison, the login zsh in a normal tab
  is 5.1 MB, and `/bin/cat` 1.3 MB.
- Idle with the tab open: 0.0% CPU, 0 wakeups, one thread (both stubs).

The real crate (`crates/term-prefs`, 30 Oct 2026, same machine and
profile) is bigger than the stub: in a small binary that links only it
(`cargo bloat --release -p term-prefs --example fake --crates`),
term-prefs is 171 KB of code, toml_edit and toml_parser 114 KB, and std
323 KB, much of which the app links already.
Stripped, that binary is 913 KB against 331 KB for an empty one. So the
app should grow by about 300–350 KB rather than 151 KB; the exact number
is measured when `prefs` is wired into the app.

## Testing

- Schema and `Config`: the field names of Config and Colors, from a small
  deserializer that records the `fields` serde passes to
  `deserialize_struct`, equal the schema's keys; each schema default,
  written as a one-key file, loads with no errors and equals
  `Config::default()`; the new-windows flags match the reload's.
- `toml_edit` edits on a set of files: inline and leading comments, dotted
  keys, inline tables, quoted keys, CRLF, a BOM, no final newline,
  multi-line arrays with comments, chords spelled several ways; and an
  empty, missing, symlinked, read-only and unparsable file (the last left
  untouched).
- Property test: random edits on random valid files still parse,
  `config::load` gives the expected values with no errors, and the lines
  not edited are the same byte for byte.
- Screens: the TUI's output fed into term-core's `Terminal` (a real
  terminal as the test screen) at 80×24, at 20×5 and after a resize:
  navigation, each editor, search, undo and reset, the read-only mode.
- Chords: the reply text for `KeyInput`s (plain, in `keybind`); finding
  and replacing an entry by its parsed chord.
- Pipeline: `terminal-decline prefs` through `session` with a temp
  `XDG_CONFIG_HOME`: keys in, the file and the screen checked; exit 0 on
  `q`; SIGHUP in the middle of an edit leaves a valid file.
- Smoke, through a control verb `settings`: Cmd+, opens one prefs tab and
  a second focuses it; an edit recolours another window (the dump's
  foreground and background) and writes no notice text into it; recording
  a chord: arm it, post a Cmd key with `CGEventPostToPid` (as the latency
  harness does), and check the reply and that the menu action didn't run;
  the prefs program's footprint and its 0 idle wakeups are recorded.
- The rename's tests are listed with its steps.

## Rename to Terminal Decline

Names: bundle `Terminal Decline.app`, id `red.forbes.terminal-decline`;
pin ids `red.forbes.terminal-decline.pin.<id>`; command `terminal-decline`
with a `decline` alias (the cask installs both; `cargo install` installs
`terminal-decline`); config `~/.config/terminal-decline` (moved from `~/.config/decline` on
first run, which then links to it); logs
`~/Library/Logs/terminal-decline`; `TERM_PROGRAM=terminal-decline`; the
defaults domain follows the bundle id.

### When

Now (roadmap N.10), before Developer ID and notarized builds go out (N.4),
P.1/P.2 and Sparkle (B.4). P.2's launcher never changes once a pin is made
and it finds the app, so it must be written for the new name (finding the
app by bundle id); a later rename would change every pin's bundle id and
ask for its permissions again, which is what P.2 is there to stop; and
Sparkle expects updates to come from the same app.

### Steps, in order

1. A small commit first, dogfooded before the rename: a dogfood build whose
   own bundle has gone relaunches `Terminal Decline.app` from the same
   folder with `--restore`. Today `restart_into_new_build` finds its bundle
   through `current_exe().canonicalize()`, which fails once `Decline.app`
   is deleted, and returns without doing anything, so the windows wouldn't
   come back.
2. Names in the build: package and binary `terminal-decline` (`-p decline`
   and jq's `select(.name=="decline")` in dogfood.sh, release.yml and
   weekly.yml; `CARGO_BIN_EXE_decline` in the smoke and pipeline tests;
   `target/release/decline` in the pressure and latency scripts).
   bundle.sh makes `Terminal Decline.app` with the new id, name and
   executable. Paths now have a space, so quote them (release.yml's ditto,
   notarytool, stapler and dwarfdump lines and dogfood.sh's `pgrep -f` are
   unquoted today).
3. Names in the code: `TABBING_ID`; the os_log subsystem and the logs
   folder (crash.rs); `ID_PREFIX` (pin.rs); the restore file's folder
   (reload.rs); USAGE, `-V`, the default window title, the notification
   fallback title, the `decline:` notice prefix, and the "could not start"
   and crash texts; the latency harness's `red.forbes.decline.latency`;
   `TERM_PROGRAM` and its tests (term-pty); DESIGN.md, README.md, AGENTS.md
   and CLAUDE.md (the dogfood path). `git grep -i decline` lists 40 files;
   each hit is renamed or kept on purpose. The `DECLINE_*` environment
   variables (test and latency hooks) are decided here too.
4. Pin detection: `own_pin_config` compares the canonical executable's name
   (the `decline` alias symlink reports `decline`) and accepts both names as
   the app; `check_name` refuses pins whose id would be `decline` or
   `terminal-decline`.
5. Migration at first start, in `main` before any subcommand (pin, import,
   prefs):
   - Config folder: if only `~/.config/decline` exists, rename it to
     `terminal-decline` and leave a symlink `decline -> terminal-decline`,
     so pins not yet updated (old cloned binaries reading
     `~/.config/decline/pins/<id>.toml`) and the user's own scripts keep
     working. If the old folder is itself a symlink, make the new one a
     symlink to the same target and leave the old one. If both exist, use
     the new one and say so once; never merge. `XDG_CONFIG_HOME` is
     honoured for both. Two processes starting together are fine: the
     rename is atomic, and on EEXIST or ENOENT it looks again.
   - Defaults: copy `SecureKeyboardEntry` from the `red.forbes.decline`
     domain (`CFPreferencesCopyAppValue`) when the new domain has none.
   - Logs: new crash logs go to the new folder; old ones stay where they
     are.
6. Pins: `pin --update` regenerates each pin's Info.plist before signing
   (today it only re-clones and re-signs, so `CFBundleIdentifier` would
   keep the old prefix while codesign gets the new identifier): the name
   from the bundle's file name, the icon if `Resources/icon.icns` exists,
   the new id prefix. It copies the pin's `SecureKeyboardEntry` the same
   way. dogfood.sh runs it once. Dock tiles are stored by path, so they
   keep working. The app and every pin ask again for their permissions
   (files and folders, Full Disk Access, notifications): the release notes
   say so. Alex's Dock tile for `Decline.app` is replaced by hand, and
   dogfood.sh runs `lsregister -u` on the old path.
7. `TERM_PROGRAM=terminal-decline`: no program treats `decline` specially
   (Claude Code picks a notification channel only for iTerm.app, kitty,
   ghostty and Apple_Terminal; tools such as yazi or supports-hyperlinks
   go by known names), so nothing changes for them; Alex's shell files are
   checked for `decline`. This is the string to keep for good: once OSC 8
   links land, those projects can be asked to recognise it.
8. Homebrew tap (`0xdeafcafe/homebrew-tap`), in one commit:
   `Casks/terminal-decline.rb`, `git rm Casks/decline.rb`, and
   `cask_renames.json` with `{"decline": "terminal-decline"}` so an
   existing install follows on `brew update`. The cask
   (`packaging/homebrew/terminal-decline.rb`): `app "Terminal
   Decline.app"`; `binary` for `terminal-decline`, and again with
   `target: "decline"`; `zap` both old and new paths (config, logs,
   caches, the plist and saved state for both ids); a caveat to run
   `terminal-decline pin --update`. release.yml writes the new file name.
9. CI artifacts: GitHub turns spaces in release asset names into dots,
   which would break the cask URL, so the zips are
   `Terminal-Decline-<version>-macos-arm64.zip` and
   `…-macos-arm64.dSYM.zip`. The dSYM becomes `terminal-decline.dSYM`
   (cargo names it after the binary): the `cp -RL` and `dwarfdump` lines
   follow, as do the upload-artifact name and the release title.
10. The GitHub repo is already `0xdeafcafe/terminal-decline` (origin, and
    `repository` in Cargo.toml): nothing to rename.

Tests: the config folder migration (old only, new only, both, the old one
a symlink, two processes at once); `own_pin_config` with both names and the
alias; `pin --update` rewriting Info.plist (with the recording runner); the
defaults copy as a plain function.

## Decisions from the review

1. Hand-rolled TUI, not ratatui + crossterm: +40 KB against +405 KB (see
   "Cost"); the UI is one fixed layout and a few editors, and term-core can
   be the test screen. If building quickly matters more, ratatui-core +
   ratatui-widgets with our own backend and input decoder (+269 KB) is the
   fallback. crossterm stays out either way (mio, signal-hook, and its
   startup probes).
2. Its own process, not in-process: it costs 2.1–2.8 MB, and only while the
   tab is open (not the ~10 MB first guessed). In-process would need a
   session without a real program (term-pty is built on forkpty), couldn't
   use SIGWINCH (signals go to the whole process), and a hang or leak in
   the TUI would sit in the process holding every window (panics are caught
   per session; hangs aren't). A separate binary without AppKit would save
   only 0.8 MB.
3. Font picker: monospace fonts first, all fonts behind a toggle (see
   "Fonts").

## Later

- A single-column layout, for when VoiceOver can read the terminal.
- A problems list: unknown keys with "did you mean font_size?".
- Import from Ghostty or iTerm2 (`import … --write`) under Advanced.
