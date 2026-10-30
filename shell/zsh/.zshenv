# Terminal Decline's shell integration for zsh, loaded by the app
# (shell_integration = true; see docs/shell-integration.md).
#
# The app starts the login shell with ZDOTDIR pointing here, and the
# user's own ZDOTDIR, if they had one, in DECLINE_ZSH_ZDOTDIR. Each of
# the startup files here puts the user's ZDOTDIR back, sources the
# user's own file of the same name as zsh would have, and then points
# ZDOTDIR here again for the next one. After the user's .zshrc comes
# decline.zsh, so its hooks go after those of any prompt theme. The last
# file (.zlogin for a login shell, else .zshrc) leaves the user's ZDOTDIR
# in place, so shells started from this one aren't touched.
#
# Everything runs at the top level, not in a function, so `typeset` and
# `local` in the user's files mean what they always do.

# This directory, and the user's ZDOTDIR (unset: $HOME).
builtin typeset -g _decline_zdotdir=$ZDOTDIR
if [[ -n ${DECLINE_ZSH_ZDOTDIR+x} ]]; then
  builtin export ZDOTDIR=$DECLINE_ZSH_ZDOTDIR
  builtin unset DECLINE_ZSH_ZDOTDIR
else
  builtin unset ZDOTDIR
fi

[[ ! -r ${ZDOTDIR-$HOME}/.zshenv ]] || builtin source -- "${ZDOTDIR-$HOME}/.zshenv"

if [[ -o interactive && -o rcs ]]; then
  # The user's .zshenv may have set ZDOTDIR: remember it as it is now.
  if [[ -n ${ZDOTDIR+x} ]]; then
    builtin typeset -g _decline_user_zdotdir=$ZDOTDIR
  else
    builtin unset _decline_user_zdotdir
  fi
  builtin export ZDOTDIR=$_decline_zdotdir
else
  builtin unset _decline_zdotdir _decline_user_zdotdir
fi
