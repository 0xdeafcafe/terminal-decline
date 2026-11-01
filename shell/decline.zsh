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

# `decline` for the app this came with (decline pin, decline ctl, …),
# last in PATH so an installed one wins: bundled, this file is in
# Contents/Resources/shell-integration and the link in Contents/Resources/bin.
() {
  local bin=${${(%):-%x}:A:h:h}/bin
  [[ -x $bin/decline ]] && (( ! ${+commands[decline]} )) && path+=($bin)
}

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
  (( ${+functions[_decline_check_tab]} )) && _decline_check_tab
  # A hook added after this one (a theme loaded later) would set PS1
  # after the marks went in: be the last again from the next prompt.
  if [[ ${precmd_functions[-1]} != _decline_precmd ]]; then
    precmd_functions=(${precmd_functions:#_decline_precmd} _decline_precmd)
  fi
}

# C carries the command line as typed ($1), kitty's way (cmdline_url,
# percent-encoded): % ; and control characters escaped, so it can't end
# the sequence early. The terminal shows it in titles and types it again
# after a restart.
_decline_preexec() {
  emulate -L zsh
  local c=${1//\%/%25}
  c=${c//;/%3B}
  c=${c//$'\n'/%0A}
  c=${c//$'\t'/%09}
  c=${c//[[:cntrl:]]/}
  printf '\e]133;C;cmdline_url=%s\a' "$c"
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

  # A choice taken from the app's completion list: the app types
  #   ESC [ 7701 ~ <start>;<keep>;<name>;<suffix> BEL
  # and this puts <name>, quoted by zsh, in place of the word that starts
  # at byte <start>, keeping its first <keep> bytes as typed (`src/`,
  # `~/`), then <suffix> (a space after a file). The name is
  # percent-encoded like the reports. Bound only while the app gave a
  # token, so another terminal never sends it.
  # A payload `#<n>` is zsh's own match number <n> from the last Tab
  # (_decline_tab below), which zsh inserts itself.
  _decline_take() {
    local c payload=
    while read -rk 1 c && [[ $c != $'\a' ]]; do
      payload+=$c
      (( ${#payload} > 4096 )) && return 0
    done
    if [[ $payload == '#'<1-> ]]; then
      (( ${+functions[_decline_pick]} )) && _decline_pick ${payload#\#}
      return 0
    fi
    _decline_take_word $payload
  }

  _decline_take_word() {
    emulate -L zsh -o extendedglob +o multibyte
    local payload=$1
    local -a f=("${(@s:;:)payload}")
    (( ${#f} == 4 )) || return 0
    local start=$f[1] keep=$f[2] suffix=$f[4]
    [[ $start == <-> && $keep == <-> && ( -z $suffix || $suffix == ' ' ) ]] || return 0
    (( start + keep <= ${#LBUFFER} )) || return 0
    local name=${f[3]//(#b)%([[:xdigit:]](#c2))/${(#):-0x$match[1]}}
    local head=${LBUFFER[1,start+keep]}
    # Quoted as text, not bytes.
    setopt multibyte
    LBUFFER=$head${(q-)name}$suffix
  }

  # Tab asks zsh (docs/design/completion.md §7): the widget that was on
  # Tab runs as before, with compadd wrapped so each match is kept as
  # zsh would show it. With several, zsh inserts what they share and
  # lists nothing, and the app shows them in its list:
  #   ESC ] 7701 ; matches ; <token> ; <count> ; <shown, one per line> BEL
  # Taking one sends its number back (`#<n>` to _decline_take), and the
  # completion runs again adding only that match, so zsh inserts it
  # with its own quoting, prefixes and suffixes. Over 1,000 matches or
  # 32 KB, zsh lists them itself as before.
  if [[ ,$_decline_features, == *,tab,* ]]; then
    typeset -gHA _decline_tab_orig
    typeset -gHa _decline_hits
    typeset -gH _decline_mode= _decline_send=0 _decline_want=0 _decline_seen=0 _decline_picked=0

    _decline_compadd() {
      # Not ours, or another completer asking what would match: as is.
      if [[ $_decline_mode != (capture|pick) ]] || (( ${@[(I)-[OAD]*]} )); then
        builtin compadd "$@"
        return
      fi
      (( ${comppostfuncs[(I)_decline_post]} )) || comppostfuncs+=(_decline_post)
      # The display array (-d), if any, to show what zsh would.
      # -Q: the words come quoted already (files), shown unquoted.
      local a dname= fname= quoted=0 i
      for (( i = 1; i <= $#; i++ )); do
        a=${@[i]}
        case $a in
          (--) break ;;
          (-d) dname=${@[i+1]}; (( i++ )) ;;
          (-F) fname=${@[i+1]}; (( i++ )) ;;
          (-[PSpsiIWJVXxrRDOAEM]) (( i++ )) ;;
          (-d*) dname=${a#-d} ;;
          (-F*) fname=${a#-F} ;;
          (-[[:alnum:]]#Q[[:alnum:]]#) quoted=1 ;;
          (-*) ;;
          (*) break ;;
        esac
      done
      local -a hits shown
      [[ -n $dname ]] && shown=("${(@P)dname}")
      if [[ -n $dname ]]; then
        builtin compadd -O hits -D shown "$@"
      else
        builtin compadd -O hits "$@"
      fi
      if [[ $_decline_mode == capture ]]; then
        for (( i = 1; i <= $#hits; i++ )); do
          if [[ -n $shown[i] ]]; then
            a=$shown[i]
          elif (( quoted )); then
            a=${(Q)hits[i]}
          else
            a=$hits[i]
          fi
          _decline_hits+=("${a//[[:cntrl:]]/ }")
        done
        builtin compadd "$@"
        return
      fi
      # Picking: only the one wanted, the rest ignored (-F).
      local from=$_decline_seen
      (( _decline_seen += $#hits ))
      if (( _decline_picked || _decline_want <= from || _decline_want > _decline_seen )); then
        return 1
      fi
      _decline_picked=1
      local want=${hits[_decline_want-from]}
      local -a _decline_ignore old others
      others=("${(@)hits:#${~${(b)want}}}")
      _decline_ignore=("${(@b)others}")
      # Quoted words: ignored as written and unquoted.
      (( quoted )) && _decline_ignore+=("${(@b)${(@Q)others}}")
      if [[ -n $fname ]]; then
        old=("${(@P)fname}")
        set -A $fname "${old[@]}" "${_decline_ignore[@]}"
        builtin compadd "$@"
        set -A $fname "${old[@]}"
      else
        builtin compadd -F _decline_ignore "$@"
      fi
    }

    _decline_post() {
      case $_decline_mode in
        (capture)
          if (( compstate[nmatches] > 1 && $#_decline_hits > 1 && $#_decline_hits <= 1000 )) \
              && (( ${#${(pj:\n:)_decline_hits}} <= 32768 )); then
            compstate[list]=
            compstate[insert]=unambiguous
            _decline_send=1
          fi ;;
        (pick)
          compstate[list]= ;;
      esac
    }

    # The widget Tab ran before, run with compadd wrapped.
    _decline_complete() {
      local orig=${_decline_tab_orig[$KEYMAP]:-${_decline_tab_orig[main]:-expand-or-complete}}
      functions[compadd]='_decline_compadd "$@"'
      zle $orig
      local ret=$?
      unfunction compadd
      _decline_mode=
      return ret
    }

    _decline_tab() {
      # zsh beeps at an ambiguous completion; the list shows instead.
      setopt localoptions nolistbeep
      _decline_mode=capture _decline_send=0 _decline_hits=()
      _decline_complete
      local ret=$?
      (( _decline_send )) && _decline_send_matches
      _decline_hits=()
      _decline_line
      return ret
    }

    _decline_send_matches() {
      emulate -L zsh -o extendedglob +o multibyte
      local text=${(pj:\n:)_decline_hits}
      text=${text//(#m)[%;[:cntrl:]]/%${(l:2::0:)$(( [##16] #MATCH ))}}
      print -rnu $_decline_fd -- $'\e]7701;matches;'"$_decline_token;$#_decline_hits;$text"$'\a'
    }

    _decline_pick() {
      _decline_mode=pick _decline_want=$1 _decline_seen=0 _decline_picked=0
      _decline_complete
    }

    # Bind Tab where it leads to zsh's completion, remembering what it
    # ran; not over fzf-tab or zsh-autocomplete, which show their own
    # list. Again at each prompt: a plugin loaded later may rebind it.
    _decline_bind_tab() {
      local km w
      for km in main emacs viins; do
        w=${${(z)"$(bindkey -M $km '^I' 2>/dev/null)"}[2]}
        case $w in
          (_decline_tab|undefined-key|'') ;;
          (fzf-tab-complete|*autocomplete*) return ;;
          (*)
            _decline_tab_orig[$km]=$w
            bindkey -M $km '^I' _decline_tab ;;
        esac
      done
    }
    # At each prompt, one look at the main keymap's Tab.
    _decline_check_tab() {
      local w=${${(z)"$(bindkey -M main '^I' 2>/dev/null)"}[2]}
      [[ $w == (_decline_tab|fzf-tab-complete|*autocomplete*) ]] || _decline_bind_tab
    }
    # Not when something else already wraps compadd.
    if (( ! ${+functions[compadd]} )); then
      zle -N _decline_tab
      _decline_bind_tab
    else
      unfunction _decline_check_tab
    fi
  fi

  zle -N _decline_line
  if [[ ,$_decline_features, == *,list,* ]]; then
    zle -N _decline_take
    for _decline_km in main emacs viins; do
      bindkey -M $_decline_km $'\e[7701~' _decline_take
    done
    unset _decline_km
  fi
  autoload -Uz add-zle-hook-widget
  # line-init too: a new line (the prompt's, or a continuation's) is
  # drawn before any redraw hook runs.
  add-zle-hook-widget line-init _decline_line
  add-zle-hook-widget line-pre-redraw _decline_line
fi

