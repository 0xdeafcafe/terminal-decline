#!/usr/bin/env python3
"""History to scroll through, for scroll.py: asks for a COLSxLINES window
(the test control's `resize`), then prints LINES lines of mixed output, and
waits like a shell at its prompt.

    history.py [--cols 300] [--lines 100] [--history 10000] [--seed 1]

The newer half, above the last screens, has CJK picked at random, so
scrolling back into it finds glyphs not drawn yet; all of it is code- and log-like text in 16, 256 and
true colours, with CJK, emoji (ZWJ sequences and skin tones too), box
drawing, wide lines that wrap, and OSC 133 prompt marks.
"""

import argparse
import random
import sys
import time

WORDS = "the quick brown fox jumps over lazy dog cargo build test release frame glyph scroll history row cell".split()
CJK_COMMON = "日本語の文字列中文汉字한국어테스트終端機器表示"
EMOJI = ["😀", "🚀", "👍🏽", "👨‍👩‍👧‍👦", "🇬🇧", "✅", "❤️", "🔥"]
BOX = "─│┌┐└┘├┤┬┴┼═║"


def line(rng, i, rare):
    parts = []
    for _ in range(rng.randint(3, 14)):
        r = rng.random()
        if rare and r < 0.3:
            parts.append("".join(chr(rng.randint(0x4E00, 0x9FFF)) for _ in range(rng.randint(2, 6))))
        elif r < 0.45:
            parts.append(f"\x1b[3{rng.randint(1, 7)}m{rng.choice(WORDS)}\x1b[0m")
        elif r < 0.55:
            parts.append(f"\x1b[38;5;{rng.randint(16, 231)}m{rng.choice(WORDS)}\x1b[0m")
        elif r < 0.62:
            c = [rng.randint(0, 255) for _ in range(3)]
            parts.append(f"\x1b[48;2;{c[0]};{c[1]};{c[2]}m {rng.choice(WORDS)} \x1b[0m")
        elif r < 0.68:
            parts.append(CJK_COMMON[rng.randint(0, 10) :][: rng.randint(2, 6)])
        elif r < 0.72:
            parts.append(rng.choice(EMOJI))
        elif r < 0.75:
            parts.append(rng.choice(BOX) * rng.randint(4, 30))
        elif r < 0.78:
            parts.append("x" * rng.randint(200, 500))
        else:
            parts.append(rng.choice(WORDS))
    text = " ".join(parts)
    if i % 40 == 0:
        return f"\x1b]133;A\x07$ \x1b]133;B\x07make {i}\x1b]133;C\x07\r\n{text}"
    return text


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cols", type=int, default=300)
    ap.add_argument("--lines", type=int, default=100)
    ap.add_argument("--history", type=int, default=10000)
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()
    out = sys.stdout
    out.write(f"\x1b]7770;resize {args.cols} {args.lines}\x07")
    out.flush()
    time.sleep(1.0)
    rng = random.Random(args.seed)
    # Rare CJK in the newer half, but not on the last screen: scrolling
    # back soon reaches glyphs never drawn.
    rare = range(args.history // 2, args.history - 2 * args.lines)
    for i in range(args.history):
        out.write(line(rng, i, i in rare) + "\r\n")
    out.write("\x1b]133;A\x07$ \x1b]133;B\x07")
    out.flush()
    while True:
        time.sleep(3600)


if __name__ == "__main__":
    main()
