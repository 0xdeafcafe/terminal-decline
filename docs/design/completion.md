# Completion and highlighting at the prompt

Status: **draft**, not reviewed. Alex's decisions of 31 Oct 2026 are in
§1; open questions in §12.

## Goal and done-when

While you type a command line at the shell's prompt, decline:

1. colours it: the command, its flags, strings, variables, pipes and
   redirects, a command or folder that doesn't exist;
2. opens a list under the cursor, after a short pause in typing, of what
   the word being typed could be: files and folders, commands (on PATH,
   aliases, functions, builtins), and a command's flags and subcommands
   with what each does, from its man page;
3. on Tab with nothing of its own to offer, asks zsh's own completion
   (git branches, ssh hosts, make targets…) and shows its answers in the
   same list.

Done when: all three work in zsh started by the app; each can be turned
off on its own in Settings (§10); every budget in §11 is met and
measured (nothing when off or unused, typing latency unchanged when on);
and the list and colours are tested as below (§12). bash and fish get 1 and 2 from the screen
(§3.3), without Tab-to-zsh.

## 1. Decisions (Alex, 31 Oct 2026)

| Question | Decision |
|---|---|
| Where suggestions come from | Both: decline itself for files, commands and flags as you type; Tab asks zsh for the rest |
| When the list opens | By itself, after a short pause in typing; Up/Down move in it while it's open, Esc closes it, Tab takes the choice, Enter still runs the line |
| What the list looks like | Drawn by the renderer in the terminal's font and theme colours (not AppKit) |
| `--help` | Man pages only; a menu item reads one program's `--help` when you ask |

The find bar stays the only AppKit UI of our own; the list is drawn like
the cursor and the selection, so it costs no view, layer or backing
store.

## 2. What it looks like

Colours (theme ANSI slots, so every theme works; §5):

```
~/src ❯ git commt -m "fix it" | grep -v $HOME > ./out.txt
        ─── ───── ── ──────── ─ ──── ── ───── ─ ─────────
        │   │     │  │        │ │    │  │     │ └ path (exists: plain; missing folder: red)
        │   │     │  │        │ │    │  │     └ redirect: magenta
        │   │     │  │        │ │    │  └ variable: cyan
        │   │     │  │        │ │    └ flag: blue
        │   │     │  │        │ └ command found: green
        │   │     │  │        └ pipe: magenta (next word is a command)
        │   │     │  └ string: yellow
        │   │     └ flag: blue
        │   └ subcommand git has no man page for: red
        └ command found: green (not found: red)
```

The list, below the cursor's row (above it when there aren't enough
rows below), over the cells, never resizing the program:

```
~/src ❯ ls -l▏
          ┌──────────────────────────────────────────────┐
          │▸-l   List in long format                     │
          │ -la  combined: -l -a                         │
          │ -lh  combined: -l -h                         │
          ├──────────────────────────────────────────────┤
          │ -l  List files in the long format, as        │
          │     described in The Long Format below.      │
          │                                 — ls(1)      │
          └──────────────────────────────────────────────┘
```

Commands, with the man page's one-line description (its NAME section,
as `whatis` shows it), and the top choice shown grey after the cursor:

```
~ ❯ carg▏o
      ┌──────────────────────────────────────────┐
      │▸cargo          Rust's package manager    │
      │ cargo-clippy   Checks a package for lint │
      │ cargo-fmt      Formats Rust code         │
      └──────────────────────────────────────────┘
```

Files and folders, folders first, with the size of a file:

```
~/src/terminal-decline ❯ cat crates/term-c▏
                                   ┌────────────────────────┐
                                   │▸term-core/      folder │
                                   │ term-cli/       folder │
                                   └────────────────────────┘
```

At most 8 rows of choices, scrolling; the detail pane shows only for
flags with more description than fits on their row.

## 3. Knowing the command line

### 3.1 zsh reports it

`shell/decline.zsh` adds a `zle-line-pre-redraw` hook (zsh 5.3+; chained
with any the user has, as `add-zle-hook-widget` does) that sends, on each
redraw of the line:

```
ESC ] 7701 ; line ; <token> ; <cursor> ; <base64 of $BUFFER> BEL
```

`<cursor>` is `$CURSOR` in characters. Once per prompt (precmd) it sends
what the shell can run, each list NUL-separated and base64'd, only when
it changed since the last prompt:

```
ESC ] 7701 ; env ; <token> ; path=<…> ; alias=<…> ; func=<…> ; builtin=<…> BEL
```

