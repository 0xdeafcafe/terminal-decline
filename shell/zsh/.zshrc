# Terminal Decline's shell integration for zsh: the user's .zshrc, with
# their ZDOTDIR while it runs, then decline.zsh (see .zshenv here).

if [[ -n ${_decline_user_zdotdir+x} ]]; then
  builtin export ZDOTDIR=$_decline_user_zdotdir
else
  builtin unset ZDOTDIR
fi

[[ ! -r ${ZDOTDIR-$HOME}/.zshrc ]] || builtin source -- "${ZDOTDIR-$HOME}/.zshrc"

# After the user's .zshrc, so its hooks come after a prompt theme's. It
# does nothing if the user's .zshrc sourced it already.
[[ ! -r ${_decline_zdotdir:h}/decline.zsh ]] || builtin source -- "${_decline_zdotdir:h}/decline.zsh"

if [[ -o login ]]; then
  # .zlogin comes next: ours, for the user's.
  if [[ -n ${ZDOTDIR+x} ]]; then
    builtin typeset -g _decline_user_zdotdir=$ZDOTDIR
  else
    builtin unset _decline_user_zdotdir
  fi
  builtin export ZDOTDIR=$_decline_zdotdir
else
  builtin unset _decline_zdotdir _decline_user_zdotdir
fi
