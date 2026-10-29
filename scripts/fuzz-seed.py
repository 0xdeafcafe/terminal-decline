#!/usr/bin/env python3
"""Writes the seed corpus for the `parse` fuzz target to fuzz/corpus/parse.

The files are committed; re-run this after changing it:

    python3 scripts/fuzz-seed.py            # writes fuzz/corpus/parse/seed-*
    python3 scripts/fuzz-seed.py --check    # fails if the files are stale

Each seed is terminal output in the style of a real program, with the
fuzz-only commands from fuzz/steps.rs mixed in (0xFF 'R' = resize, 'V' =
scroll the view, 'S' = select, 'C' = clear the selection, 'A' = set the
ambiguous width).
"""
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "fuzz", "corpus", "parse")

ESC = "\x1b"
CSI = ESC + "["
OSC = ESC + "]"
ST = ESC + "\\"


def resize(cols, lines):
    """Resize to exactly cols x lines (1..=200, 1..=60)."""
    return bytes([0xFF, ord("R"), cols - 1, lines - 1])


def scroll_view(delta):
    return bytes([0xFF, ord("V"), delta & 0xFF])


def select(al, ac, hl, hc):
    return bytes([0xFF, ord("S"), al, ac, hl, hc])


CLEAR = bytes([0xFF, ord("C")])


def ambiguous(width):
    """Set the ambiguous width to `width` (1 or 2) cells."""
    return bytes([0xFF, ord("A"), width - 1])


def t(s):
    return s.encode("utf-8")


def at(row, col):
    return f"{CSI}{row};{col}H"


def vim():
    """Alt screen, scroll region, status line, insert mode, :q."""
    out = [t(f"{CSI}?1049h{CSI}?1h{ESC}={CSI}?25l{CSI}H{CSI}2J")]
    for row in range(1, 23):
        out.append(t(f"{at(row, 1)}{CSI}38;5;12m{row:>3} {CSI}m" + ("fn main() {" if row == 1 else "~")))
    out.append(t(f"{at(24, 1)}{CSI}7m src/main.rs [+]{CSI}K{CSI}27m{at(24, 60)}1,1  All"))
    out.append(t(f"{CSI}1;23r{at(23, 1)}{ESC}D{ESC}D{CSI}2S{CSI}T{ESC}M{CSI}r"))
    out.append(t(f"{at(2, 5)}{CSI}4h    println!(\"hi\");{CSI}4l{CSI}?25h{CSI}6 q"))
    out.append(t(f"{at(10, 1)}{CSI}3L{CSI}2M{CSI}5@{CSI}3P{CSI}4X"))
    out.append(resize(100, 30))
    out.append(t(f"{CSI}?2004h{CSI}?1006h{CSI}?1002h{at(30, 1)}:q!{CSI}?1002l{CSI}?1006l{CSI}?2004l"))
    out.append(t(f"{CSI}?1049l{CSI}?1l{ESC}>"))
    return b"".join(out)


def less():
    """Alt screen pager: reverse-index scrolling, prompt, view scrolling."""
    out = [t("".join(f"line {i} of the file before less\r\n" for i in range(40)))]
    out.append(scroll_view(10))
    out.append(t(f"{CSI}?1049h{CSI}?1h{ESC}={CSI}H{CSI}2J"))
    out.append(t("".join(f"{i:>4}  text in the pager\r\n" for i in range(23))))
    out.append(t(f"{CSI}7m:{CSI}27m{CSI}K"))
    out.append(scroll_view(-5))
    for _ in range(5):
        out.append(t(f"{CSI}H{ESC}M{CSI}1mbold{CSI}22m earlier line{CSI}K{at(24, 1)}{CSI}K:"))
    out.append(t(f"{at(24, 1)}{CSI}7m(END){CSI}27m{CSI}K\r{CSI}K{CSI}?1049l{CSI}?1l{ESC}>"))
    out.append(scroll_view(127))
    out.append(select(0, 0, 5, 30))
    return b"".join(out)