7701 is unused by any terminal we know of (OSC 133 is FinalTerm's and
taken; 633 is VS Code's and 1337 iTerm2's).

**The token**: the app puts a random 128-bit `DECLINE_TOKEN` in the
shell's environment when it loads the integration, and term-core drops
any 7701 sequence without it. Without it, `cat`ing a file could forge a
command line and steer what Tab types into the shell. The snippet
unsets the variable after reading it, so programs the shell starts don't
inherit it.

### 3.2 term-core

term-core parses 7701 (no platform code, so it stays there) and keeps
the latest report, taken by the reader thread like the commands
(`take_commands`):

```rust
pub struct LineReport { pub text: String, pub cursor: usize, pub generation: u64 }
pub fn take_line(&mut self) -> Option<LineReport>;
pub fn take_shell_env(&mut self) -> Option<ShellEnv>;
```

A report counts only between a B mark and the next C mark (at the
prompt): after C, a stale report is dropped, so nothing shows over a
running program.

### 3.3 Other shells: read the screen

bash and fish have no per-keystroke hook. For them (and for zsh started
by hand without the snippet), the line is the text from the B mark to
the end of the cursor's line (with wrapped rows joined), and the cursor
is where the screen cursor is. This is what Option-click already relies
on. It is wrong when a right prompt or another tool's grey suggestion is
on the line, so it only counts while the cells after the cursor are
blank. Tab-to-zsh (§7) is zsh only.

## 4. Reading the line (term-app `complete::lex`, pure)

A small shell reader, not a parser: words, quotes (`'…'`, `"…"`, `$'…'`),
backslashes, `$VAR`/`${…}`/`$(…)` (kept as one word), operators
(`| || & && ; ;; > >> < 2> &>`), comments, and which words are in a
command position (first word, after an operator, after `sudo`, `env
X=y`, `time`, `nohup`, `exec`, `command`). Its output is spans with a
kind, plus what the cursor is on:

```rust
enum At { Command { prefix }, Subcommand { cmd, prefix }, Flag { cmd, sub, prefix },
          Path { prefix }, Variable { prefix }, Nothing }
```

Anything it doesn't understand (here-docs, `case`, unbalanced quotes
across lines) gives `Nothing` and plain colours, so it can't do worse
than today. Lines over 4 KB aren't read.

## 5. Colouring (term-render)

Colouring changes how cells are drawn, not the cells: the terminal's
grid, scrollback and copy are untouched, and the shell's redraws can't
fight it.

- `FrameInput` gets `line_colors: Option<&[(Point, Point, Color)]>` for
  the command line's spans, with `Color` an ANSI slot (green 2, red 1,
  yellow 3, blue 4, magenta 5, cyan 6) so themes and `keep_legible`
  apply.
- Only cells in the default foreground with no attributes are recoloured.
  A prompt theme, zsh-syntax-highlighting or zsh-autosuggestions' grey
  text keep their own colours, so nothing needs turning off for them.
- The rows it touches (usually one) skip the row cache for that frame;
  the cache key is unchanged, so no other row rebuilds.
- Paths and commands are checked off the main thread (§6); until a check
  comes back a word is plain, never red.

## 6. Where suggestions come from (term-app `complete::sources`)

All of it runs on one background thread per window, never on the main
thread or the PTY reader. Each source answers for a prefix and is
cached; the list shows whatever has answered when the pause ends and
fills in as the rest arrive.

- **Files and folders**: `read_dir` of the folder the word points into,
  relative to the shell's folder (OSC 7). Cached per folder until its
  mtime changes. Hidden files only for a prefix starting with `.`.
  Names are quoted for the shell when taken (`my file` → `my\ file`;
  control characters are never typed). Off when OSC 7's host isn't this
  Mac (ssh), since the folder is somewhere else.
- **Commands**: the shell's PATH, aliases, functions and builtins from
  the `env` report (§3.1); without one (bash, fish) the app's own PATH.
  Executables are listed once per PATH and refreshed when a PATH folder's
  mtime changes. Descriptions come from the man pages' NAME lines.
