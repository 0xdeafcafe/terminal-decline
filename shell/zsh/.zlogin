# Terminal Decline's shell integration for zsh: the user's .zlogin, the
# last startup file, with their ZDOTDIR left in place (see .zshenv here).

if [[ -n ${_decline_user_zdotdir+x} ]]; then
  builtin export ZDOTDIR=$_decline_user_zdotdir
else
  builtin unset ZDOTDIR
fi
builtin unset _decline_zdotdir _decline_user_zdotdir

[[ ! -r ${ZDOTDIR-$HOME}/.zlogin ]] || builtin source -- "${ZDOTDIR-$HOME}/.zlogin"
