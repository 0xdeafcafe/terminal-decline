# Shell integration

Terminal Decline can jump between your shell's prompts and select what a
command printed, if the shell marks its prompts and commands. The marks
are OSC 133 sequences, the ones FinalTerm introduced and iTerm2, kitty,
WezTerm and Ghostty also read:

| Mark | Sent | Means |
|---|---|---|
| `ESC ] 133 ; A BEL` | where the prompt starts | a prompt |
| `ESC ] 133 ; B BEL` | where the prompt ends | the command line starts |
| `ESC ] 133 ; C BEL` | when a command runs | its output starts |
| `ESC ] 133 ; D ; status BEL` | before the next prompt | the command ended, with its exit status |

The app loads the marks into the shell by itself, the way Ghostty and
kitty do: nothing to source. `shell/` in this repository has the snippet
for each of zsh, bash and fish that sends them (and reports the folder
with OSC 7); the app bundles it as
`Terminal Decline.app/Contents/Resources/shell-integration` (a build in
a checkout uses the checkout's `shell/`).

## How the app loads it

Only into the login shell it starts for a tab, window or pane, or a
command that is just a shell (`command = ["fish"]`, a pin running
`bash`). Other commands, Settings, and shells started from those shells
(`ssh`, `sudo -s`, `tmux`, a `zsh` typed at the prompt) are left alone.
TERM, TERM_PROGRAM and the rest are as without it. Your startup files
still run, all of them, in the order the shell reads them:

- **zsh**: `ZDOTDIR` points at `shell-integration/zsh`, whose `.zshenv`,
  `.zprofile`, `.zshrc` and `.zlogin` each source yours (from your own
  `ZDOTDIR`, or `$HOME`) with your `ZDOTDIR` in place while it runs; after
  your `.zshrc` comes `decline.zsh`, so its hooks follow those of any
  prompt theme (Powerlevel10k and its instant prompt, Oh My Zsh,
  Starship). Once startup is done `ZDOTDIR` is yours again (or unset).
- **bash**: started as `bash --rcfile shell-integration/bash/decline-inject.bash`,
  which reads what a login bash would (`/etc/profile`, then the first of
  `~/.bash_profile`, `~/.bash_login` and `~/.profile`), then
  `decline.bash`. A login bash ignores `--rcfile`, so the shell isn't
  flagged as a login shell (`shopt login_shell` is off, `logout` says to
  use `exit`), as with Ghostty. Works with macOS's bash 3.2, which
  doesn't read `ENV` in POSIX mode (Ghostty's other route); there the
  snippet's `DEBUG` trap replaces one your files set (bash-preexec's,
  say), so turn the setting off if you rely on one.
- **fish**: `shell-integration` goes first in `XDG_DATA_DIRS`, so fish
  reads `fish/vendor_conf.d/decline.fish` there before `config.fish`; it
  takes the directory out of `XDG_DATA_DIRS` again and loads
  `decline.fish`, which wraps the prompt when the first one is drawn.

`shell_integration = false` (Settings > Advanced > Shell integration)
starts the shell exactly as it is, for new tabs and windows.

## Sourcing it by hand

For a shell the app didn't start (over `ssh`, in `tmux`), or with
`shell_integration = false`, source the snippet for the shell. A dotfile
that already sources it is fine with the app loading it too: each
snippet does nothing the second time.

**zsh**: at the end of `~/.zshrc`, after any prompt theme (Powerlevel10k,
Starship, Oh My Zsh themes), so its marks go around the prompt they set:

```zsh
source /path/to/terminal-decline/shell/decline.zsh
```

**bash** (3.2, as macOS ships it, and later): at the end of `~/.bashrc`
or `~/.bash_profile`, after anything that sets `PS1` or `PROMPT_COMMAND`:

```bash
source /path/to/terminal-decline/shell/decline.bash
```

Bash 4.4 and later mark a command's start from `PS0`. Bash 3.2 has no
`PS0`, so the snippet sets a `DEBUG` trap, which replaces one you
already have.

**fish** (3.1 and later): in `~/.config/fish/config.fish`:

```fish
source /path/to/terminal-decline/shell/decline.fish
```

The snippets do nothing in a shell that isn't interactive, and nothing
the second time they are sourced. Other terminals ignore the marks or use
them the same way, so a dotfile shared between terminals can source the
snippet unconditionally.

## What it gives you

| Shortcut | Menu | Does |
|---|---|---|
| Cmd+Up | View > Jump to Previous Prompt | scrolls back so the prompt above the top of the view is at the top |
| Cmd+Down | View > Jump to Next Prompt | scrolls forward to the next prompt, or to the bottom |
| Shift+Cmd+A | Edit > Select Command Output | selects the output of the command at the top of the view (where a jump leaves it); at the bottom, the running command's or the last one's |
| Shift+Cmd+C | Edit > Copy Last Command | copies the last command line run (the running one, or the last finished), wherever the view is |
| Option+Shift+Cmd+C | Edit > Copy Last Command's Output | copies the running command's output, or the last one that printed something, without selecting it |
| Option+Shift+Cmd+Up | View > Jump to Previous Failed Command | scrolls back to the nearest prompt above whose command exited non-zero |
| Option+Shift+Cmd+Down | View > Jump to Next Failed Command | the same, forward |

A prompt whose command exited non-zero (its D mark's status) gets a thin
mark in the theme's red in the left padding beside it, on the screen and
in the scrollback.

Right-clicking a command's prompt, command line or output also offers
**Select Command Output** for that command. Cmd+C then copies it.

**Option-click** on the command line you are typing moves the shell's
cursor to the cell you click, as in iTerm2 and Ghostty. The app types the
Left or Right arrow keys, as many as there are characters between the
cursor and the click (a wide character or an emoji is one; in application
cursor mode it sends the keys that mode asks for), so the shell does the
move itself. It works only on the command line: after the prompt's B mark,
on the rows of the line being edited (a line that wraps counts as one), while
no command is running. A click past the end of the text moves to the end.
Anywhere else, with no marks, or on a command line of several lines, the
Option-click is the usual rectangular selection. Shift, Cmd or Ctrl held
with Option, or a program that has taken the mouse, leave it alone too.

With no prompt that way, or no output to select, the app beeps. The
shortcuts can be changed in `[keybind]` (`previous_prompt`,
`next_prompt`, `select_output`, `copy_command`, `copy_output`,
`previous_failed`, `next_failed`); the copies' chords are Warp's;
importing a Ghostty config maps
`jump_to_prompt`.

Marks are kept for lines in the scrollback as well as on the screen, and
go when their lines are dropped from it or it is cleared (Cmd+K). A
full-screen program (an editor, `less`) has no scrollback, so the
shortcuts beep there.

## Checking it works

```sh
printf '\e]133;A\a$ \e]133;B\aecho hi\n\e]133;C\ahi\n\e]133;D;0\a'
```

prints a marked prompt by hand; Cmd+Up should then scroll to it once
it is above the top of the view. With a snippet sourced, every prompt is
marked the same way.