- **Flags and subcommands**: from the command's man page, found the way
  `man -w` does (MANPATH, else the folders next to the binary's
  `../share/man`, then `/usr/share/man`), gunzipped if needed, read
  directly. Two layouts cover what we have here:
  - mdoc (BSD, macOS's own): `.It Fl x Ar arg` items, description to the
    next `.It`;
  - man(7) (GNU, Homebrew, git's asciidoc): `.TP` or `.PP` followed by a
    line of bold flags (`\fB\-\-detach\fR`), description in the `.RS`
    block after it.
  Subcommands are the man pages named `<cmd>-<sub>` (202 `git-*`, the
  `cargo-*` ones) with their NAME lines. A parsed page is cached under
  `~/Library/Caches/<bundle id>/man/` keyed by path and mtime.
- **History order**: commands are ordered by how often they appear in
  the shell's history file (zsh's `HISTFILE`, read on the background
  thread at the first prompt and then from where it left off), then A–Z.
  Flags and files stay in their own order.
- **`--help`, when asked** (§8).

## 7. Tab asks zsh

With the list closed or empty, Tab goes to zsh as today. The snippet
binds Tab to a widget that runs zsh's completion with `compadd`
captured (the method fzf-tab uses): with one match zsh inserts it as
usual; with several it sends them, with their descriptions and groups,
as

```
ESC ] 7701 ; matches ; <token> ; <base64 of NUL-separated word/description pairs> BEL
```

and leaves the line alone, and decline shows them in the list. Taking
one types the rest of the word (§9). If the app isn't listening (no
token, or the setting is off) the widget falls back to zsh's own
`expand-or-complete`, so the snippet behaves the same in other
terminals.

## 8. Reading `--help` on request

