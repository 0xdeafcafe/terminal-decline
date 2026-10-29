# Terminal Decline shell integration for zsh.
#
# Marks each prompt, command line, command output and exit status with
# OSC 133 (A, B, C, D;status), so Cmd+Up / Cmd+Down jump between prompts
# and Shift+Cmd+A selects a command's output. Other terminals that know
# OSC 133 use the same marks; those that don't ignore them.
#
# Source it at the end of ~/.zshrc, after any prompt theme:
#
#     source /path/to/decline.zsh
#
# See docs/shell-integration.md.

[[ -o interactive ]] || return 0
(( ${+_decline_integration} )) && return 0
typeset -g _decline_integration=1

# The marks go in PS1, inside %{ %} so zsh doesn't count them in the
# prompt's width, rather than being printed by precmd: zsh clears the
# line after precmd (PROMPT_SP), which would take a mark there with it.
# D (the last command's end and status) and A start the prompt, B ends it.
typeset -g _decline_a=$'%{\e]133;A\a%}'
typeset -g _decline_b=$'%{\e]133;B\a%}'
typeset -g _decline_d=

_decline_precmd() {
  local ret=$?
  # Themes rebuild PS1 in their own precmd: take the old marks out and
  # put them back each time.
  [[ -n $_decline_d ]] && PS1=${PS1//$_decline_d/}
  PS1=${PS1//$_decline_a/}
  PS1=${PS1//$_decline_b/}
  _decline_d=
  if (( ${+_decline_ran} )); then
    _decline_d=$'%{\e]133;D;'$ret$'\a%}'
    unset _decline_ran
  fi
  PS1=$_decline_d$_decline_a$PS1$_decline_b
}

_decline_preexec() {
  printf '\e]133;C\a'
  typeset -g _decline_ran=1
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd _decline_precmd
add-zsh-hook preexec _decline_preexec