def sgr():
    """Every SGR form: 16/256/truecolour, colon sub-parameters, styles."""
    parts = []
    for p in ["1", "2", "3", "4", "4:3", "5", "7", "8", "9", "21", "22", "23", "24", "27", "29", "53", "0"]:
        parts.append(f"{CSI}{p}mstyle {p}{CSI}0m ")
    for i in range(0, 256, 37):
        parts.append(f"{CSI}38;5;{i}m{CSI}48;5;{255 - i}m{i}")
    parts.append(f"{CSI}38;2;255;128;0mtrue{CSI}48:2::10:20:30mcolon{CSI}58;5;1mul{CSI}59m")
    parts.append(f"{CSI}38;2;300;-1mbad{CSI}38;5mshort{CSI}38;2;1;2mshort{CSI};;;m{CSI}90;107mbright{CSI}39;49m")
    parts.append(f"{CSI}" + ";".join(["1"] * 40) + "mmany")
    return t("".join(parts) + "\r\n")


def wide():
    """Wide and combining characters, wrapping at the right margin."""
    out = [resize(20, 6)]
    out.append(t("漢字かなカナ한글中文字符 at the edge 界\r\n"))
    out.append(t("🙂🎉 ❤️ ☺️ é ạ̈ 👩‍💻 🇬🇧\r\n"))
    out.append(t(f"{at(1, 19)}漢{at(1, 20)}字{at(2, 1)}{CSI}1@{CSI}1P"))
    out.append(t(f"{at(3, 2)}{CSI}2K界{at(3, 3)}x{at(3, 1)}{CSI}2X"))
    out.append(t("".join("字" * 15 + "\r\n" for _ in range(8))))
    out.append(select(0, 1, 2, 19))
    out.append(resize(19, 6))
    out.append(t("é" * 30 + "́" * 5))
    out.append(resize(1, 1))
    out.append(t("漢x漢"))
    out.append(resize(80, 24))
    return b"".join(out)


def clusters():
    """Grapheme clusters: widening and narrowing in place and at the right
    margin, mode 2027 on and off, REP of a cluster, insert/delete over
    clusters, and the ambiguous width changed between lines."""
    out = [resize(12, 6)]
    out.append(t("cafe\u0301 \u2764\ufe0f 1\ufe0f\u20e3 \U0001f44d\U0001f3fd\r\n"))
    out.append(t("\U0001f1ec\U0001f1e7\U0001f468\u200d\U0001f469\u200d\U0001f467 \u0915\u094d\u0937\r\n"))
    out.append(t(f"{at(3, 11)}ab\u2764\ufe0f{at(4, 12)}\u2764\ufe0f\u231a\ufe0e"))
    out.append(t(f"{CSI}?7l{at(5, 12)}\u2764\ufe0f{CSI}?7h"))
    out.append(t(f"{at(1, 1)}e\u0301{CSI}5b{at(2, 1)}{CSI}2@{CSI}3P{CSI}4h\u2764\ufe0f{CSI}4l"))
    out.append(t(f"{CSI}?2027l\u2764\ufe0f\U0001f1ec\U0001f1e7 e\u0301{CSI}?2027$p{CSI}?2027h"))
    out.append(ambiguous(2))
    out.append(t("\r\n\u00a7\u00b1\u2460\u2500\u2500 \u2194\ufe0e \u2194\ufe0f\r\n"))
    out.append(ambiguous(1))
    out.append(t("\u00a7\u00b1\u2460\u2500\u2500\r\n" + "a\u0301" * 40 + "\u0301" * 40))
    out.append(select(0, 0, 5, 11))
    out.append(resize(7, 6))
    out.append(resize(30, 6))
    return b"".join(out)


