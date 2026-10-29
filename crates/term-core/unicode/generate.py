#!/usr/bin/env python3
"""Generate term-core's Unicode table (src/unicode/table.rs).

Reads the UCD files of the Unicode version `unicode-width` uses, so the
widths agree with it scalar by scalar (tests/unicode.rs checks this, with
the documented exceptions). Downloads the files into a cache directory on
first use:

    python3 crates/term-core/unicode/generate.py [--ucd DIR]

Per scalar the table gives (see src/unicode/mod.rs for the bit layout):
- width 0, 1 or 2 (Emoji_Presentation characters are 2, as in
  unicode-width, even where East Asian Width says Ambiguous), or "control" (C0 and C1 controls, never printed);
- whether it is East Asian Ambiguous: width 1, and 2 where ambiguous
  characters are wide (`unicode-width`'s `width_cjk`);
- the Grapheme_Cluster_Break class;
- Extended_Pictographic, Indic_Conjunct_Break, Emoji_Presentation,
  Emoji_Modifier_Base, Emoji_Modifier;
- whether it is the base of an emoji variation sequence.

The width rules are the per-character rules of unicode-width 0.2's
scripts/unicode.py (`load_east_asian_widths`, `load_zero_widths`), with
one change: U+17D8 KHMER SIGN BEYYAL is 1, not 3 (a cell is 1 or 2 wide,
and `string-width` says 1).
"""

import argparse
import os
import re
import subprocess
import sys
import urllib.request

UNICODE_VERSION = (18, 0, 0)
NUM = 0x110000
BASE_URL = "https://www.unicode.org/Public/{}.{}.{}/ucd/".format(*UNICODE_VERSION)

FILES = [
    "UnicodeData.txt",
    "EastAsianWidth.txt",
    "DerivedCoreProperties.txt",
    "HangulSyllableType.txt",
    "PropList.txt",
    "LineBreak.txt",
    "extracted/DerivedGeneralCategory.txt",
    "auxiliary/GraphemeBreakProperty.txt",
    "auxiliary/GraphemeBreakTest.txt",
    "emoji/emoji-data.txt",
    "emoji/emoji-variation-sequences.txt",
]

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)

# Grapheme_Cluster_Break classes, in the order of `Gcb` in src/unicode/mod.rs.
GCB = [
    "Other",
    "CR",
    "LF",
    "Control",
    "Extend",
    "ZWJ",
    "Regional_Indicator",
    "Prepend",
    "SpacingMark",
    "L",
    "V",
    "T",
    "LV",
    "LVT",
]
INCB = ["None", "Linker", "Consonant", "Extend"]

# Bit layout of one entry (u16); must match src/unicode/mod.rs.
WIDTH_CONTROL = 3
SHIFT_AMBIGUOUS = 2
SHIFT_GCB = 3
SHIFT_EXT_PICT = 7
SHIFT_INCB = 8
SHIFT_EMOJI_PRESENTATION = 10
SHIFT_MODIFIER_BASE = 11
SHIFT_MODIFIER = 12
SHIFT_VS_BASE = 13

# Two-level lookup: LEAVES[ROOT[c >> SHIFT]][c & MASK] is an index into PROPS.
SHIFT = 7


def fetch(ucd, name):
    path = os.path.join(ucd, os.path.basename(name))
    if not os.path.exists(path):
        os.makedirs(ucd, exist_ok=True)
        print("fetching", name, file=sys.stderr)
        urllib.request.urlretrieve(BASE_URL + name, path)
    return path


def ranges(ucd, name):
    """(low, high, value) for each data line: `XXXX(..YYYY)? ; value`."""
    line_re = re.compile(r"^([0-9A-F]+)(?:\.\.([0-9A-F]+))?\s*;\s*([^#]*?)\s*(?:#|$)")
    with open(fetch(ucd, name), encoding="utf-8") as f:
        for line in f:
            if m := line_re.match(line):
                low = int(m.group(1), 16)
                high = int(m.group(2), 16) if m.group(2) else low
                yield low, high, m.group(3)


def property_set(ucd, name, value):
    out = set()
    for low, high, v in ranges(ucd, name):
        if v == value:
            out.update(range(low, high + 1))
    return out