Edit > Read Flags from --help (and the list's right-click) runs the
command under the cursor as `cmd --help`, once, in a temporary empty
folder, stdin `/dev/null`, killed after 2 s, output capped at 256 KB.
GNU-style lines (`-x, --long[=ARG]   description`) are read into the
same flag list and cached like a man page. Never automatic: a script
that doesn't know `--help` would just run.

## 9. Keys and taking a choice

- The list opens after `completion_delay` ms with no key (default 150)
  when the cursor is on a word with choices, or right after `-` on a
  flag. It closes on Esc, a key that isn't text or one of its own, the
  cursor leaving the word, a C mark, or the window losing focus.
- While it's open: Up/Down (and Ctrl+P/Ctrl+N) move, Tab takes the
  choice, Esc closes. Enter closes it and goes to the shell, so it runs
  what you typed. Typing goes to the shell as always and filters the
  list.
- Right arrow, at the end of the line, takes the grey text after the
  cursor.
- Taking a choice types into the PTY, as Option-click does: Backspaces
  for the part of the word that differs (usually none), then the rest.
  Bracketed paste isn't used, so the shell's own widgets see keys.

## 10. Settings

Each part can be turned off by itself, in `config.toml` and in Settings
(Cmd+,) in a new **Completion** section of term-prefs' schema, next to
Shell integration. All are live: a change applies to open tabs at the
next keystroke, without a restart.

| Key | Default | Range | Does |
|---|---|---|---|
| `highlight_command_line` | true | bool | colours the command line (§5) |
| `completion` | true | bool | the list (§9); off turns off everything below |
| `completion_files` | true | bool | files and folders in the list |
| `completion_commands` | true | bool | commands in the list |
| `completion_flags` | true | bool | flags and subcommands from man pages |
| `completion_tab_zsh` | true | bool | Tab asks zsh (§7); off leaves Tab to zsh as today |
| `completion_delay` | 150 | 0–2000 ms | pause before the list opens |
| `completion_rows` | 8 | 3–20 | choices shown before it scrolls |
| `completion_ghost_text` | true | bool | the top choice grey after the cursor |
| `completion_history_order` | true | bool | commands by how often you ran them |
| `[keybind] completion_next`, `completion_previous`, `completion_accept`, `completion_close` | Down, Up, Tab, Esc | chords | the list's keys |

With `highlight_command_line` and `completion` both off, the app gives
the shell no token (§3.1), so new shells don't install the line hook and
send nothing; shells already open keep sending until they exit, and the
app drops their reports after one match arm in the OSC parser (§11).
`shell_integration = false` turns all of it off, as it does the marks.

## 11. Cost and how it is measured

The rule: **off costs nothing, and on but unused costs nothing either**.
Only typing at a prompt pays, and it must not show in keypress-to-screen
latency.

What each state may cost:

| State | Allowed cost |
|---|---|
| Off (both settings) | none: no thread, no timer, no cache, no shell hook in new shells; binary size only |
| On, a program running (flood, `vim`, `cargo build`) | one branch per frame (`line_colors` is `None`) and an OSC arm never hit; parse and frame benchmarks unchanged within noise |
| On, idle at a prompt | 0 wakeups and 0.0% CPU: no polling and no timer armed; the pause timer is a one-shot armed by a keystroke |
| On, typing | per keystroke on the main thread: read the line and colour it ≤ 20 µs for a 200-character line; latency p50 and p99 unchanged within noise |
| List open | ≤ 50 µs added to a frame; sources answer off the main thread |

Threads and memory:
- The background thread is started on first use in a window, blocks on
  its channel while idle (no wakeups), and exits after 60 s unused, its
  caches with it.
- Caches are bounded: 64 folders, 32 parsed man pages and one PATH list
  per window in memory; the man cache on disk is capped at 20 MB, oldest
  first. History is read from its end, at most 10,000 lines.
- Budgets: footprint unchanged with both settings off; ≤ 2 MB per
  process after a session of use with every cache warm; release binary
  ≤ +200 KB.

New benchmarks (criterion, in the PRs that add the code):
- `term-core` `parse`: a typing session with 7701 reports between the
  keystrokes' echoes, against the same input without them.
- `term-app` (new `benches/complete.rs`): `lex` on lines of 20, 200 and
  4,000 characters; ranking 3,000 commands against a prefix; the man
  readers on the `ls`, `openrsync` and `git-checkout` fixtures.
- `term-render` `frame`: a prompt row with colours and with none; the
  list open (8 rows, detail pane) and closed.

Before and after, with `uptime` load noted (and a #perfwindow chirp):
- `cargo bench -p term-core`, `cargo bench -p term-render`, the new
  `term-app` bench;
- `scripts/latency/run.py` typing at a zsh prompt with everything on,
  and with everything off, against main;
- `scripts/pressure/run.py` with everything on: must match main, since
  nothing runs while a program floods;
- `/usr/bin/footprint -p <pid>` off, on and idle, on after use;
- `top -stats idlew` on an idle tab at a prompt: 0 wakeups;
- zsh's side: the hook timed over 1,000 redraws (`zmodload zsh/zprof`),
  ≤ 50 µs each, so the shell's own echo doesn't slow down.

A PR that misses a budget doesn't land until it meets it or Alex agrees
to the number. The results go in §15 as each PR lands, and in DESIGN.md
at the end of the phase.

## 12. Tests

- Unit tables: `lex` (quotes, operators, command positions, `sudo`,
  broken input → `Nothing`); quoting of taken names; the 7701 parser
  and the token check (term-core); man readers on small handwritten
  mdoc and man(7) fixtures shaped like `ls.1`, `openrsync.1` and
  `git-checkout.1`.
- Frame tests (term-render): recolouring only default cells; the list's
  instances placed below, above and clipped at the right edge.
- zsh snippet: a `zpty` test that the hook sends 7701 with the token and
  that Tab falls back without the app.
- One smoke test (CI): type `ls -` in zsh, wait, check the dump shows
  the list with `-l`, Tab, check the line reads `ls -l`.
- Settings: each key in §10 off gives no colours / no entries of its
  kind (dump checks); both off gives no token in a new shell's
  environment; the schema lists every key (term-prefs' schema test).
- Measurement: §11.

## 13. PRs

Each PR adds its settings to the schema and its benchmarks, and puts the
before-and-after numbers in its description.

1. core + shell: 7701 parsing, token, `take_line`/`take_shell_env`;
   zsh reports. Parse bench with reports; zsh hook timing.
2. app: `complete::lex` and `At`, with tables; `benches/complete.rs`.
3. render + app: colouring (§5), `highlight_command_line`. Frame bench,
   latency, pressure.
4. app + render: the list, keys, files and commands sources, the
   Completion section in Settings. Frame bench with the list, latency,
   idle wakeups, footprint.
5. app: man page readers, flags and subcommands, cache. Reader bench,
   footprint with the cache warm, disk cap.
6. shell + app: Tab asks zsh (§7).
7. app: Read Flags from --help (§8).
8. app: bash and fish from the screen (§3.3); docs/shell-integration.md.

## 14. Open questions for Alex

1. The grey text after the cursor for the top choice (fish-style): on,
   or only the list?
2. Commands by how often you ran them: fine to read the history file
   for that?
3. On by default for everyone, or off until 1.0?
4. Enter while the list is open: runs the line as typed (decided), or
   should it take the choice when you've moved in the list with Up/Down?

## 15. Measurements

None yet: each PR adds its numbers here as it lands (machine, `uptime`
load, before and after).
