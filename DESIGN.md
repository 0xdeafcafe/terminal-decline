# Terminal Decline — design

A GPU-accelerated terminal for macOS, written in Rust. Goal: lower memory and
CPU use than Ghostty and Warp, with zero UI of its own, and the ability to pin
any command as its own Dock app.

## Goals

- **Idle costs nothing.** No timers, no redraws, no wakeups while nothing
  changes. Cursor blink (when an app asks for it) is the only timer, and it
  stops when the window is hidden or unfocused.
- **Small memory.** Measured as macOS *physical footprint* (what Activity
  Monitor shows; RSS misses GPU surfaces), compared at the same window pixel
  size. The two window drawables are unavoidable (~6.6 MB each at
  1624×1018 px) and scale with window size; the target is for everything else:
  < 25 MB for one window with 10k lines of scrollback.
- **Fast output.** `cat` of a large file must never block typing. A macOS PTY
  delivers at most ~240 MB/s (1 KB per read), so parsing must stay well above
  that: target ≥ 2× the PTY limit.
- **Zero UI.** The window is the terminal surface only: transparent, full-size
  content title bar with no title text, no tab bar of our own (tabs are
  macOS's native window tabs), no settings window, no status bar. Config is
  a file; Settings… (Cmd+,) is a program the app's own binary runs in a
  terminal tab, editing that file (`docs/design/preferences.md`). Menus
  exist only because macOS requires a menu bar (Quit, Copy, Paste, New
  Window, New Tab); they are standard AppKit menus.
- **Dock pins.** `terminal-decline pin` turns any command into its own macOS app.

## Modules (Cargo workspace)

| crate         | does                                                                  |
|---------------|-----------------------------------------------------------------------|
| `term-core`   | escape-code handling (via vendored `vte`), screen grid, scrollback, selection |
| `term-pty`    | spawns the shell on a pseudo-terminal, reader thread, resize          |
| `term-font`   | Core Text: font loading, cell metrics, glyph rasterising              |
| `term-render` | Metal: glyph textures shared by every window, panes drawn into one layer |
| `term-app`    | AppKit window/view, keyboard + IME, mouse, clipboard, config, pins    |
| `term-prefs`  | the preferences: settings schema, safe `config.toml` edits, the TUI (no platform code) |
| `term-icon`   | generated pin icons by the Slope rule (`slope-pins-2`): Core Text outline of the initial, CoreGraphics drawing after `pin-template.svg`, ImageIO `.icns` |
| `term-image`  | image parsers and decoders with no terminal state: kitty keys, base64, zlib, iTerm2 arguments, Sixel, header sizes, pure-Rust PNG/JPEG/GIF/WebP, conversion to Display P3 (no platform code; docs/design/images.md) |

`term-core` has no platform code and is tested and benchmarked on its own.

`term-app` (the `terminal-decline` package) is a library plus a thin `main.rs`. The
AppKit code (`app`, `view`) only turns events into the inputs of plain
modules, each tested on its own: `session` (the program, its PTY, the
reader and writer threads, the shared terminal and the dirty flag),
`keys::translate` (a key press → bytes, input method or menu), `mouse`
(reporting encodings), `layout` (padding, grid size, cell
under a point), `frame_policy::decide` (draw, skip, pause, hold or hide on
each display tick), `config` and `pin`. Paths (config dir, apps dir, the
executable) are passed in from `main`, and pins run other programs through
an injected runner, so tests use temp dirs. `tests/pipeline.rs` runs real
programs through `session` with no window; `tests/smoke.rs` drives the real
app through a test-only control sequence (`ESC ] 7770 ; cmd BEL`, honoured
only with `DECLINE_TEST_DUMP=<file>` set: `dump`, `resize`, `font`, `tab`,
`quit`) that writes JSON dumps.

## Screen storage

- A cell is 8 bytes (`u64`): a 21-bit Unicode scalar, its width (narrow,
  wide, spacer), a "shows when empty" bit, and a 32-bit id into its screen's
  style table. Each screen (primary, alternate) has its own table of
  distinct `Style`s (colours, attributes, underline style and colour,
  overline, link), each stored once. Ids are freed by a sweep over the
  screen when the table reaches 1.5 × the screen's cells, with no reference
  counts; clearing the whole screen renumbers them and gives the memory
  back. History stores styles by value. Details and measurements:
  `docs/design/cells.md` §4–§5.
- The visible screen is a ring of rows (`VecDeque<Row>`), so scrolling the
  whole screen moves the top row to the bottom without shifting the others;
  scroll regions rotate just their rows. A row is `Vec<Cell>` plus a
  soft-wrap flag, and is cleared with a memset when the fill is the default
  blank (all zero).
- Scrollback (`term-core/src/scrollback.rs`) stores each line as UTF-8 text
  plus style runs (only non-default colours/styles are written), packed into
  ~64 KB blocks that are freed whole from the oldest end. Lines are expanded
  back into cells only when shown or copied. Measured per line: plain 52 B,
  `ls` colours 93 B, compiler errors 81 B, full-width background 183 B, CJK
  90 B (was 518 / 674 / – / ~1500 / – with full cell rows): 10k lines take
  0.5–1.8 MB. Encoding finds a line's end by skipping all-zero cells 8 at a
  time, and lines of plain default-style ASCII are a byte copy.
- term-render and term-app read cells only through `RowRef` and `CellRef`
  (content: empty, a character, cluster text or an image placeholder; width;
  and a `&Style` with colours, attributes, underline style and link), and
  the terminal only through `Terminal`'s methods: its fields are private.
  How cells are stored is term-core's own business
  (`docs/design/cells.md`).
- Points in the buffer use an ever-increasing line number so a selection stays
  attached to its text while output scrolls. The app and the renderer do no
  line arithmetic of their own: term-core gives them `point_at` (view row
  and column to a Point), `cursor_in_view`, `view_top`/`set_view_top`, the
  selection gestures (`select_word`, `select_line`, `select_all`,
  `extend_selection`) and the word rule (`term_core::word`).
- Parsing: `vte` is vendored (`vendor/vte`, through `[patch.crates-io]`)
  with `Perform::print_ascii`, which hands over each run of printable ASCII
  whole; the end of a run is found 8 bytes at a time. `Terminal::write_ascii`
  writes a run one row segment at a time (pen read once, wrap handled only at
  segment ends). Segments over part of a wide character, insert mode, DEC line
  drawing and autowrap off go one character at a time. A property test checks
  the bulk path against one character at a time, fed one byte per call.
- Widths come from term-core's own Unicode table (`src/unicode`, generated
  from the Unicode 18.0.0 files; equal to `unicode-width` per scalar but
  for U+17D8). The terminal segments printed text into grapheme clusters
  (UAX #29) as it arrives, including after a bulk ASCII run.
- Known gap for the first milestone: combining marks / grapheme clusters are
  still dropped from the cells (needs a side table for multi-codepoint cells;
  cells design PR 6). Resize does not re-wrap lines yet.

## Threads

1. **Reader thread** (one per window): the PTY is non-blocking; the reader
   waits with `poll`, then drains everything available (up to 1 MB) before
   parsing, because a macOS PTY returns at most ~1 KB per `read()`. It parses
   in 64 KB slices, taking the lock once per slice, so drawing and typing
   never wait long. Replies to queries
   (cursor position, device attributes) are written back to the PTY from here.
   After parsing it wakes the main thread **only if** no redraw is already
   pending (an atomic flag), so a flood of output causes at most one redraw
   per display refresh.
3. **Main thread** (AppKit): input, and drawing. Drawing is driven by a
   `CADisplayLink` that is paused whenever nothing changed; it is resumed by
   the reader thread's wake-up or by input. Each frame it locks the terminal
   briefly, copies the visible cells into the GPU instance buffer, unlocks,
   then encodes the draw. While the program has synchronized-update mode
   (2026) on, drawing is held (150 ms timeout). Nothing is drawn while the
   window is hidden or covered. When the display link was paused, a wake-up
   draws immediately rather than waiting a refresh. Input doesn't draw by
   itself: the first output after it (the echo) is drawn as soon as it is
   parsed, and without one a tick draws after 8 ms (see "Input latency").

## Failures and crashes

An app started from the Dock or as a pin has no stderr, so failures are
written into the window as dim red text (and to stderr):

- Config errors appear at the top of each window, before the program's
  output: `terminal-decline: config: font_sise: unknown field …; ignored`.
- A program that can't be started leaves its error and "press any key to
  close". One that exits within 1 s with a non-zero status (a broken pin,
  `terminal-decline -e /nonexistent`) leaves `[process exited with status N]`. In
  both cases the window stays open until a key is pressed (Cmd shortcuts
  still work, so the message can be copied). A program that exits with
  status 0, or after the first second, closes its window as before. The
  exit status comes from term-pty's `Child`, which reaps and signals under
  one lock, so no signal can reach a process that reused the pid.