def east_asian_widths(ucd):
    """'N' (narrow), 'W' (wide) or 'A' (ambiguous), as unicode-width's
    `load_east_asian_widths`: missing code points are narrow."""
    codes = {"N": "N", "Na": "N", "H": "N", "W": "W", "F": "W", "A": "A"}
    eaw = ["N"] * NUM
    for low, high, v in ranges(ucd, "EastAsianWidth.txt"):
        for c in range(low, high + 1):
            eaw[c] = codes[v]
    for c in property_set(ucd, "LineBreak.txt", "AI"):
        eaw[c] = "A"
    gc = general_categories(ucd)
    for c in range(NUM):
        if eaw[c] == "A" and gc[c] in ("Lu", "Ll", "Lt", "Lm", "Lo", "Sk"):
            eaw[c] = "N"
    eaw[0x0387] = "A"
    stroke = re.compile(r"([0-9A-F]+);(?:[^;]*;){4}([0-9A-F]+) 0338;")
    with open(fetch(ucd, "UnicodeData.txt"), encoding="utf-8") as f:
        for line in f:
            if m := stroke.match(line):
                if eaw[int(m.group(2), 16)] == "A":
                    eaw[int(m.group(1), 16)] = "A"
    return eaw


_gc = None


def general_categories(ucd):
    global _gc
    if _gc is None:
        _gc = ["Cn"] * NUM
        for low, high, v in ranges(ucd, "extracted/DerivedGeneralCategory.txt"):
            for c in range(low, high + 1):
                _gc[c] = v
    return _gc


def zero_widths(ucd):
    """unicode-width's `load_zero_widths`."""
    zw = [False] * NUM
    for low, high, v in ranges(ucd, "DerivedCoreProperties.txt"):
        if v in ("Default_Ignorable_Code_Point", "Grapheme_Extend"):
            for c in range(low, high + 1):
                zw[c] = True
    zw[0xFF9E] = zw[0xFF9F] = False
    for low, high, v in ranges(ucd, "HangulSyllableType.txt"):
        if v in ("V", "T"):
            for c in range(low, high + 1):
                zw[c] = True
    for c in (0x070F, 0x0605, 0x0890, 0x0891, 0x08E2):
        zw[c] = True
    prepend = property_set(ucd, "auxiliary/GraphemeBreakProperty.txt", "Prepend")
    prepend -= property_set(ucd, "PropList.txt", "Prepended_Concatenation_Mark")
    for c in prepend:
        zw[c] = True
    zw[0x115F] = False
    zw[0x2D7F] = False
    zw[0xA8FA] = True
    return zw


def build(ucd):
    eaw = east_asian_widths(ucd)
    zw = zero_widths(ucd)
    gc = general_categories(ucd)

    gcb = [0] * NUM
    for low, high, v in ranges(ucd, "auxiliary/GraphemeBreakProperty.txt"):
        for c in range(low, high + 1):
            gcb[c] = GCB.index(v)

    incb = [0] * NUM
    incb_re = re.compile(r"InCB;\s*(\w+)")
    for low, high, v in ranges(ucd, "DerivedCoreProperties.txt"):
        if m := incb_re.fullmatch(v):
            for c in range(low, high + 1):
                incb[c] = INCB.index(m.group(1))

    emoji = {}
    for name in ("Extended_Pictographic", "Emoji_Presentation", "Emoji_Modifier_Base", "Emoji_Modifier"):
        emoji[name] = property_set(ucd, "emoji/emoji-data.txt", name)

    vs_base = set()
    seq_re = re.compile(r"^([0-9A-F]+) FE0F\s*;\s*emoji style")
    with open(fetch(ucd, "emoji/emoji-variation-sequences.txt"), encoding="utf-8") as f:
        for line in f:
            if m := seq_re.match(line):
                vs_base.add(int(m.group(1), 16))

    props = [0] * NUM
    for c in range(NUM):
        if gc[c] == "Cc":
            width, amb = WIDTH_CONTROL, 0
        elif zw[c]:
            width, amb = 0, 0
        elif GCB[gcb[c]] == "Regional_Indicator":
            # Emoji_Presentation, but one cell alone (a flag is a pair).
            width, amb = 1, 0
        elif c == 0x17A4 or eaw[c] == "W" or c in emoji["Emoji_Presentation"]:
            width, amb = 2, 0
        else:
            # U+17D8 is 3 in unicode-width; 1 here (see the docstring).
            width, amb = 1, int(eaw[c] == "A")
        p = width | amb << SHIFT_AMBIGUOUS | gcb[c] << SHIFT_GCB | incb[c] << SHIFT_INCB
        p |= int(c in emoji["Extended_Pictographic"]) << SHIFT_EXT_PICT
        p |= int(c in emoji["Emoji_Presentation"]) << SHIFT_EMOJI_PRESENTATION
        p |= int(c in emoji["Emoji_Modifier_Base"]) << SHIFT_MODIFIER_BASE
        p |= int(c in emoji["Emoji_Modifier"]) << SHIFT_MODIFIER
        p |= int(c in vs_base) << SHIFT_VS_BASE
        props[c] = p
    return props


