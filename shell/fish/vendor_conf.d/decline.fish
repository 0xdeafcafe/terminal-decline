# Terminal Decline's shell integration for fish, loaded by the app
# (shell_integration = true; see docs/shell-integration.md).
#
# The app puts the directory above fish/ first in XDG_DATA_DIRS, and its
# path in DECLINE_FISH_XDG_DIR, so fish reads this file with its other
# vendor configuration, before config.fish. It takes that directory out
# of XDG_DATA_DIRS again, so programs started from fish don't see it, and
# loads decline.fish, which wraps the prompt only when the first one is
# drawn: a prompt or theme set in config.fish is the one wrapped.

if set -q DECLINE_FISH_XDG_DIR
    # One colon-separated string: fish splits only *PATH variables.
    set -l dirs
    for dir in (string split : -- "$XDG_DATA_DIRS")
        test "$dir" = "$DECLINE_FISH_XDG_DIR"; or set -a dirs $dir
    end
    if test (count $dirs) -gt 0
        set -gx XDG_DATA_DIRS (string join : -- $dirs)
    else
        set -e XDG_DATA_DIRS
    end
    set -l snippet $DECLINE_FISH_XDG_DIR/decline.fish
    set -e DECLINE_FISH_XDG_DIR
    status is-interactive; and test -r $snippet; and source $snippet
end