Release builds unwind (`panic = "unwind"`). A panic hook writes the
message, a backtrace and the version to
`~/Library/Logs/terminal-decline/crash-<time>.log` and logs a fault to os_log
(subsystem `red.forbes.terminal-decline`, a few lines of FFI to `_os_log_impl`). A
panic while a reader thread parses output is caught inside the terminal's
lock, so the lock is never poisoned: that session's terminal is replaced by
a fresh one saying "Terminal Decline hit a bug and stopped this session; details in
…", its program is killed, and other windows keep running. A panic on the
main thread (AppKit) aborts once it has been logged. Each release keeps
its `terminal-decline.dSYM` (`split-debuginfo = "packed"`) as a release download,
for symbolicating crash logs. Unwinding costs binary size (unwind tables
and landing pads: 157 KB when it was added, 1.34 → 1.50 MB; 261 KB on 29
Oct 2026, 1.90 → 2.16 MB) and no measurable parse speed.

## Rendering (Metal)

- One `CAMetalLayer`, `presentsWithTransaction = false`,
  `displaySyncEnabled = false` (frames go to the window server as soon as
  the GPU is done; see "Input latency"), two drawables, triple-buffered
  instance buffers in shared memory (Apple Silicon has unified memory, so no
  copies to the GPU).
- Per cell the GPU gets one instance: glyph rectangle in the texture (u16×4),
  glyph offset (i16×2), fg colour (rgba8), bg colour (rgba8), flags (u32). The
  vertex shader places a quad per cell from its index and the grid size, so no
  vertex buffer is needed. Two passes in one encoder: backgrounds, then glyphs
  (glyphs can overflow their cell, e.g. italics and wide icons).
- Glyph texture: an `R8` texture for text (coverage, blended in sRGB space
  like native Mac text) and a `BGRA8` texture for colour emoji, **one pair
  per process**, shared by every window and keyed by (fonts, glyph key)
  (splits design §3.3). Shelf-packed; only the texture that fills up grows
  (caps: 4096² and 2048², 16 MB each), and growing copies the old texture
  into the corner of the new one on the GPU, so glyphs already placed stay
  where they are and nothing is rebuilt. At the cap it is emptied and
  refilled with what's on screen, and every pane built before is built
  again. Glyphs are rasterised once by Core Text; ASCII is pre-rasterised
  for each new font setting. When fonts no window uses are dropped (a
  window closed, the font size changed), both textures are emptied back to
  their starting sizes, so they never stay bigger than the open windows
  need. The textures go purgeable only when every window is hidden.
- One process-wide Metal device, queue and pair of pipelines; one `Fonts`
  per font setting and scale, shared by the windows using it. Each window
  has its own instance buffers and waits only on its own frames; uploads
  into the shared textures wait on every window's frames.
- Panes: each terminal's instances (its `PaneRender`, with the row cache)
  are placed relative to the pane's corner. A frame copies every pane's
  instances back to back into the window's buffers and draws each pane
  with `baseInstance`, its origin in the uniforms and a scissor (clamped to
  the drawable) around its rectangle, then the dividers.
- Box drawing (U+2500–257F), block elements (U+2580–259F), braille
  (U+2800–28FF), Powerline (U+E0A0–E0A3, U+E0B0–E0BF) and the legacy
  computing sextants, wedges and eighth blocks (U+1FB00–1FB8B) are drawn
  in code (`term_font::sprite`), not taken from the font: bitmaps exactly
  the cell's size, straight lines in whole pixels centred in the cell (light
  = the underline thickness, heavy twice that, double = two light strokes a
  light thickness apart), so neighbours join with no seam at any size.
  Rounded corners, diagonals, Powerline triangles and wedges are
  anti-aliased; shades (░▒▓) are even 25/50/75 % coverage, since a pixel
  pattern can't repeat across odd-width cells. Each shape has a `SpriteId`
  for the glyph keys of the cell design (docs/design/cells.md §10.4). Uploading a
  new glyph waits for in-flight frames, since `replaceRegion` doesn't
  synchronise with the GPU.
- Shaders are compiled to a `.metallib` at build time (falls back to runtime
  compilation if the Metal toolchain is missing).
- Cursor, selection and underline/strikethrough are drawn in the shader from
  flags; no extra geometry.
- Background blur/transparency: `NSVisualEffectView` behind a non-opaque
  Metal layer, opt-in by config.
- sRGB colours (no Display P3 tagging, which would over-saturate app colours).
- Later: dirty-row tracking so only changed rows are re-copied (currently the
  whole visible grid is re-copied when anything changed — ~30k cells × 24 B =
  720 KB memcpy, which is cheap but not free).

## Input

- Text through `NSTextInputClient` (so IME, dead keys and emoji picker work).
  Marked (in-progress IME) text is drawn at the cursor.
- Keys with Ctrl / Option / arrows / function keys / Home / End / PgUp / PgDn
  mapped to xterm sequences; application cursor and keypad modes respected.
  While an input method is composing, every key goes to it first.
- Ctrl and Meta work on non-Latin layouts: when the current layout's
  character isn't ASCII, the key's character on the ASCII-capable layout
  (the Latin layout macOS keeps for Cmd shortcuts) is used, so Ctrl+С on
  a Russian layout sends ^C and Meta+С sends ESC c (`ascii_layout`:
  `TISCopyCurrentASCIICapableKeyboardLayoutInputSource` + `UCKeyTranslate`
  for every key, read once and again only after the selected input source
  changes, which a distributed notification announces).
- Option-as-Meta configurable, **off by default** (like Terminal.app and
  Ghostty) so Option still types `@ [ ] { } | \ ~` on European layouts.
