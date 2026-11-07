#!/usr/bin/env python3
"""Display the README's dated measurements. This does not run a benchmark.

Used to capture a real Decline screen for the website's HTML terminal preview.
Run with --hold when capturing via ctl; Enter exits without a timer.
"""
import json
from pathlib import Path
import sys

DATA = json.loads((Path(__file__).resolve().parents[1] / 'src/data/comparison.json').read_text())
RESET = '\033[0m'
GREEN = '\033[32m'
BOLD = '\033[1m'

print('\033[2J\033[H\033[?25l', end='')
print(f"{BOLD}{DATA['date']}{RESET}\r")
print(f"M1 Max, medians of {DATA['runs']} runs\r\n\r")
for metric, title, suffix in [('idle', 'Idle memory', 'MB'), ('cat', 'cat 100 MB', 's')]:
    print(f'{BOLD}{title}{RESET}\r')
    maximum = max(t[metric] for t in DATA['terminals'])
    for terminal in DATA['terminals']:
        value = terminal[metric]
        amount = f'{value:.2f}' if metric == 'cat' else str(value)
        bar = '━' * round(value / maximum * 14)
        colour = GREEN if terminal['name'] == 'Decline' else ''
        print(f"{colour}{terminal['name']:<8} {bar:<14} {amount:>4} {suffix}{RESET}\r")
    print('\r')
print('Lower is better.\r')
sys.stdout.flush()
if '--hold' in sys.argv:
    sys.stdin.readline()
print('\033[?25h', end='')
