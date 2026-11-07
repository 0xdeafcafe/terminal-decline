#!/usr/bin/env python3
"""Refresh the HTML preview's real cells using an installed Decline on macOS.

All temporary state and PNGs stay under TMPDIR. No windows or Dock changes.
This displays the existing benchmark data; it does not run new benchmarks.
"""
from pathlib import Path
from datetime import date
import hashlib
import json
import os
import subprocess
import sys
import tempfile

SITE = Path(__file__).resolve().parents[1]
BINARY = Path.home() / 'Applications/Terminal Decline.app/Contents/MacOS/terminal-decline'
if not BINARY.is_file():
    raise SystemExit('Install Terminal Decline in ~/Applications before capturing.')
# A short name matters: macOS Unix-domain socket paths have a small limit.
scratch = Path(tempfile.mkdtemp(prefix='dc-', dir=os.environ['TMPDIR']))
(scratch / 'home').mkdir()
env = {
    'HOME': str(scratch / 'home'), 'TMPDIR': str(scratch),
    'TERMINAL_DECLINE_CTL_DIR': str(scratch / 'ctl'),
    'PATH': '/usr/bin:/bin:/usr/sbin:/sbin', 'SHELL': '/bin/zsh',
    'TERM': 'xterm-256color', 'LANG': 'en_GB.UTF-8',
}

def ctl(*arguments):
    result = subprocess.run([str(BINARY), 'ctl', *arguments], env=env,
                            text=True, capture_output=True, timeout=20)
    with (scratch / 'transcript.txt').open('a') as transcript:
        transcript.write(' '.join(arguments) + '\n' + result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(result.stderr or result.stdout)
    return result.stdout

spawned = False
try:
    ctl('spawn', '--name', 'website-comparison', '--size', '38x16', '--',
        sys.executable, str(SITE / 'scripts/comparison-terminal.py'), '--hold')
    spawned = True
    ctl('wait-for', 'website-comparison', '--text', 'Lower is better.', '--timeout', '5000')
    raw = ctl('screen', 'website-comparison', '--json')
    (scratch / 'screen.json').write_text(raw)
    ctl('screenshot', 'website-comparison', str(scratch / 'screen.png'))
finally:
    if spawned:
        ctl('kill', 'website-comparison')

screen = json.loads(raw)
rows = []
for cells in screen['cells']:
    runs = []
    for cell in cells:
        style = {'color': cell['fg'], 'bold': cell.get('bold', False)}
        if runs and all(runs[-1][key] == value for key, value in style.items()):
            runs[-1]['text'] += cell['t']
        else:
            runs.append({**style, 'text': cell['t']})
    while runs and not runs[-1]['text'].strip():
        runs.pop()
    if runs:
        runs[-1]['text'] = runs[-1]['text'].rstrip()
    rows.append(runs)
while rows and not rows[-1]:
    rows.pop()
preview = {
    'captured': date.today().isoformat(), 'cols': screen['cols'],
    'background': screen['colors']['background'],
    'foreground': screen['colors']['foreground'], 'rows': rows,
    'text': '\n'.join(screen['lines']).rstrip(),
    'sourceSha256': hashlib.sha256(raw.encode()).hexdigest(),
}
(SITE / 'src/data/terminal-preview.json').write_text(
    json.dumps(preview, indent=2, ensure_ascii=False) + '\n')
print(f'Captured cells saved. PNG and transcript: {scratch}')