- Kitty keyboard protocol, all five flags, following the spec's tables
  (`keys`; table tests from the spec's examples). Flag 1 (disambiguate):
  Shift+Enter, Ctrl+Enter, Ctrl+Tab, Esc vs Alt, keypad keys by their own
  codes (Claude Code, Neovim, Helix rely on these). Flag 2: repeats and
  releases, so the view translates `keyUp` too, and sends a release only
  for a key whose press went to the program. Flag 4: the shifted key and
  the base layout key (the ASCII-capable layout's, so Ctrl+С on a Russian
  layout carries `c`). Flag 8: every key as an escape code, modifier keys
  too (`flagsChanged`); text keys still go through the input method, and
  what it types is encoded as that key (`keys::committed_text`), so IMEs
  and dead keys keep working; composed text with no key of its own is a
  key-0 text event under flag 16 and plain UTF-8 without it. Flag 16: the
  text inside the escape code. Cmd stays with the menus and the Cmd
  line-editing keys whatever the flags. term-core keeps the flags stack
  (`kitty_keyboard_flags()`).
- Cmd+← / Cmd+→ / Cmd+⌫ send start-of-line / end-of-line / delete-line.
- Cmd shortcuts are handled by the app, never sent to the program:
  Cmd+C copy (selection), Cmd+V paste (bracketed paste when on), Cmd+N new
  window, Cmd+T new tab, Cmd+Shift+[ / ] previous / next tab, Cmd+1…8 that
  tab and Cmd+9 the last, Cmd+W close, Cmd+Q quit, Cmd+K clear scrollback,
  Cmd+= / Cmd+- font size, Ctrl+Cmd+F full screen. These are the
  defaults of the `[keybind]` config (below), which remaps or unbinds them
  and makes keys send text; pins may override it in their own config.
- Secure Keyboard Entry (Terminal Decline menu, with a checkmark; kept in user
  defaults as `SecureKeyboardEntry`): `EnableSecureEventInput` so no other
  process sees the keys typed, as in Terminal.app. macOS has one switch
  for the whole session, so Terminal Decline holds it only while it is the active
  app (other apps' hot keys and input tools keep working while it is in
  the background) and gives it back when it quits (`secure_input`). Pins
  have their own bundle id, so their own setting.
- Mouse: drag to select (word on double-click, line on triple-click; words
  and lines are logical, so a word split by a soft wrap is selected whole,
  and dragging after a double- or triple-click grows by words or lines), wheel
  scrolls scrollback, or when the program asks for mouse reporting (1000/1002/
  1003 + SGR 1006) events go to the program; Shift overrides to select.
  Which click does what is `select::gesture`: Shift-click extends the
  selection (by words or lines if it was made that way), Option-drag makes
  a rectangle. Dragging past the top or bottom edge scrolls, faster the
  further past (10 lines a second per cell height, 3 to 150), driven by a
  16 ms dispatch timer that runs only while the mouse is held past an edge.
  `copy_on_select = true` copies a selection when the button is let go
  (a plain click without a drag copies nothing). term-core's selection has
  no rectangular mode, so a rectangle is kept in the app: its text is read
  a line at a time, and while it is shown term-core holds, and the
  renderer draws, the run of text between its corners rather than the
  rectangle itself.
- Right-click, or Control-click, opens a menu (Copy, Paste, Select All,
  Clear Scrollback; Copy only with a selection, Paste only with text on
  the clipboard) while the program isn't taking the mouse
  (`mouse::opens_menu`); with mouse reporting on, the click goes to the
  program as before, and Shift still overrides.

## Windows and tabs

Tabs are macOS's own window tabs: each tab is an `NSWindow` with its own
view, session and renderer, all sharing one tabbing identifier. Shell >
New Tab (Cmd+T, and the tab bar's "+", through `newWindowForTab:`) adds a
tab after the current one, in the current tab's working directory, as
Cmd+N does for windows. The Window menu has AppKit's own tab actions (Show
Previous/Next Tab, Move Tab to New Window, Merge All Windows) and hidden
Cmd+1…9 items. A new window is created with tabbing disallowed and allowed
once it is on screen, so Cmd+N gives a window even when System Settings
prefers tabs, while Merge All Windows and dragging tabs together still work.

The content view runs under the title bar (full-size content), so the
view's size doesn't change when the tab bar appears or goes. Each view
observes its window's `contentLayoutRect` (KVO): when the top inset (window
height less that rect's height: the title bar, plus the tab bar while it
shows, 36 pt on macOS 27) changes, the top padding follows and the grid
shrinks or grows, and the program gets a resize (`layout::top_inset`).

A background tab is a hidden window: it stops drawing and gives back its
drawables like a covered window. macOS doesn't reliably send a background
tab the occlusion event, so a window that isn't its tab group's selected
window counts as hidden whatever its occlusion state, and a tab switch is
noticed through the key window changing: the tab brought to the front
becomes key and is shown (and drawn) at once, and on the next turn of the
main queue, once the group has selected it, every window hides or shows to
match (as it does after a window opens or is brought to the front, which
an inactive app gets no key notifications for). (Watching the group's `selectedWindow` by KVO would need
re-registering each time a window moves between groups, and AppKit doesn't
document that property as observable.) The key window counts as seen even
before its occlusion state catches up. Nothing polls. Since it is
never drawn, what a program asks for that must happen anyway (a new title,
shown in the tab bar; the bell) is taken out of the terminal by the reader
thread as it parses (`session::Events`) and handed to the main thread,
rather than when a frame is drawn.

A dogfood restart saves each window's directory and frame, plus, for tabs,
the index of their tab group and which tab was in front
(`reload::SavedWindow`); they reopen as tabs of one window, in order. Files
from builds before tabs restore as plain windows. The build keeps its
bundle path from startup: if the bundle is gone rather than replaced (the
build from before the rename to Terminal Decline, removed by the dogfood
script), it restarts into `Terminal Decline.app` in the same folder
(`reload::relaunch_target`).

A tab whose shell was running a program (`claude`, `nvim foo.rs`,
`ssh host`) also saves that program's command line
(`SavedWindow::command`, and per pane in `SavedPanes`; absent in older
files and for tabs at their prompt). After the restart it is typed at the
new shell's prompt but not run, so Enter starts it again. Only tabs
running the login shell save one: not a window started with `-e PROGRAM`,
a pin's or one with `command` in the config (they run their program again
anyway), nor the Settings tab.

- What is saved (`running::foreground_command`): the arguments of the
  terminal's foreground process group's programs, read with sysctl
  `KERN_PROCARGS2`. A pipeline is its programs that the shell started, in
  the order they started, joined with ` | `. A script run through its
  `#!` interpreter is saved as the script (`/opt/homebrew/bin/vite dev`,
  not `node /opt/homebrew/bin/vite dev`). Each argument is quoted for the
  tab's shell (`running::quote`): as it is when no shell gives it a
  meaning, else in single quotes, with control characters as escapes
  (`$'…'`, or for fish unquoted `\n`), so the line has none and can't
  run before Enter. Tests read the quoted line back through sh, bash 3.2,
  bash 5 and zsh. Nothing is saved when the arguments can't be read
  (another user's process, such as `sudo`'s), aren't UTF-8, or come to
  over 64 KB. macOS 27 leaves the environment out of `KERN_PROCARGS2`, so
  `FOO=1 cmd` comes back as `cmd`.
- When it is typed (`Session::type_at_prompt`): typed as soon as the shell
  starts, the terminal echoes it before the prompt and the line editor
  then shows it again (seen with zsh and both bashes). So a thread waits
  until the shell has written something and has the terminal reading keys
  unechoed (canonical mode and echo off, read from the PTY's termios:
  `term_pty::Modes`), with the output quiet for 50 ms, looking every
  50 ms for at most 30 s. zsh, bash and fish all set that mode at their
  prompt; OSC 133 isn't needed. It is sent as a paste when the shell has
  bracketed paste on (zsh, bash 5.1+, fish), so key bindings and fish
  abbreviations don't act on it; Enter runs a paste as it does typed text.
  Anything typed into the tab first cancels it. Tested on a PTY with zsh
  5.9, bash 3.2 and bash 5 (fish isn't installed here, so its test is
  skipped), with oh-my-zsh's `.zshrc` (shown after about 0.8 s), and in
  the app (`smoke_restore_types_the_command_at_the_prompt`).

Splits (docs/design/splits.md) are being built. In place so far, with no
AppKit yet: the pane tree (`panes`: layout in whole device pixels with a
1 pt divider and a minimum pane size, focus by direction and in order,
zoom, close, equalize, divider drags and keyboard resizes, all property
tested), the per-pane and per-window frame decisions
(`frame_policy::pane` / `combine`; `decide` is now the one-pane case), the
pane actions and their keys, and `reload::SavedPanes`, the tree with each
pane's directory, which `SavedWindow` carries (absent in older files: one
pane). `unfocused_pane_opacity` (default 1.0: not dimmed) is read but not
yet drawn.

