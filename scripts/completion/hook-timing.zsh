# The zsh side of the command line reports (shell/decline.zsh,
# docs/design/completion.md §3.1, §11): how long the line hook takes per
# redraw, and the env report per prompt, in µs. Budget: 50 µs a key.
#
#     zsh -fic 'source scripts/completion/hook-timing.zsh [runs]'
#
# (interactive, as decline.zsh only loads in an interactive shell)
#
# Runs the hook outside zle, with BUFFER, LBUFFER, CONTEXT and KEYMAP set
# as zle would, writing its reports to /dev/null, for a 20 and a 200-byte
# line with nothing to encode, and a 200 and a 2,000-byte one with `;`,
# `%` and a tab.
emulate -L zsh
zmodload zsh/datetime
local runs=${1:-1000}
[[ -o interactive ]] || { print -u2 "run it with zsh -fic 'source …'"; return 1 }
typeset -g _decline_token=0123456789abcdef0123456789abcdef _decline_features=highlight
TTY=/dev/null
source ${0:A:h}/../../shell/decline.zsh 2>/dev/null
(( ${+functions[_decline_line]} )) || { print -u2 "the line hook didn't load"; return 1 }

local plain='git commit -m "fix the thing" && ls -la | grep x '
local unit='git commit -m "fix; 100% done"	&& ls -la | grep x '
local -a lines=("${plain[1,20]}" "${(l:200::x:)plain}" "${(l:200::x:)unit}" "${(l:2000::x:)unit}")
local line t0 total i
for line in $lines; do
  BUFFER=$line LBUFFER=${line[1,-3]} CONTEXT=start KEYMAP=main PREBUFFER=
  total=0
  for (( i = 0; i < runs; i++ )); do
    _decline_last_line=
    t0=$EPOCHREALTIME
    _decline_line
    (( total += EPOCHREALTIME - t0 ))
  done
  printf 'line hook, %4d-byte line: %6.1f µs\n' ${#line} $(( total / runs * 1e6 ))
done

total=0
for (( i = 0; i < 50; i++ )); do
  _decline_last_env=
  t0=$EPOCHREALTIME
  _decline_env
  (( total += EPOCHREALTIME - t0 ))
done
printf 'env report (%d functions, %d aliases): %6.1f µs, once per prompt when it changed\n' ${#functions} ${#aliases} $(( total / 50 * 1e6 ))
t0=$EPOCHREALTIME
for (( i = 0; i < 50; i++ )); do _decline_env; done
printf 'env report unchanged: %6.1f µs\n' $(( (EPOCHREALTIME - t0) / 50 * 1e6 ))
