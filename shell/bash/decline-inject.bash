# Terminal Decline's shell integration for bash, loaded by the app
# (shell_integration = true; see docs/shell-integration.md).
#
# The app starts bash as an interactive shell that isn't a login shell,
# with --rcfile naming this file, so bash reads it in place of ~/.bashrc
# (a login shell would ignore --rcfile). This file reads the startup
# files bash would have read: for what was to be a login shell
# (DECLINE_BASH_LOGIN), /etc/profile and then the first of
# ~/.bash_profile, ~/.bash_login and ~/.profile; otherwise ~/.bashrc.
# Then decline.bash, so its hooks go after anything those files set.
# Everything runs at the top level, not in a function, so the user's
# files mean what they always do.

if [[ -n ${DECLINE_BASH_INJECT-} ]]; then
  _decline_dir=${BASH_SOURCE[0]%/*}
  _decline_login=${DECLINE_BASH_LOGIN-}
  builtin unset DECLINE_BASH_INJECT DECLINE_BASH_LOGIN

  if [[ -n $_decline_login ]]; then
    builtin unset _decline_login
    [[ ! -r /etc/profile ]] || builtin source /etc/profile
    if [[ -r ~/.bash_profile ]]; then
      builtin source ~/.bash_profile
    elif [[ -r ~/.bash_login ]]; then
      builtin source ~/.bash_login
    elif [[ -r ~/.profile ]]; then
      builtin source ~/.profile
    fi
  else
    builtin unset _decline_login
    [[ ! -r ~/.bashrc ]] || builtin source ~/.bashrc
  fi

  # It does nothing if the user's files sourced it already.
  [[ ! -r $_decline_dir/../decline.bash ]] || builtin source "$_decline_dir/../decline.bash"
  builtin unset _decline_dir
fi