## Files, folders and Services

What the app does with files and folders from the rest of macOS
(`desktop`, used by `view` and `app`):

- Files or folders dragged onto a window are typed into it as a paste
  (bracketed when the program asked for bracketed paste): each path quoted
  for the shell the way Terminal.app does it (`My\ File.txt`; a name with
  a control character, which a paste would drop, as `$'…'`), separated by
  spaces. Finder's file reference URLs are resolved to paths. A dragged
  URL or text is typed as it is.
- A folder dropped on the Dock icon, or `open -a "Terminal Decline" DIR`
  (`application:openURLs:`; the bundle declares `public.folder` in
  `CFBundleDocumentTypes`): a new window whose shell starts there; a file
  opens in its folder. It is a tab of the window in front when macOS's
  "Prefer tabs when opening documents" says so (Always, or In Full Screen
  while that window is full screen; `NSWindow.userTabbingPreference`). An
  app not yet running opens those windows instead of its first one.
- Services (`NSServices` in the Info.plist that `scripts/bundle.sh`
  writes): "New Terminal Decline Window at Folder" and "New Terminal
  Decline Tab at Folder", for folders in Finder (its right-click menu's
  Services, or Finder > Services). The app delegate is the services
  provider (`newWindowAtFolder:userData:error:`,
  `newTabAtFolder:userData:error:`); the tab one falls back to a window
  when none is open. The app menu has the standard Services submenu.
  `scripts/dogfood.sh` runs `pbs -update` so a new build's items show.
- `confirm_close = true` (off by default): closing a window or tab while a
  program other than the shell runs in its foreground asks first, in a
  sheet ("vim is still running in it"), and quitting asks once for all of
  them. "Running" is read from the shell's `proc_bsdinfo`: the terminal's
  foreground process group (`e_tpgid`) isn't the shell's own (`running`).
  A window started with `-e PROGRAM` counts only while that program runs
  another in the foreground. A dogfood restart and the test-only `quit`
  never ask.

The product's name in these strings is `PRODUCT_NAME` in the code and
`product_name` in `scripts/bundle.sh`.

## Notifications and progress

