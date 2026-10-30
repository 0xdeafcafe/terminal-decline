# Terminal Decline shell integration for fish (3.1 and later).
#
# Marks each prompt, command line, command output and exit status with
# OSC 133 (A, B, C, D;status), so Cmd+Up / Cmd+Down jump between prompts
# and Shift+Cmd+A selects a command's output, and reports the folder
# with OSC 7. Other terminals that know OSC 133 use the same marks;
# those that don't ignore them.
#
# The app loads it by itself (shell_integration = true), before
# config.fish. Otherwise source it from ~/.config/fish/config.fish:
#
#     source /path/to/decline.fish
#
# Loading it twice does nothing the second time. See
# docs/shell-integration.md.

status is-interactive; or exit 0
set -q __decline_integration; and exit 0
set -g __decline_integration 1

function __decline_preexec --on-event fish_preexec
    printf '\e]133;C\a'
end

function __decline_postexec --on-event fish_postexec
    printf '\e]133;D;%s\a' $status
end

# The folder, before each prompt.
function __decline_cwd --on-event fish_prompt
    printf '\e]7;file://%s%s\a' (prompt_hostname) (string escape --style=url -- $PWD)
end

# A before the prompt and B after it. fish_prompt is wrapped when the
# first prompt is about to be drawn, so a prompt (or theme) set in
# config.fish after this file is the one wrapped.
function __decline_wrap_prompt --on-event fish_prompt
    functions -e __decline_wrap_prompt
    functions -q fish_prompt; or return
    functions -c fish_prompt __decline_original_prompt
    function fish_prompt
        printf '\e]133;A\a'
        __decline_original_prompt
        printf '\e]133;B\a'
    end
end
