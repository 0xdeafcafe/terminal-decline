# Links and file paths

What a click on a URL or a file path in the terminal does, and how it is
found. Roadmap E.2. Landed 1 Nov 2026 (`ec0d4ca`, `ab2c6e1`), asked for
by Alex.

## 1. What you see

- **Hover.** Move the pointer over a link or a path: it is underlined (all
  of it, across rows if it wraps) and the pointer becomes a pointing hand.
  No key needs holding. When a program takes the mouse (vim, htop, less
  with mouse on), only with Cmd held, as before.
- **A plain click** (left button, no modifier, no drag, one click) on one:
  after the double-click time (`NSEvent.doubleClickInterval`, about half a
  second by default) the whole link or path is selected, as a normal
  selection, so Cmd-C copies it, and a menu opens at the click:
  - a link: **Open**, **Copy**;
  - a path: **Open**, **Reveal in Finder**, **Copy Path**.
- **Cmd-click** opens it straight away, links and now paths too.
- **Unchanged:** double-click selects a word, triple-click a line, a drag
  selects, Option-click on the command line moves the shell's cursor,
  and a program taking the mouse gets the clicks. There, Shift-click (which
  already keeps a click from the program) is what brings up the menu.

## 2. What counts

Looked for in this order at the cell under the pointer; the first found
wins.

1. **An OSC 8 hyperlink** a program wrote (`ls --hyperlink`, `gcc`,
   `delta`). Its cells are the run of neighbouring cells with the same
   link (`Terminal::link_span`).
2. **A plain URL** in the text: starting with `https://`, `http://`,
   `file://` or `mailto:`, not in the middle of a word, up to the next
   space or a character no URL holds. Trailing `.`, `,`, `:`, `;`, `!`,
   `?`, and a closing bracket with no opening one, are left out
   (`term_core::word::url_at`).
3. **A file path that exists** (`term_core::paths::path_at`):
   - absolute (`/etc/hosts`), `~` and `~/…`, or relative to the shell's
     current folder, which the shell reports with OSC 7 (`./a`, `src/a.rs`,
     `../b`). Without a reported folder, relative paths aren't found.
   - It must look like a path: hold a `/` or a `.`, or start with `~`. A
     bare word such as `src` doesn't count even if a folder of that name
     exists, else every word of prose naming a folder would underline. So
     `Cargo.toml` in `ls` output is clickable, `crates` isn't.
   - A `:line` or `:line:col` after it, as compilers, grep and test
     runners print (`src/main.rs:12:5`), is kept in what is selected and
     copied, but left out of the path checked and opened. Grep's
     `a.rs:12:fn main` gives `a.rs:12`.
   - Trailing punctuation is left out as for URLs; quotes, `<`, `>`, `|`,
     `;` and backticks end a path. A path may follow `=`, `(`, `[` or `{`:
     `--manifest-path=a/Cargo.toml`, `[notes](docs/a.md)`.
   - A path holding spaces is found only when written in quotes
     (`"/tmp/a b/c.txt"`).
   - Only while the shell is on this Mac: OSC 7's host is empty,
     `localhost` or this Mac's name. Over ssh the paths would name the
     other machine's files, which can't be checked or opened here.

**Wrapped lines.** A link or path that wraps onto the next row (a soft
wrap, not a newline the program printed) is found from either row: the
text is read as the whole line the rows make up
(`Terminal::logical_text`), and the found characters are mapped back to
their cells.

**Whether a path exists** is asked of the filesystem (`stat`) in the app;
term-core only takes a function that answers, so it stays free of the
filesystem and its tests need no real files. For the hover, answers are
kept for 3 s, at most 512 of them (`links::ExistsCache`), so moving the
mouse doesn't `stat` on every move. A click asks again, fresh.

## 3. How a click becomes a menu

```
mouse down (plain, single)  → find the link or path under it; remember it
                              and where the click was; start a selection
                              as for any click
mouse up, not dragged       → wait the double-click time (a run loop timer,
                              performSelector:afterDelay:)
  another mouse down, a key,
  the pane losing focus     → the wait is cancelled; a double-click goes
                              on to select a word as ever
timer fires                 → find what is there again; if it is no longer
                              the same text (the screen scrolled, the
                              program redrew), nothing happens. Else select
                              it and pop up the menu at the click
```

Why wait: the menu is modal. Opened on the first release, it would take
the second click of a double-click, so double-clicking a path could never
select a word. The selection is made when the menu opens, not on release,
so a double-click doesn't flash the whole URL first. Choosing from the
menu doesn't copy, even with `copy_on_select` on; Copy does.

## 4. What the menu does

| Item | For | Does |
|---|---|---|
| Open | link | opens it as Cmd-click does: only `http`, `https`, `mailto`, and `file` links naming this Mac; any other scheme is refused with a notice in the terminal, so text a program printed can't start arbitrary apps |
| Copy | link | the URL on the clipboard |
| Open | path | `NSWorkspace openURL` with a file URL: the file in the app macOS opens it with, a folder in Finder |
| Reveal in Finder | path | a Finder window with it selected (`activateFileViewerSelectingURLs`) |
| Copy Path | path | the path as written, with its `:line:col` |

## 5. Where it lives

- `crates/term-core/src/paths.rs`: finding a path in a line of text, the
  `:line:col`, trailing punctuation, quotes, and resolving `~` and
  relative paths. Pure; unit tests.
- `crates/term-core/src/word.rs`: plain URLs.
- `crates/term-core/src/term.rs`: `path_at`, `link_span` and `url_at` over
  the screen, following soft wraps; `set_hover` (a link's underline) and
  `set_hover_span` (a path's cells). The renderer underlines either.
- `crates/term-app/src/links.rs`: `target_at` (link, else URL, else path,
  in that order; paths only on this Mac), the exists cache, and which links
  may be opened.
- `crates/term-app/src/pane_view.rs`: the mouse handlers (`mouse_button_event`,
  `mouse_motion`), the wait before the menu (`schedule_click_menu`,
  `cancel_click_menu`, `show_click_menu`), the menu's actions, and the
  hover (`set_hover`).
- Tests: `crates/term-core/tests/links.rs` (wrapped paths, the hover
  underline). Under the test control the menu isn't shown but recorded in
  the dump's `menus`, and opening or revealing a path in `opened`.

## 6. Not done

- `..` isn't tidied in the path shown or copied; the filesystem still finds
  `../x`.
- Open doesn't go to the `:line` in an editor: it opens the file in its
  default app. An "Open in editor at line" item would need an editor
  setting.
- No paths over ssh, and none in a program taking the mouse without Shift.