- `ESC ] 9 ; body BEL` (iTerm2's form; the title is the window's) and
  `ESC ] 777 ; notify ; title ; body BEL` (rxvt's) post a macOS
  notification through `UNUserNotificationCenter` when their window or tab
  isn't the one in front (app inactive, or another window or tab key). In
  front, nothing is shown. term-core keeps the latest one not yet taken, so
  several in one parse slice give one notification (a flood can't bury the
  screen in banners). Permission is asked once, on the first notification;
  notifications wait for the answer. A click brings that window or tab to
  the front (the request's identifier names the view).
- `UNUserNotificationCenter` throws outside an app bundle, so a bare
  `cargo install` binary checks for a bundle (`NSBundle.mainBundle` with
  an identifier, at a `.app` path) and, without one or without
  permission, does what the bell does: bounces the Dock icon while the
  app isn't active. The bell itself is unchanged.
- `ESC ] 9 ; 4 ; state ; percent BEL` (ConEmu's progress) shows on the
  Dock icon's badge: the lowest percentage of the windows reporting one,
  "!" while one reports an error, "…" while they are only busy; state 0
  or closing the window clears it. Pins have their own Dock icon, so their
  own badge.
- Like the title, these are taken by the reader thread as they are parsed,
  so a background tab (never drawn) still notifies.

What Claude Code sends (checked in its 2.1.306 binary): with
`preferredNotifChannel` at its default `auto`, it picks a channel only
for `TERM_PROGRAM` iTerm.app (OSC 9, `Claude Code: <message>`), kitty (OSC
99), ghostty (OSC 777 `notify;<title>;<message>`) and Apple_Terminal (a
bell, if the profile's bell is on); anything else, `terminal-decline` included,
gets nothing, not even a bell. So in Terminal Decline set it once:
`claude config set --global preferredNotifChannel iterm2` (or `ghostty`;
`iterm2_with_bell` adds a bell). Its OSC 9;4 progress
(`terminalProgressBarEnabled`) is sent only to ghostty ≥ 1.2.0, iTerm.app
≥ 3.6.6 and ConEmu, so Terminal Decline doesn't get it from Claude Code today. A
Notification or Stop hook can also return `terminalSequence` (OSC 9 / 777
/ 9;4) for Claude Code to print. OSC 99 (kitty) isn't handled.

## Dock pins ("make ./rush its own app")

`terminal-decline pin Rush [--icon rush.png] [--config rush.toml] [--dock] -- ./rush`
creates `~/Applications/Rush.app`:

```
Rush.app/Contents/
  Info.plist           CFBundleIdentifier = red.forbes.terminal-decline.pin.rush
  MacOS/rush           APFS clone of the Terminal Decline binary (no extra disk)
  Resources/icon.icns  generated from --icon (via `sips` + `iconutil`)
~/.config/terminal-decline/pins/rush.toml   command, cwd, env, any config setting
```

Each pin has its own bundle id, so macOS gives it its own Dock icon, Cmd+Tab
entry and window list. The binary knows it is a pin from its own file name.
Settings live outside the bundle so editing them doesn't break the signature.
The bundle is ad-hoc signed with its own identifier, the quarantine flag is
removed, and it is registered with LaunchServices. The program runs through a
login shell, so it gets the user's `PATH` even when started from the Dock.
A missing or nonexistent `cwd` starts in `HOME` (the app's own working
directory is `/` when started from the Dock).
`terminal-decline pin --update` re-clones the binary after upgrades, and
rewrites each pin's Info.plist (name from the bundle's file name, its icon
if it has one), so pins made before the rename (ids
`red.forbes.decline.pin.*`) get the new id prefix and their Secure Keyboard
Entry choice.
`terminal-decline pin --remove` deletes a bundle only if it is a `.app` inside the
apps dir containing `Contents/MacOS/<id>`, since the settings file naming it
can be edited. Pin names can't contain `/` or `..`, and can't take the app's
own names (`terminal-decline`, `decline`): the binary under those names is
never a pin, also when started through the cask's `decline` link.

Known limit: an ad-hoc signature is tied to the binary's hash, so after
`--update` macOS asks again for permissions the pin was granted (Documents,
Full Disk Access, …). The fix (next milestone) is Chrome's web-app approach:
a small launcher executable that never changes, loading Terminal Decline's code from
the installed `Terminal Decline.app`, found by its bundle id.

Generated icons (`term-icon`, docs/design/pins.md §8; not yet used by
`pin`): `term_icon::generate(name)` gives the kept record (canonical name,
FNV-1a hash, palette index and colours, initial, its outline as path data)
and `render_icns` draws it. Measured (release, M-series, macOS 27): ~65 ms
per pin for the outline and all ten `.icns` sizes, ~1.2 MB per `.icns`;
the outline of "R" equals the brand template's point for point (±0.01),
and 128/32/16 px renders differ from `docs/brand/pins.png` by 0.35–0.46
per channel on average. Samples: `docs/brand/samples-generated/`.

## The name

The app was called Decline until E.17; it is Terminal Decline now:

| | now | before |
|---|---|---|
| app | `Terminal Decline.app`, `red.forbes.terminal-decline` | `Decline.app`, `red.forbes.decline` |
| command | `terminal-decline` (the cask also links `decline`) | `decline` |
| pins | `red.forbes.terminal-decline.pin.<id>` | `red.forbes.decline.pin.<id>` |
| config | `~/.config/terminal-decline` | `~/.config/decline` |
| logs, caches | `~/Library/{Logs,Caches}/terminal-decline` | `…/decline` |
| `TERM_PROGRAM`, XTVERSION | `terminal-decline` | `decline` |

The defaults domain, os_log subsystem and tab group follow the bundle id.
The `DECLINE_*` variables for tests and measurements keep their names.

Moving over (`Paths::move_old_config`, before anything else runs, even
`pin`): if only `~/.config/decline` exists it is renamed to the new name
and the old name becomes a link to it, so pins made before the rename
still find their settings. If the old one is itself a link (dotfiles), the
new one links to the same place. If both exist the new one is used, nothing
is merged, and the window says so once. Another process doing the same at
once (the app and a pin starting together) is fine: the move is one
`rename`, and whoever loses looks again. The app copies its Secure Keyboard
Entry choice from the old defaults domain when the new one has none; old
crash logs stay where they are. macOS treats the renamed app and pins as new
apps, so permissions they were granted are asked for again.

## Config

`~/.config/terminal-decline/config.toml`: font family/size, colours (16 + fg/bg/cursor/
selection), padding, opacity/blur, option-as-meta, scrollback lines, shell.

Themes (`theme`): `theme = "name"` loads `~/.config/terminal-decline/themes/<name>.toml`,
else the built-in theme of that name; `theme = { light = "x", dark = "y" }`
picks by each window's macOS appearance and switches when it changes
(`viewDidChangeEffectiveAppearance`, a notification, no polling). A theme
file has the keys of `[colors]` (foreground, background, cursor, selection,
split_divider (unset: the background mixed 25% towards the foreground),
`palette` of up to 16; unset ones are the defaults'), and any `[colors]` in
the config override the theme's, one by one (`""` in `palette` keeps the
theme's entry). Colours must be `#rrggbb`; a bad one, a bad theme file or a
missing theme is a notice. The built-in set is embedded TOML
(`crates/term-app/themes/`, each file naming its source and licence):
slope-dark and slope-light (Terminal Decline's own, from the Slope brand palette),
solarized-dark/-light (MIT), gruvbox-dark (MIT), catppuccin-mocha/-latte
(MIT), tokyo-night (MIT; colours from tokyonight.nvim, Apache-2.0). With
no theme the colours are Terminal Decline's defaults, as before. Programs aren't
told about appearance changes yet: colour-scheme reports (mode 2031,
`CSI ? 997 ; 1|2 n`) need term-core support first.

Importers (`import`): `terminal-decline import ghostty [PATH]` reads Ghostty's config
(default: the first of `~/.config/ghostty/config.ghostty`, `…/config` and
the same in `~/Library/Application Support/com.mitchellh.ghostty`) and maps
theme (a built-in Terminal Decline theme by loose name, else the Ghostty theme file
becomes `themes/<name>.toml`; `light:x,dark:y` becomes a pair), palette,
foreground/background/cursor/selection colours, font-family (the first is
`font_family`, the rest `font_fallback`), the bold/italic faces,
font-size, adjust-cell-height/width percentages, background-opacity,
background-blur, macos-option-as-alt, copy-on-select, window-padding-x,
window-width/height, split-divider-color (`[colors] split_divider`),
unfocused-split-opacity (`unfocused_pane_opacity`) and keybinds (actions
with a Terminal Decline equivalent, new_split right/down, goto_split,
resize_split (its amount dropped: our keys move a divider one cell),
equalize_splits and toggle_split_zoom included; text:/csi:/esc:, unbind;
not sequences or global ones). `terminal-decline import
iterm FILE.itermcolors [--name NAME]` turns an iTerm2 preset (XML, or
binary through `plutil`) into `themes/<name>.toml`, converting calibrated
and P3 colours to sRGB with AppKit. Both print the TOML; `--write` writes
it and refuses to replace any existing file without `--force` (nothing is
written if one would be). Whatever doesn't map is listed as comments with
the reason.

Live reload (`watch`, `AppDelegate::reload_config`): the config files (the
user's, a pin's, `--config`, and the theme files the config names) are
watched with kqueue vnode watches on each file and its directory (editors save by renaming a new file over the old;
a missing file is watched through the nearest directory that exists, a
symlink at its target too). One thread blocks in `kevent`, so idle stays at
0 wakeups (`top -l 2 -s 5 -stats idlew` with the watcher on: 0). After an
event it waits 40 ms, arms the watches again and compares the files'
contents, so only a real change reloads. A reload reads every layer again
(`-e PROGRAM` still wins) and applies:

- to every window at once: `theme` and `[colors]`, the font settings
  (`font_family`, `font_size`, `font_fallback`, `font_bold`/`_italic`/`_bold_italic`,
  `line_height`, `cell_width`; Fonts rebuilt as Cmd+= does; a new
  `font_size` replaces a size set with Cmd+= / Cmd+-), `padding`,
  `opacity`, `option_as_meta`, `copy_on_select` and `[keybind]` (the menus
  are rebuilt);
- to new windows only: `command`, `cwd`, `env`, `columns`, `lines`,
  `blur` (a view set up with the window), `title`, and `scrollback`
  (term-core has no way to change a running terminal's limit; once it
  does, the new limit can apply from the next line). The list is the
  settings schema's (`term_prefs::schema::new_windows_keys`, read by
  `config::new_windows_only`); Settings marks those rows, and a reload
  says so on stderr only (`terminal-decline: config: columns changed: it
  applies to new windows`), never in a window.

`confirm_close` (see Files, folders and Services) applies at once.

Errors in the edited file are notices in every window, written after the
program's output, as at startup, but only those new since the last load,
and never into a terminal on the alternate screen (vim, Claude Code, the
preferences), where they would land under the program's cursor in its
colours. While the Settings tab is open no notices are written at all:
it shows the same messages, from the same loader. New windows show the
errors of the config in effect.

Settings… (Terminal Decline menu, Cmd+, — the `settings` action) is
`terminal-decline prefs` (crate `term-prefs`, design in
docs/design/preferences.md) in a tab of the front window; a second
Cmd+, brings the same tab back. The app runs its own binary (the main
app's, found by bundle id, when in a pin) directly, not through the
login shell (term-pty's `Command::direct`), with `--layer` for each
config file, `--edit` for the one edited (the user's, or a pin's own
file) and `--in-app`. The tab is titled "Settings", stays opaque, gets no
startup notices, and isn't a running program for `confirm_close`.
"Edit config.toml" beside it opens the file with `open -t`. The program
checks each edit with the app's loader through `prefs::AppHost`
(`term_prefs::Host`) before writing it, so the TUI's messages are the
app's own. To record a shortcut it sends `OSC 7771 ; record-chord`,
honoured only in that tab (a session hook set by the app, so no other
program can ask): a local key-down monitor takes the next key in that
window before the menus, the input method and `[keybind]`, and the app
replies through the PTY input with `OSC 7771 ; chord ; <chord>`
(`keybind::recorded_chord`, spelled as `Chord`'s `Display`) or
`OSC 7771 ; cancelled` (plain Esc, the window losing focus, a tab switch,
or 10 s).

Shortcuts (`[keybind]`, `keybind`): a chord (`"cmd+k"`, `"ctrl+shift+t"`,
`"cmd+enter"`; modifiers cmd, ctrl, alt, shift; keys by character or name)
set to a menu action (new_window, new_tab, close, copy, paste, select_all,
clear_scrollback, font_bigger, font_smaller, font_reset,
toggle_full_screen, next_tab, previous_tab, settings (Cmd+,), and find, a placeholder whose
Edit > Find… item stays disabled until the find bar exists; and the pane
actions of docs/design/splits.md §5, split_right/down, close_tab,
close_window, focus_pane_left/right/up/down/next/previous,
resize_pane_left/right/up/down, equalize_panes and toggle_pane_zoom, which
have their defaults (Cmd+D, Cmd+Opt+arrows, …) but no menu items or
handlers until panes are wired in, so their keys do what they did),
`"none"`, or
`{ text = "…" }`. The menus' own shortcuts are the defaults and each entry
replaces or removes one chord; the menus are built from the result, so
they show what works (an action's first chord shows, any others work
hidden, as Cmd+1…9 do), and the fixed items (Quit, Hide, Minimize,
Cmd+1…9) give up a chord the config names. Cmd chords bound to actions
are matched by the menus; the view performs those without Cmd and sends
text bindings before `keys::translate`. An unbound key, `"none"`
included, does what it does with no config: Cmd+← / → / ⌫ edit the line,
other Cmd keys do nothing, the rest go to the program. Characters are
matched on the ASCII-capable layout, so bindings work on non-Latin
layouts. A pin's `[keybind]` merges over the user's chord by chord (each
layer's chords are spelled one way before merging, so `"command+D"`
replaces `"cmd+d"`). A bad entry is a config notice naming it and is
ignored.

Fonts (`config::Config::font_config` → `term_font::Fonts::with_config`):
`font_family`, `font_size`, `font_fallback = [..]` (tried in order before
the system's fallback), `font_bold` / `font_italic` / `font_bold_italic`
(faces by name; unset, the family's own), `line_height = 1.0` (multiplies
the cell height; must be above 0) and `cell_width = "0"` (points, e.g.
`"1.5"` or `-1`, or a percentage of the advance, `"10%"`). A bad
`line_height` or `cell_width` is reported like any other bad setting. The
first window checks the named fonts are installed (`Fonts::missing`); any
that aren't become config notices in that window and later ones
(`terminal-decline: config: font_fallback: no font named "X" is installed;
ignored`). Cmd+= / Cmd+- change only the size; the other font settings
stay.

## Performance testing

- `term-core` benchmarks (criterion): plain ASCII, coloured output (SGR-heavy),
  cursor-movement-heavy (vim/htop style), `seq`-style short lines, Unicode/wide
  text, all with a full scrollback; each fed in 64 KB pieces (`parse`) and in
  1 KB pieces (`parse_1k`, what a macOS PTY returns per read).
- Render benchmark: frame build time for 300×100 grid (CPU side) and GPU time
  via Metal counters.
- End-to-end: wall time of `cat` 100 MB file; idle CPU over 60 s
  (`top -l`/`powermetrics`); resident memory with empty and full scrollback;
  input latency (keypress → on screen), see "Input latency" below.
- Profile with `samply` and Instruments (`xctrace` Time Profiler + Metal
  System Trace). Compare against Ghostty on the same machine.

## Conformance (vttest, esctest2)

`scripts/conformance/run.py` runs vttest's named menus and esctest2 against
term-core on a real PTY with no window (`crates/term-pty/examples/headless.rs`),
nightly in CI when main changed. Known failures are checked in
(`tests/conformance/accepted-failures.txt`, each `to-fix` or `not-planned`)
and explained in `docs/conformance-failures.md`; how to run and update it is
in `docs/conformance.md`. The harness answers DECRQCRA itself, from the
screen, because esctest reads the screen back through it.

## Measured (M-series Mac, macOS 27, release build)

| | Terminal Decline | Ghostty 1.x |
|---|---|---|
| Idle CPU | 0.0% | 0.9–2.7% |
| Idle wakeups/s | 0 | 1–2 |
| Footprint, idle window | 38 MB (13 MB is drawables) | 117 MB |
| `cat` 100 MB coloured text | 1.44–1.53 s | 1.90 s |
| CPU time during that `cat` | 1.6 s | 4.4 s |
| Footprint after it (10k lines scrollback) | 59 MB | 145–177 MB |
| Binary size | 2.78 MB with the preferences (2.17 MB before them; unwinding; 29 Oct 2026) | — |
| Settings tab (prefs process) | 2.8 MB footprint, 0 idle wakeups | — |
| Parse, plain text (1 KB pieces) | 423 MB/s | — |
| Parse, coloured / TUI / `seq` / Unicode | 172 / 289 / 110 / 131 MB/s | — |

`term-core` parse throughput (criterion, `cargo bench -p term-core --bench
parse`, M1 Max, MB/s; 64 KB pieces, then 1 KB pieces). Before the bulk ASCII
work, then after it:

| input | before | after |
|---|---|---|
| plain text | 113 / 110 | 431 / 423 (3.8×) |
| coloured (`ls`, compiler errors) | 128 / 125 | 172 / 172 (1.4×) |
| TUI redraws | 180 / 175 | 290 / 289 (1.6×) |
| `seq` | 39 / 39 | 111 / 110 (2.8×) |
| Unicode / wide | 121 / 124 | 132 / 131 |

By step (1 KB pieces, plain / coloured / TUI / `seq`): bulk ASCII runs
201 / 164 / 290 / 43; faster scrollback encoding 328 / 167 / 280 / 74; 8-byte
run scan 355 / 164 / 286 / 73; screen as a ring 419 / 171 / 285 / 102;
memset row reset 423 / 172 / 289 / 110. Plain text is now above the PTY's
~240 MB/s but short of the 2× goal.
Caveats: window sizes, fonts and scrollback limits were not matched exactly
yet; Terminal Decline does less work (no text shaping, no grapheme clusters, no
reflow). See the performance-testing section for the fair-comparison plan.

### Input latency (keypress → on screen)

`scripts/latency/run.py` (roadmap N.5). Two methods:

- **In decline.** With `DECLINE_LATENCY_LOG=<file>` (test and bench runs
  only; unset, it costs one check per key, parse and frame) Terminal Decline writes
  a JSON line per key: the event's timestamp (t0), `keyDown`, the bytes
  handed to the writer thread (t1), the next parse in that window, taken
  with the terminal locked (t2), the frame that shows it (start, commit,
  GPU done) and the drawable's `presentedTime` (t3), all on one clock
  (`CLOCK_UPTIME_RAW`, which `NSEvent`, `CACurrentMediaTime` and Metal use).
  The driver starts Terminal Decline as a throwaway app bundle through `open` (an
  app started from a script may not be given the focus), running
  `scripts/latency/echo.py` (raw mode, echoes each key at once) or
  `htop -d 1` (typing into its search bar). Keys: Terminal Decline posts key events
  to its own process with `CGEventPostToPid` when asked through
  `DECLINE_LATENCY_INPUT` (a FIFO). That needs no permission (posting to
  another process needs Accessibility) and still goes through the window
  server. 200 keys per run, 50–150 ms apart at random, after 6 warm-up keys.
  Scenarios: idle; flood (a second window of the same process running
  `yes`); htop in the same window; the display link fixed at 120 or 60 Hz
  (`DECLINE_LATENCY_HZ`). With `--type` the driver instead sends the FIFO
  command `type <hex>`, which hands the key's bytes to the first window as
  `keyDown` would, without the window server: it works without the focus
  (macOS refuses to activate an app while someone types in another), and
  t0 is then `keyDown`.
- **From outside, for Ghostty and Terminal Decline alike.** `scripts/latency/probe.swift`
  posts the key to the app with `CGEventPostToPid` and watches its window
  with ScreenCaptureKit at every refresh; the latency is the display time
  of the first frame whose pixels changed minus the time just before
  posting. Needs Accessibility and Screen Recording for the terminal
  running it. Error bars: one refresh (8.3 ms at 120 Hz) per sample. On the
  same Terminal Decline frames, ScreenCaptureKit's display time is 5–13 ms (median
  11 ms) earlier than Metal's `presentedTime`, so add about 11 ms for the
  time to the glass; the comparison between terminals is like for like.
  Ghostty is started with Terminal Decline's defaults (Menlo 13, 100×30, padding 6,
  no cursor blink, no config file).

M1 Max, built-in ProMotion display, macOS 27, release build; the Mac was
in use (other apps and agents running) during the runs. In Terminal Decline, ms,
p50 / p95 / p99, 200 keys per run:

| scenario | key event → on glass | event → bytes to writer | writer → echo parsed | parsed → frame starts | frame build + encode | commit → on glass |
|---|---|---|---|---|---|---|
| idle, 120 Hz | 36.5 / 44.6 / 49.8 | 7.1 / 13.1 / 15.8 | 0.3 / 3.1 / 7.5 | 4.4 / 7.1 / 8.3 | 1.5 / 3.3 / 9.6 | 23.1 / 23.3 / 23.3 |
| idle, 60 Hz | 35.8 / 45.1 / 49.6 | 5.3 / 9.9 / 13.1 | 0.2 / 1.4 / 3.2 | 4.7 / 13.3 / 16.0 | 0.9 / 4.9 / 6.2 | 23.8 / 24.5 / 24.6 |
| flood, 120 Hz | 37.7 / 45.9 / 46.3 | 8.3 / 9.4 / 12.5 | 0.1 / 0.5 / 1.2 | 0.2 / 5.0 / 5.3 | 5.2 / 8.2 / 8.4 | 23.9 / 24.0 / 24.0 |
| flood, 60 Hz | 34.3 / 41.6 / 43.3 | 2.0 / 9.9 / 12.8 | 0.1 / 0.2 / 0.8 | 4.3 / 8.0 / 16.0 | 0.7 / 8.7 / 9.6 | 24.0 / 31.3 / 31.5 |
| htop, 120 Hz | 37.2 / 48.7 / 61.6 | 3.7 / 9.3 / 12.4 | 0.9 / 3.9 / 6.0 | 5.4 / 8.9 / 11.6 | 0.7 / 8.8 / 17.1 | 23.8 / 32.1 / 32.2 |
| htop, 60 Hz | 36.4 / 48.9 / 51.7 | 3.2 / 9.7 / 11.7 | 0.9 / 3.9 / 6.9 | 5.6 / 14.9 / 16.7 | 0.5 / 5.2 / 9.8 | 23.9 / 32.1 / 32.3 |

The GPU itself takes 0.3–0.7 ms (p50). These runs were made before t1 was
logged ahead of the hand-off to the writer: a key whose echo was parsed
before it was logged went unrecorded (46 of 200 at idle 60 Hz, at most 3
elsewhere) rather than mistimed. In htop, t2 is the first output after the
key, which can be htop's own refresh rather than its answer.

From outside (key posted → pixels changed, ms; 29 and 20 keys, too few for
a p99):

| scenario | Terminal Decline p50 / p95 / max | Ghostty 1.4.0 p50 / p95 / max |
|---|---|---|
| idle | 29.7 / 43.1 / 45.4 | 23.4 / 37.0 / 40.5 |
| flood (second window running `yes`) | 19.5 / 28.5 / 28.5 | 9.8 / 48.3 / 48.3 |

Screen Recording and Accessibility were granted only briefly, so the full
outside comparison (200 keys, htop) is still to run:
`scripts/latency/run.py --rates "" --external decline,ghostty`.

What this showed: Terminal Decline was 6–10 ms slower than Ghostty at the median.
Where its time went:
- About 23 ms from commit to `presentedTime`, nearly constant at either
  display link rate: a frame reached the glass about three 120 Hz
  refreshes after it was sent to the GPU.
- The key's own redraw: typing marked the screen dirty and, with the
  display link paused, drew at once, before the echo had arrived; the echo
  then waited for the next tick (4–5 ms p50, up to a full tick at 60 Hz).
- 2–8 ms (p50) from the event's timestamp to the bytes reaching the writer:
  window-server delivery plus the text input system (`interpretKeyEvents`).

#### The fix (October 2026)

Cause: the layer had `displaySyncEnabled` on (the default), so each
drawable was held for the display's next refresh before the window server
even saw it, and the window server then composited it on the refresh after
that. With only two drawables, a second frame inside that window (the
key's own redraw, then its echo; or the flood window's frames)
also blocked in `nextDrawable`, on the main thread, until one came back:
that is the 5–10 ms p95 of "frame build + encode" above, and it held up
every other window's drawing too.

Two changes:
1. `displaySyncEnabled = false` (term-render `configure_layer`): the
   drawable goes to the window server as soon as the GPU is done. A window
   doesn't tear, since the window server composites at the refresh anyway.
   Commit → on glass drops from 23–31 ms to 4–9 ms at the median. Not
   checked: a full-screen window, where macOS may show an opaque layer
   directly without compositing it, and frames could then tear.
2. Input no longer draws at once (view `send_input`): it starts the display
   link and waits for the echo, and the first output after it is drawn as
   soon as the reader has parsed it (`output_arrived`) instead of on the
   next tick. With no output within `ECHO_WAIT` (8 ms) a tick draws the
   input's own changes (cursor, scroll to the bottom). Parsed → frame
   starts drops from 4–9 ms to 0.1 ms, and the wasted frame before the echo
   is gone, so the echo's frame never waits for a drawable.

Tried and dropped: three drawables (the harness hung after a few keys, and
a third drawable only queues more), `presentsWithTransaction` with
`waitUntilScheduled` and `present` (frames took 400 ms to build, as the
main thread waited on the transaction). `CAMetalDisplayLink` and
`presentDrawable:atTime:` were not needed once the two changes above
closed the gap.

Before and after, in Terminal Decline with `--type` (t0 = `keyDown`), ms, p50 / p95
/ p99, 200 keys per run, the same M1 Max with other agents' builds and
benchmarks running (load average 5–24). The window was never the key
window (Alex was typing elsewhere), which is why the old build's commit →
on glass reads 24–31 ms rather than the 23 ms above:

| scenario | before: keyDown → on glass | after | before: commit → on glass | after |
|---|---|---|---|---|
| idle, 120 Hz | 39.2 / 54.3 / 80.7 | 8.6 / 13.9 / 20.4 | 31.6 / 39.0 / 57.0 | 8.0 / 12.2 / 19.8 |
| idle, 60 Hz | 39.2 / 43.3 / 43.8 | 9.3 / 26.9 / 31.2 | 30.9 / 32.3 / 35.4 | 8.8 / 26.5 / 30.7 |
| flood, 120 Hz | 30.4 / 32.3 / 32.9 | 9.5 / 13.4 / 22.1 | 23.9 / 24.0 / 24.1 | 8.8 / 12.9 / 14.5 |
| flood, 60 Hz | 34.8 / 41.9 / 46.3 | 4.1 / 9.6 / 17.8 | 24.4 / 31.7 / 32.1 | 3.9 / 9.5 / 17.7 |
| htop, 120 Hz | 36.3 / 43.8 / 48.0 | 5.8 / 22.1 / 27.5 | 25.7 / 32.4 / 36.6 | 5.2 / 17.8 / 24.4 |
| htop, 60 Hz | 36.5 / 43.7 / 48.8 | 5.4 / 21.5 / 28.7 | 24.2 / 32.4 / 36.5 | 4.2 / 19.0 / 24.1 |

Every p50 and p99 is lower. The new tail is no longer steady: commit → on
glass is mostly 3–10 ms with some keys at 17–31 ms, in stretches of
seconds. One flood 120 Hz run of the new build had such a stretch through
a third of its keys (7.5 / 32.9 / 41.3 key → glass, the old build's p99
then being 32.9); the run in the table, minutes later, had none. Likely
causes, not yet confirmed: the window server busy with other apps' frames,
and the panel slowing its refresh while the screen is still (ProMotion),
so the first frame after a pause waits for a slow refresh. Before, every
frame waited a fixed two to three refreshes, which hid both. The idle
60 Hz run lost its window to other apps after 146 keys (the 54 left had no
frame, as for the old build in runs that were stopped).

Idle: 0.0% CPU and 0 idle wakeups (`top -stats cpu,idlew`), as before:
the display link still pauses after two ticks with nothing new, and an
input with no echo keeps it running for at most `ECHO_WAIT`.

The outside comparison with Ghostty could not be run again: Screen
Recording and Accessibility are off for the terminal running the harness
(`probe check`: `screen_recording=false post_events=false`). Against the
outside numbers above (Ghostty idle 23.4 ms p50, flood 9.8 ms), Terminal Decline's
in-app key → glass gained 25–30 ms at the median, so it should now be well
ahead; to confirm, with the permissions on:
`scripts/latency/run.py --rates "" --external decline,ghostty`.

CI: hosted macOS runners have no real display, so presented times mean
nothing there. CI can track the part that is Terminal Decline's own work, from the
key's bytes to the encoded frame (t1 → commit): a headless test sending a
key through `session` to `echo.py` and timing until the reader has parsed
the echo, then `Renderer::prepare` and `encode` into an offscreen texture
(as term-render's offscreen test does), reported like the other benches.
Not built yet.

## Reviews

Design reviewed by Opus (xhigh effort); code reviewed by Astra (OpenAI) and
tested by Kimi. Fixed from those reviews: PTY write deadlocks, 1 KB read
batching, resize in the alt screen losing shell lines, scrollback view
sliding when full, origin-mode cursor panic, wide characters split by
insert/delete, G1 charset, DECSTR as soft reset, RIS keeping counters,
pending-wrap after erase, query replies (DECRQM, OSC 10/11/12, kitty flags,
XTVERSION, window size), VS16 emoji width, OSC 52/7, kitty keyboard,
keypad mode, paste filtering, environment for bare program names, child
reaping, glyph upload synchronisation, glyph texture caps, 16-bit position
overflow, synchronized-update stall, drawing while hidden, IME text overlap,
pin identity collisions, pin signing and quarantine.

## Next milestone

1. ~~Scrollback as text + style runs~~ (done). Next: compress sealed blocks.
2. ~~8-byte cells with a style table per screen~~ (done, cells design
   PR 5). Next: grapheme clusters per row (PR 6), OSC 8 links (PR 9).
3. ~~Parse plain-text runs in bulk~~ (done). Next: a CSI/SGR fast path.
4. **Copy cells under the lock, build GPU instances outside it**, with
   per-row dirty flags so a spinner doesn't rebuild the whole screen.
5. ~~Box-drawing and Powerline glyphs drawn in code~~ (done).
6. **Pin launcher** (see Dock pins).
7. Reflow on resize; Ctrl keys on non-Latin layouts; OSC 8 links.

## Pressure tests

`scripts/pressure/run.py target/release/terminal-decline <workdir>` (inputs from
`scripts/pressure/gen.py`). All survived, release and debug (overflow-checked)
builds:

| scenario | time | CPU | peak | after |
|---|---|---|---|---|
| 1 GB coloured flood | 12.5 s | 13.0 s | 59 MB | 55 MB |
| 3M short lines (scrollback churn) | 6.5 s | 5.1 s | 25 MB | 24 MB |
| 64 MB escape-code fuzz | 1.5 s | 1.6 s | 65 MB | 72 MB |
| 30k unique glyphs (texture growth/eviction) | – | 0.2 s | – | 38 MB |
| one 50 MB line | 2.6 s | 0.8 s | 56 MB | 54 MB |
| synchronized-update flood, 15 s | 15 s | 1.4 s | 58 MB | 55 MB |
| htop, 20 s | 20 s | 0.17 s | 43 MB | 38 MB |
| blinking cursor idle, 20 s | 20 s | 0.17 s | 41 MB | 36 MB |
| 10 windows idle (separate processes) | – | 0% | – | 286 MB total |
| 56 resizes during 1 GB output; 60 in htop | – | – | – | survived |

### Frame building after the cell interface (F3.1)

`cargo bench -p term-render --bench frame`, best of 5 alternating runs under
load: 80×24 plain 8.31 → 6.55 µs, 300×100 plain 126.4 → 97.0 µs, 300×100
coloured 113.5 → 110.4 µs (before PR 1 → after F3.1). Rows are built from
their stored cells, with selection/cursor/IME checks only on rows that need
them, and runs of same-styled cells share one resolved style. Text whose
style changes every one or two cells is still 10–25% slower than before PR 1
(each change re-reads the style); PR 5's style ids make that a single
compare.

### 8-byte cells and style tables (cells design PR 5)

Interleaved A/B against `main`, best of 5, busy machine. Parse
(`cargo bench -p term-core --bench parse`, 64 KB / 1 KB pieces): plain
418 → 534 / 413 → 520 MiB/s, `seq` +29% / +28%, coloured +8.5% / +7.6%,
unicode +19% / +19%, `tui` 0.0% / −1.5% (noise). Frames
(`cargo bench -p term-render --bench frame`): 80×24 plain 6.61 → 6.33 µs,
300×100 plain 99.3 → 93.4 µs, coloured 112.6 → 106.8 µs, a different
style every cell 152.8 → 140.9 µs. Memory: 8 B a cell (was 12); style
tables 92 B for plain text, 14 KB for 256 colours, 117 KB / 1.83 MB at
80×24 / 300×100 for a truecolour gradient, back under 4 KB once the screen
is cleared.

### Frames from the snapshot (cells design F3.3)

Frames are built from term-core's snapshot with a row cache keyed by each
row's change counter; the cursor, selection and input-method text are
drawn over the cached rows (cells.md §9.3, "Landed (F3.3)"). Still under
the lock. `cargo bench -p term-render --bench frame`, main and this
interleaved, best of 3, load 5–13: when nothing changed 80×24 6.4 → 1.2 µs
and 300×100 coloured 108 → 26 µs; one row changed (a spinner) at 300×100
coloured 114 → 24 µs; scrolled a line 118 → 44 µs. When every row changed
it costs more, 80×24 6.4 → 8.3 µs and 300×100 plain 94.6 → 127.2 µs,
coloured 108.2 → 143.6 µs, a style per cell 142.1 → 179.3 µs: the snapshot
copy and copying the rows' instances into the frame, which R.2 removes.

### Panes in one layer, shared glyph textures (splits design B2.1)

Each window's frame draws any number of panes into its one layer, and
every window shares one device, one pair of glyph textures and one `Fonts`
per font setting (splits.md §3, "Landed (B2.1)"). The app still draws one
full-window pane. Interleaved with main, best of 3, load 5–8, Menlo 28 px
with real glyphs (`cargo bench -p term-render --bench renderer`):

| case | main | B2.1 |
|---|---|---|
| 1 pane 160×48, every row changed (build) | 55.8 µs | 55.9 µs |
| 1 pane 160×48, nothing changed (build) | 4.41 µs | 4.20 µs |
| 1 pane 160×48, encode | 15.0 µs | 14.7 µs |
| 4 panes 80×24, every row changed (build) | | 57.6 µs |
| 4 panes 80×24, nothing changed (build) | | 5.81 µs |
| 4 panes 80×24 and 2 dividers, encode | | 16.8 µs |

The same cells in four panes cost 3% more to build when every row
changed, 1.6 µs more when nothing did (four snapshots and row caches),
and 2 µs more to encode (eight draws and four scissors). With the bench
glyph source (`--bench frame`): 160×48 coloured 36.4 µs against 4 × 80×24
37.2 µs, and 4.8 against 6.3 µs unchanged. The `frame` cases of one pane
are unchanged (the code is the same).
