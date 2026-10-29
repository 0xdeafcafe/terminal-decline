# Changelog

All notable changes to Terminal Decline, generated from the commit
messages by [git-cliff](https://git-cliff.org) (see `cliff.toml`).
Regenerate with `git cliff --output CHANGELOG.md`.

## Unreleased

### Build and packaging

- **ci:** fetch a pinned Sparkle with scripts/sparkle.sh
- **pin:** bundle the app as launcher, library and pin stub
- **pin:** run pin --repair after a dogfood install
- **render:** fail on CI when the shaders can't be precompiled
- integer build numbers and 0.0.x versions before the beta
- ship Apple Silicon only; cask declares auto_updates
- embed and sign Sparkle in bundle.sh
- ship Sparkle's licence and the cask's update notes

### Features

- **app:** add a test-only control OSC behind DECLINE_TEST_DUMP
- **app:** dogfood builds restart into a newer build when installed
- **app:** show failures in the window and contain crashes to one session
- **app:** log keypress-to-present timing with DECLINE_LATENCY_LOG
- **app:** add native window tabs
- **app:** add secure keyboard entry to the decline menu
- **app:** read font fallback, faces, line height and cell width
- **app:** auto-scroll, rectangles, Shift-click and copy on select
- **app:** use the ascii-capable layout for ctrl and meta
- **app:** support the kitty keyboard protocol's flags 2 to 16
- **app:** add a keybind config for the app's shortcuts
- **app:** reload the config when its files change
- **app:** add themes, built-in and from the themes folder
- **app:** import settings from ghostty and iterm2
- **app:** open files and folders from Finder, the Dock and Services
- **app:** restart into Terminal Decline.app once the old bundle is gone
- **app:** restart into Terminal Decline.app once the old bundle is gone
- **app:** add the pane tree and its layout
- **app:** decide frames per pane and per window
- **app:** add the pane actions, their keys and Ghostty's
- **app:** add the prefs subcommand and the app's side of the preferences
- **app:** open the preferences as a Settings tab and record chords for it
- **app:** read the command line of the job in the foreground
- **app:** type a restored tab's program at its shell's prompt
- **app:** add the font_features setting
- **app:** add the ambiguous_width setting
- **app:** split, close and focus panes
- **app:** open links with Cmd-click
- **app:** jump between prompts and select a command's output
- **app:** draw the colours programs set
- **app:** drag dividers, resize, equalize and zoom panes
- **app:** contain a crash while drawing to its pane
- **app:** shape and build panes with their terminals unlocked
- **app:** option-click moves the shell's cursor on the command line
- **app:** add the find bar with search behind a trait
- **app:** search with term-core's engine behind the find bar
- **app:** run the app from a library, in the app, pin or bare role
- **app:** add the update module, with Sparkle loaded at run time
- **app:** log update-check in update.log
- **app:** share the find options on the find pasteboard
- **core:** store scrollback as text plus style runs
- **core:** add invariant checker and screen dump for tests
- **core:** add the view API and logical-line selection gestures
- **core:** add the unicode table and incremental grapheme segmentation
- **core:** store cells in 8 bytes with a per-screen style table (breaking)
- **core:** add row change counters and the snapshot
- **core:** add the ambiguous-width setting
- **core:** store grapheme clusters per row and add mode 2027
- **core:** add OSC 8 hyperlinks and OSC 133 prompt marks on the screen
- **core:** answer feature queries and keep program-set colours (F.5)
- **core:** find plain URLs and highlight them on hover
- **core:** keep prompt marks of lines in history
- **core:** store history in scrollback v2 blocks
- **core:** change the scrollback limit on a running terminal
- **core:** keep OSC 8 links and OSC 133 marks in history
- **core:** search history and the screen for the find bar
- **core:** reflow the screen on resize
- **core:** show history at the current width after a width change
- **core:** add the image interfaces and the process-wide pool
- **core:** store kitty unicode placeholders as cells
- **deps:** add APC, put_slice and an OSC size policy to vendored vte
- **font:** report the loaded family, fallback fonts and advances
- **font:** draw box-drawing, block, braille and Powerline glyphs in code
- **font:** add the font registry and rasterise by glyph key
- **font:** draw curly, dotted and dashed underlines as sprites
- **font:** shape runs with CTLine and cache shaped clusters
- **font:** add FontConfig with fallback list, face overrides and icon fitting
- **font:** add the ligature api, switched off
- **font:** shape runs for faces with ligatures
- **font:** find the characters a face's ligatures can match
- **image:** add term-image, the platform-free image parsers
- **image:** decode png, jpeg and gif in pure rust
- **image:** convert images to display p3 when decoded
- **image:** decode webp with image-webp
- **pin:** generate pin icons by the slope-pins-2 rule
- **pin:** spike pins on real signing with a minimal stub
- **pin:** build the pin stub and the app's launcher in full
- **pin:** make pins from the app's stub
- **prefs:** add the settings schema and the safe config writer
- **prefs:** add the hand-rolled preferences TUI and term_prefs::run
- **prefs:** add the pane settings to the schema
- **pty:** report the program's exit status
- **pty:** add Command::direct to exec a program without the login shell
- **pty:** say whether the program reads keys one at a time
- **render:** report frames submitted and completed, and glyph texture sizes
- **render:** draw underlines, overline and strike from the cell's Style
- **render:** build frames from the snapshot with overlays
- **render:** draw panes into one layer and share glyph textures
- **render:** draw a cluster cell's first character
- **render:** underline the hovered link
- **render:** draw ligatures in pieces and cache shaping by row
- **render:** draw grapheme clusters whole and ambiguous characters at two cells
- **render:** draw search hits under the selection
- **vte:** report how a dcs string ended with dcs_end(complete)
- first version of decline, a GPU-accelerated macOS terminal
- post notifications for osc 9 and 777, and show osc 9;4 progress
- rename to Terminal Decline (breaking)

### Fixes

- **app:** keep valid config settings when some are wrong
- **app:** hide background tabs without waiting for occlusion
- **app:** end open sheets before quitting without asking
- **app:** send CR LF for Return in newline mode
- **app:** restart for a new build only when the executable changed
- **app:** stop a running test waiting for a name the job never has
- **app:** handle the test-only key before the next control runs
- **app:** find prompts and search hits by view row, not by line
- **app:** search again after an alternate-screen switch or a new terminal
- **app:** keep the current find hit out from under the find bar
- **app:** focus a pane when its find bar is clicked
- **core:** erase a wide char that VS16 widening would split
- **core:** keep selections on their text when lines are not stored
- **core:** don't lose text after a UTF-8 character split across reads
- **core:** match xterm for mode 1047 and full reset
- **core:** let OSC 52 clipboard copies grow to 4 MB
- **core:** send each screen's style table whole to a new renderer
- **core:** keep REP copies of a joining scalar apart
- **core:** conformance fixes from vttest and esctest (E.19)
- **core:** give back text room past 64 KB when a stored line fits
- **core:** keep search hits right as the screen and history change
- **core:** search lines a resize shows again, and test resizes
- **image:** count the scaler's rows against the transient cap
- **image:** sum the scaler's spans in 64 bits
- **image:** apply the Display P3 matrix in 64 bits
- **pin:** make pins and config hermetic, and harden --remove
- **prefs:** keep a file's line endings when undo puts back its only line
- **pty:** start in HOME when cwd is missing or doesn't exist
- **pty:** build on Linux
- **pty:** kill the shell's whole process group after the grace period
- **pty:** retry opening a PTY when it fails transiently
- **pty:** set direct in the headless example's command
- **pty:** time the hang-up grace period against a deadline
- **render:** grow the glyph texture for glyphs bigger than it
- **render:** measure input-method text with term-core's width rule
- **render:** keep glyphs when a managed glyph texture grows

### Internal changes

- **app:** split decline into a library with AppKit-free modules
- **app:** split the view into a window view and a pane view
- **app:** remove the fallback TextSearch and note E.7 as landed
- **core:** remove unused Row::compact
- **core:** read cells through RowRef/CellRef and a Style value (breaking)
- **image:** share the golden image check between test files
- **pty:** add Command.shell and pure env and argv builders
- **render:** split encoding out of present

### Performance

- **app:** free memory for idle and hidden windows, cap frame rates
- **app:** draw the echo of input as soon as it is parsed
- **core:** bench parsing in 1 KB pieces and seq output
- **core:** write printable ASCII runs in bulk
- **core:** encode scrollback lines faster
- **core:** find the end of ASCII runs 8 bytes at a time
- **core:** keep the screen as a ring of rows
- **core:** reset rows with the default blank as a memset
- **core:** read cell styles with one shift and reuse them across runs
- **core:** key history runs on the style id and write UTF-8 in place
- **core:** store a history cluster's width and look styles up faster
- **core:** store history lines with less work per row and cell
- **core:** read history lines with less work per line and cell
- **core:** cheaper slot cache resets and width checks in history
- **core:** skip history's width check while the terminal's widths are the table's
- **core:** count history rows per block and from text length
- **pty:** return after one read instead of draining until empty
- **render:** hand frames to the window server without waiting for vsync
- **render:** build rows from their stored cells with per-row checks hoisted
- **render:** keep a cached glyph's cell in its spare flag bits

