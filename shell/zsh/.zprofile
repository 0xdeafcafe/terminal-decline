# Terminal Decline's shell integration for zsh: the user's .zprofile, with
# their ZDOTDIR while it runs (see .zshenv here).

if [[ -n ${_decline_user_zdotdir+x} ]]; then
  builtin export ZDOTDIR=$_decline_user_zdotdir
else
  builtin unset ZDOTDIR
fi

[[ ! -r ${ZDOTDIR-$HOME}/.zprofile ]] || builtin source -- "${ZDOTDIR-$HOME}/.zprofile"

if [[ -n ${ZDOTDIR+x} ]]; then
  builtin typeset -g _decline_user_zdotdir=$ZDOTDIR
else
  builtin unset _decline_user_zdotdir
fi
builtin export ZDOTDIR=$_decline_zdotdir
