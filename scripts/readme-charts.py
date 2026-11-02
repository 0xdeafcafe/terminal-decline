#!/usr/bin/env python3
"""Draws the README's benchmark chart, light and dark, as plain SVG.

    python3 scripts/readme-charts.py   # writes docs/img/bench-{light,dark}.svg

The numbers are the README table's; change them here when it changes.
"""

from pathlib import Path

TERMINALS = ["Decline", "Ghostty", "kitty"]
PANELS = [
    ("cat 100 MB", "seconds", [0.58, 0.66, 1.05], "{:.2f} s"),
    ("CPU for that cat, with WindowServer", "seconds", [0.66, 2.04, 1.63], "{:.2f} s"),
    ("Memory with 4 tabs open", "MB", [47, 87, 104], "{:.0f} MB"),
]

THEMES = {
    "light": dict(text="#1f2328", muted="#59636e", us="#2a78d6", them="#c8ccd1", rule="#d1d9e0"),
    "dark": dict(text="#f0f6fc", muted="#9198a1", us="#3987e5", them="#3d444d", rule="#3d444d"),
}

W = 720
LABEL_W = 84  # terminal names
VALUE_W = 72  # room for the value after the longest bar
BAR_H = 16
BAR_GAP = 8
TITLE_H = 30
PANEL_GAP = 22
FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif"


def bar(x, y, w, h, r=4):
    """A bar from the baseline at x, its far end rounded."""
    r = min(r, w, h / 2)
    return (
        f"M{x},{y} H{x + w - r} A{r},{r} 0 0 1 {x + w},{y + r} "
        f"V{y + h - r} A{r},{r} 0 0 1 {x + w - r},{y + h} H{x} Z"
    )


def draw(t):
    out = []
    y = 0
    span = W - LABEL_W - VALUE_W
    for title, unit, values, fmt in PANELS:
        top = max(values)
        out.append(
            f'<text x="0" y="{y + 18}" fill="{t["text"]}" font-size="15" font-weight="600">{title}'
            f'<tspan fill="{t["muted"]}" font-weight="400">  ·  {unit}, lower is better</tspan></text>'
        )
        y += TITLE_H
        out.append(f'<line x1="{LABEL_W}" y1="{y - 4}" x2="{LABEL_W}" y2="{y + 3 * BAR_H + 2 * BAR_GAP + 4}" stroke="{t["rule"]}"/>')
        for i, (name, v) in enumerate(zip(TERMINALS, values)):
            us = i == 0
            w = max(2, span * v / top)
            weight = "600" if us else "400"
            out.append(
                f'<text x="{LABEL_W - 10}" y="{y + 12}" text-anchor="end" fill="{t["text"] if us else t["muted"]}" '
                f'font-size="13" font-weight="{weight}">{name}</text>'
            )
            out.append(f'<path d="{bar(LABEL_W + 1, y, w, BAR_H)}" fill="{t["us"] if us else t["them"]}"/>')
            out.append(
                f'<text x="{LABEL_W + w + 8}" y="{y + 12}" fill="{t["text"] if us else t["muted"]}" '
                f'font-size="13" font-weight="{weight}">{fmt.format(v)}</text>'
            )
            y += BAR_H + BAR_GAP
        y += PANEL_GAP - BAR_GAP
    h = y - PANEL_GAP + 4
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{h}" viewBox="0 0 {W} {h}" '
        f'font-family="{FONT}" role="img" aria-label="Benchmarks: Decline against Ghostty and kitty">\n  '
        + "\n  ".join(out)
        + "\n</svg>\n"
    )


def main():
    root = Path(__file__).resolve().parent.parent / "docs" / "img"
    root.mkdir(parents=True, exist_ok=True)
    for name, theme in THEMES.items():
        (root / f"bench-{name}.svg").write_text(draw(theme))


if __name__ == "__main__":
    main()