def scroll_regions():
    """DECSTBM, origin mode, margins with IL/DL/SU/SD, out-of-range values."""
    out = [t("".join(f"{i}\r\n" for i in range(30)))]
    out.append(t(f"{CSI}5;15r{CSI}?6h{CSI}H{CSI}20Bbottom{CSI}10Atop"))
    out.append(t(f"{CSI}3S{CSI}2T{CSI}4L{CSI}4M{ESC}D{ESC}M{ESC}E"))
    out.append(t(f"{CSI}?6l{CSI}0;0r{CSI}10;5r{CSI}1;999r{CSI}24;24r{CSI}r"))
    out.append(t(f"{CSI}2;23r{at(23, 1)}" + "scroll\n" * 30 + f"{CSI}r"))
    out.append(resize(80, 10))
    out.append(t(f"{CSI}3;8r{at(8, 1)}\n\n\n{CSI}r"))
    out.append(scroll_view(3))
    out.append(t(f"{CSI}65535S{CSI}65535T{CSI}65535L{CSI}99999999999M"))
    return b"".join(out)


def misc():
    """Titles, hyperlinks, tabs, charsets, saved cursor, queries, modes."""
    parts = [
        f"{OSC}0;title{ESC}\\{OSC}2;tab\x07{OSC}7;file://host/tmp{ST}",
        f"{OSC}8;;https://example.com{ST}link{OSC}8;;{ST}",
        f"{OSC}52;c;aGVsbG8={ST}{OSC}11;?{ST}{OSC}10;?\x07{OSC}4;1;rgb:ff/00/00{ST}",
        f"a\tb\tc{ESC}H{CSI}3g{CSI}0g{CSI}2I{CSI}2Z\r\n",
        f"{ESC}(0lqqk\r\nx  x\r\nmqqj{ESC}(B\r\n",
        f"{ESC}7{at(5, 5)}saved{ESC}8{CSI}s{CSI}u{ESC}#8",
        f"{CSI}c{CSI}>c{CSI}5n{CSI}6n{CSI}?2026$p{CSI}?1049$p{CSI}14t{CSI}18t",
        f"{CSI}?2026h{CSI}2Jsync{CSI}?2026l{CSI}>1u{CSI}?u{CSI}<u",
        f"{CSI}?7l" + "n" * 100 + f"{CSI}?7h{CSI}20h\n{CSI}20l",
        f"{ESC}P1$qm{ST}{ESC}Pq#0;2;0;0;0~~{ST}{ESC}_apc{ST}{ESC}^pm{ST}",
        f"{CSI}!p{ESC}c",
        "\x00\x05\x07\x08\x0b\x0c\x0e\x0f\x7f",
    ]
    out = [t(p) for p in parts]
    out.append(b"\xc3\x28 \xe2\x82 \xf0\x9f\x98 \xff\xfe \xed\xa0\x80")  # invalid UTF-8
    return b"".join(out)


def history():
    """Lots of scrollback with selections crossing history and screen."""
    out = [t("".join(f"{CSI}3{i % 8}m{i:05} {'word ' * (i % 12)}{CSI}m\r\n" for i in range(300)))]
    out.append(scroll_view(50))
    out.append(select(3, 4, 20, 10))
    out.append(t("more output while scrolled back\r\n" * 5))
    out.append(select(20, 70, 0, 0))
    out.append(scroll_view(-128))
    out.append(t("x" * 200))
    out.append(resize(40, 12))
    out.append(select(0, 0, 11, 39))
    out.append(t(f"{CSI}3J{CSI}2J"))
    out.append(CLEAR)
    out.append(resize(200, 60))
    return b"".join(out)


SEEDS = {
    "seed-vim": vim,
    "seed-less": less,
    "seed-sgr": sgr,
    "seed-wide": wide,
    "seed-clusters": clusters,
    "seed-scroll-regions": scroll_regions,
    "seed-misc": misc,
    "seed-history": history,
}


def main():
    check = "--check" in sys.argv[1:]
    os.makedirs(OUT, exist_ok=True)
    stale = []
    for name, make in SEEDS.items():
        data = make()
        path = os.path.join(OUT, name)
        if check:
            try:
                with open(path, "rb") as f:
                    if f.read() != data:
                        stale.append(name)
            except FileNotFoundError:
                stale.append(name)
        else:
            with open(path, "wb") as f:
                f.write(data)
            print(f"{path}: {len(data)} bytes")
    if stale:
        print("stale seeds (run scripts/fuzz-seed.py): " + ", ".join(stale), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
