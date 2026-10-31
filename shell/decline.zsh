# Terminal Decline shell integration for zsh.
#
# Marks each prompt, command line, command output and exit status with
# OSC 133 (A, B, C, D;status), so Cmd+Up / Cmd+Down jump between prompts
# and Shift+Cmd+A selects a command's output, and reports the folder
# with OSC 7. Other terminals that know OSC 133 use the same marks;
# those that don't ignore them.
#
# The app loads it by itself (shell_integration = true), after ~/.zshrc.
# Otherwise source it at the end of ~/.zshrc, after any prompt theme:
#
#     source /path/to/decline.zsh
#
# Loading it twice does nothing the second time. See
# docs/shell-integration.md.

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

# The folder, as a file:// URL with everything but letters, digits and
# /._~- percent-encoded byte by byte.
_decline_cwd() {
  emulate -L zsh
  local LC_ALL=C i ch url=
  for (( i = 1; i <= ${#PWD}; i++ )); do
    ch=${PWD[i]}
    if [[ $ch == [/._~A-Za-z0-9-] ]]; then
      url+=$ch
    else
      url+=%${(l:2::0:)$(( [##16] #ch ))}
    fi
  done
  printf '\e]7;file://%s%s\a' "$HOST" "$url"
}

_decline_precmd() {
  local ret=$?
  _decline_cwd
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
  (( ${+functions[_decline_env]} )) && _decline_env
  # A hook added after this one (a theme loaded later) would set PS1
  # after the marks went in: be the last again from the next prompt.
  if [[ ${precmd_functions[-1]} != _decline_precmd ]]; then
    precmd_functions=(${precmd_functions:#_decline_precmd} _decline_precmd)
  fi
}

_decline_preexec() {
  printf '\e]133;C\a'
  typeset -g _decline_ran=1
}

autoload -Uz add-zsh-hook
add-zsh-hook precmd _decline_precmd
add-zsh-hook preexec _decline_preexec

# The command line being edited, for the app to colour it and complete
# it (OSC 7701; docs/design/completion.md §3.1). Only with the token the
# app gave this shell, which .zshenv here moved out of the environment
# before the user's files ran; another terminal gives none, and then
# nothing below is set up.
if [[ -n ${_decline_token-} && -n ${_decline_features-} && -n $TTY ]] \
    && zmodload zsh/system 2>/dev/null \
    && builtin typeset -gH _decline_fd= \
    && sysopen -w -o cloexec -u _decline_fd -- $TTY 2>/dev/null; then
  typeset -gH _decline_last_line= _decline_last_env=

  # On each redraw of the line, before zle draws it:
  #   ESC ] 7701 ; line ; <token> ; <keymap> ; <cursor> ; <line> BEL
  # with %, ; and control bytes percent-encoded in one expansion, and the
  # cursor in bytes, both with multibyte off so the app counts the same
  # way (cheaper than a locale change on every key).
  # Nothing for a continuation line (PS2 has no prompt mark) or vared,
  # and nothing past 4 KB; nothing again until the report changes.
  _decline_line() {
    emulate -L zsh -o extendedglob +o multibyte
    local keymap=$KEYMAP cursor=0 text=
    if [[ $CONTEXT != start || -n $PREBUFFER ]]; then
      keymap=cont
    elif (( ${#BUFFER} > 4096 )); then
      keymap=long
    else
      text=$BUFFER
      # Most lines have nothing to encode: a match is cheaper than the
      # substitution's scan.
      if [[ $text == *[%\;[:cntrl:]]* ]]; then
        text=${text//(#m)[%;[:cntrl:]]/%${(l:2::0:)$(( [##16] #MATCH ))}}
      fi
      cursor=${#LBUFFER}
    fi
    local report="$keymap;$cursor;$text"
    [[ $report == $_decline_last_line ]] && return
    _decline_last_line=$report
    print -rnu $_decline_fd -- $'\e]7701;line;'"$_decline_token;$report"$'\a'
  }

  # Once per prompt, when it changed: what the shell can run and where
  # its history is.
  _decline_env() {
    # The user's option, before emulate sets zsh's defaults here.
    local autocd=0
    [[ -o autocd ]] && autocd=1
    emulate -L zsh -o extendedglob +o multibyte
    local -a f=(
      path "$PATH"
      alias "${(pj:\n:)${(@k)aliases}}"
      salias "${(pj:\n:)${(@k)saliases}}"
      galias "${(pj:\n:)${(@k)galiases}}"
      func "${(pj:\n:)${(@k)functions:#_*}}"
      builtin "${(pj:\n:)${(@k)builtins}}"
      resword "${(pj:\n:)${(@k)reswords}}"
      autocd $autocd
      histfile "${HISTFILE-}"
    )
    # A prompt starts afresh: its first line report always goes.
    _decline_last_line=
    local env=${(pj:\0:)f}
    [[ $env == $_decline_last_env ]] && return
    _decline_last_env=$env
    local k v out=
    for k v in "${f[@]}"; do
      out+=";$k=${v//(#m)[%;[:cntrl:]]/%${(l:2::0:)$(( [##16] #MATCH ))}}"
    done
    print -rnu $_decline_fd -- $'\e]7701;env;'"$_decline_token$out"$'\a'
  }

  zle -N _decline_line
  autoload -Uz add-zle-hook-widget
  # line-init too: a new line (the prompt's, or a continuation's) is
  # drawn before any redraw hook runs.
  add-zle-hook-widget line-init _decline_line
  add-zle-hook-widget line-pre-redraw _decline_line
fi

