# Terminal Decline shell integration for bash (3.2, as macOS ships it,
# and later).
#
# Marks each prompt, command line, command output and exit status with
# OSC 133 (A, B, C, D;status), so Cmd+Up / Cmd+Down jump between prompts
# and Shift+Cmd+A selects a command's output. Other terminals that know
# OSC 133 use the same marks; those that don't ignore them.
#
# Source it at the end of ~/.bashrc (or ~/.bash_profile), after anything
# that sets PS1 or PROMPT_COMMAND:
#
#     source /path/to/decline.bash
#
# Bash 4.4 and later mark a command's start from PS0. Before 4.4 there is
# no PS0, so the DEBUG trap does it, replacing any DEBUG trap already
# set. See docs/shell-integration.md.

[[ $- == *i* ]] || return 0
[[ -n ${_decline_integration-} ]] && return 0
_decline_integration=1

# A at the start of the prompt and B at its end, inside \[ \] so bash
# doesn't count them in the prompt's width.
_decline_a='\[\e]133;A\a\]'
_decline_b='\[\e]133;B\a\]'
_decline_ran=
_decline_at_prompt=

# First in PROMPT_COMMAND, so $? is the command's: the end mark.
_decline_status() {
  local ret=$?
  _decline_at_prompt=
  if [[ -n $_decline_ran ]]; then
    printf '\e]133;D;%s\a' "$ret"
    _decline_ran=
  fi
  return $ret
}

# Last in PROMPT_COMMAND: the prompt marks, put back each time in case
# something rebuilt PS1.
_decline_prompt() {
  PS1=${PS1//"$_decline_a"/}
  PS1=${PS1//"$_decline_b"/}
  PS1=$_decline_a$PS1$_decline_b
  _decline_at_prompt=1
}

PROMPT_COMMAND="_decline_status${PROMPT_COMMAND:+; $PROMPT_COMMAND}; _decline_prompt"

if (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )); then
  # PS0 is expanded after a command line is read, before it runs, in the
  # shell itself: the arithmetic notes that a command ran and expands to
  # nothing.
  PS0+='\e]133;C\a${_decline_ran:0:$((_decline_ran=1, 0))}'
else
  # The first command after the prompt is drawn is the command line's,
  # unless the line was empty and it is PROMPT_COMMAND's again.
  _decline_preexec() {
    [[ -n $_decline_at_prompt ]] || return 0
    [[ $BASH_COMMAND == _decline_status ]] && return 0
    _decline_at_prompt=
    _decline_ran=1
    printf '\e]133;C\a'
  }
  trap '_decline_preexec' DEBUG
fi
