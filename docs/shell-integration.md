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

The app doesn't inject anything into your shell. Instead `shell/` in
this repository has a snippet for each of zsh, bash and fish that sends
the marks; source the one for your shell.

## Setting it up

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

Right-clicking a command's prompt, command line or output also offers
**Select Command Output** for that command. Cmd+C then copies it.

With no prompt that way, or no output to select, the app beeps. The
shortcuts can be changed in `[keybind]` (`previous_prompt`,
`next_prompt`, `select_output`); importing a Ghostty config maps
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