def emit(props, out_path):
    values = sorted(set(props))
    assert len(values) <= 256, len(values)
    index = {v: i for i, v in enumerate(values)}
    block = 1 << SHIFT
    leaves = []
    leaf_index = {}
    root = []
    for start in range(0, NUM, block):
        leaf = tuple(index[p] for p in props[start : start + block])
        if leaf not in leaf_index:
            leaf_index[leaf] = len(leaves)
            leaves.append(leaf)
        root.append(leaf_index[leaf])
    root_type, root_size = ("u8", 1) if len(leaves) <= 256 else ("u16", 2)
    assert len(leaves) <= 0x10000

    size = len(values) * 2 + len(root) * root_size + len(leaves) * block
    with open(out_path, "w", encoding="utf-8") as f:
        w = f.write
        w("// Generated by crates/term-core/unicode/generate.py from the Unicode\n")
        w("// {}.{}.{} character database. Do not edit.\n".format(*UNICODE_VERSION))
        w("//\n")
        w(f"// {len(values)} distinct entries, {len(root)} root slots, {len(leaves)} leaves of {block}: {size} bytes.\n\n")
        w("/// The Unicode version the table comes from.\n")
        w("pub const UNICODE_VERSION: (u8, u8, u8) = ({}, {}, {});\n\n".format(*UNICODE_VERSION))
        w(f"pub(super) const SHIFT: u32 = {SHIFT};\n\n")
        w(f"pub(super) static PROPS: [u16; {len(values)}] = [\n")
        for i in range(0, len(values), 12):
            w("    " + ", ".join(f"0x{v:04X}" for v in values[i : i + 12]) + ",\n")
        w("];\n\n")
        w(f"pub(super) static ROOT: [{root_type}; {len(root)}] = [\n")
        for i in range(0, len(root), 16):
            w("    " + ", ".join(str(v) for v in root[i : i + 16]) + ",\n")
        w("];\n\n")
        w(f"pub(super) static LEAVES: [[u8; {block}]; {len(leaves)}] = [\n")
        for leaf in leaves:
            w("    [\n")
            for i in range(0, block, 32):
                w("        " + ", ".join(str(v) for v in leaf[i : i + 32]) + ",\n")
            w("    ],\n")
        w("];\n")
    # Formatted as `cargo fmt` would, so it never changes the file.
    subprocess.run(["rustfmt", "--edition", "2024", out_path], check=True)
    print(f"{out_path}: {size} bytes ({len(values)} entries, {len(leaves)} leaves)", file=sys.stderr)


def emit_break_test(ucd, out_path):
    """The test lines of GraphemeBreakTest.txt, without their comments."""
    with open(fetch(ucd, "auxiliary/GraphemeBreakTest.txt"), encoding="utf-8") as f:
        lines = f.readlines()
    with open(out_path, "w", encoding="utf-8") as out:
        for line in lines[:5]:
            out.write(line)
        out.write("# Test lines only; comments removed by crates/term-core/unicode/generate.py.\n")
        for line in lines:
            data = line.split("#", 1)[0].strip()
            if data:
                out.write(data + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--ucd", default=os.path.join(HERE, ".ucd-cache"), help="directory for the UCD files")
    args = parser.parse_args()
    props = build(args.ucd)
    emit(props, os.path.join(CRATE, "src", "unicode", "table.rs"))
    emit_break_test(args.ucd, os.path.join(CRATE, "tests", "data", "GraphemeBreakTest.txt"))


if __name__ == "__main__":
    main()
