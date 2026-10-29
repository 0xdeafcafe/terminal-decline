#!/usr/bin/env python3
"""Writes the seed corpora for the fuzz targets: `parse` in fuzz/corpus/parse
(terminal output, with images: kitty graphics, iTerm2 files, Sixel) and the
term-image targets (`sixel`, `kitty_keys`, `iterm_args`, `inflate`, `sniff`)
in fuzz/corpus/<target>. (`decode` has real image files, committed as they are.)

The files are committed; re-run this after changing it:

    python3 scripts/fuzz-seed.py            # writes fuzz/corpus/parse/seed-*
    python3 scripts/fuzz-seed.py --check    # fails if the files are stale

Each seed is terminal output in the style of a real program, with the
fuzz-only commands from fuzz/steps.rs mixed in (0xFF 'R' = resize, 'V' =
scroll the view, 'S' = select, 'C' = clear the selection, 'A' = set the
ambiguous width).
"""
import base64
import os
import struct
import sys
import zlib

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


def png(width, height, rgba):
    """A tiny valid RGBA PNG, `rgba` being one pixel repeated."""

    def chunk(kind, body):
        crc = zlib.crc32(kind + body) & 0xFFFFFFFF
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", crc)

    raw = b"".join(b"\x00" + bytes(rgba) * width for _ in range(height))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def b64(data):
    return base64.b64encode(data).decode()


def kitty(keys, payload=b""):
    """One kitty graphics command: ESC _ G keys ; base64 payload ESC backslash."""
    body = keys + (";" + b64(payload) if payload else "")
    return f"{ESC}_G{body}{ST}"


def sixel(body, params="0;0;0"):
    return f"{ESC}P{params}q{body}{ST}"


SIXEL_BAND = '"1;1;8;12#0;2;100;0;0#1;2;0;100;0#0!8~$#1!8N-#0!8~'
PIXELS_2X2 = bytes([255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255])


def images_kitty():
    """Kitty graphics: raw and PNG transmission, chunks, ids, placements,
    queries, deletes, animation, Unicode placeholders, and the id-less
    frames mpv sends (no i=, so each is a new image)."""
    parts = [
        kitty("a=q,i=31,s=2,v=2,f=32", PIXELS_2X2),
        kitty("a=T,f=32,s=2,v=2,i=1,c=4,r=2", PIXELS_2X2),
        kitty("a=T,f=100,i=2,p=1,q=2", png(8, 8, (0, 128, 255, 255))),
        # Chunked, split across commands.
        kitty("a=t,f=32,s=2,v=2,i=3,m=1", PIXELS_2X2[:8]) + kitty("m=0", PIXELS_2X2[8:]),
        kitty("a=p,i=3,p=2,x=0,y=0,w=1,h=1,X=2,Y=1,z=-1"),
        kitty("a=p,i=1,p=3,c=10,r=3,C=1"),
        # id-less frames, like mpv: each replaces the last on screen.
        "".join(f"{CSI}H" + kitty("a=T,f=32,s=2,v=2,q=2", PIXELS_2X2) for _ in range(4)),
        # Image numbers, then a file and shared memory the terminal must refuse.
        kitty("a=T,I=7,f=32,s=2,v=2", PIXELS_2X2),
        kitty("a=t,t=f,f=100,i=9", b"/etc/passwd"),
        kitty("a=t,t=s,f=32,s=2,v=2,i=10", b"/td-no-such-shm"),
        # Compressed, and a size far past the payload.
        kitty("a=t,o=z,f=32,s=2,v=2,i=11", zlib.compress(PIXELS_2X2)),
        kitty("a=t,f=32,s=100,v=100,i=12", PIXELS_2X2),
        # Animation: a frame, a control, composition.
        kitty("a=f,i=1,r=1,x=0,y=0,s=1,v=1,f=32,z=40", b"\xff\x00\x00\xff"),
        kitty("a=a,i=1,s=3,v=1"),
        kitty("a=c,i=1,r=1,c=2"),
        # Unicode placeholders: id in the foreground colour, row and column as
        # combining diacritics (U+0305 and U+030D are 0 and 1).
        kitty("a=T,U=1,f=32,s=2,v=2,i=4,c=2,r=2", PIXELS_2X2),
        f"{CSI}38;5;4m" + "\U0010eeee\u0305\u0305\U0010eeee\u0305\u030d\r\n"
        + "\U0010eeee\u030d\u0305\U0010eeee\u030d\u030d" + f"{CSI}m\r\n",
        # Deletes: by id, at the cursor, by column, with data freed, everything.
        kitty("a=d,d=i,i=1"),
        kitty("a=d,d=A"),
        kitty("a=d,d=c,x=1"),
        kitty("a=d,d=I,i=2"),
        "text" + kitty("a=T,f=32,s=2,v=2,i=20", PIXELS_2X2) + "\r\n" * 30,
        kitty("a=d,d=a"),
        # Malformed.
        f"{ESC}_Gnot=keys{ST}{ESC}_G;{ST}{ESC}_Ga=Z,i=-1{ST}{ESC}_G" + "a=T," * 50 + ST,
    ]
    out = []
    for i, p in enumerate(parts):
        out.append(t(p))
        if i == 4:
            out.append(resize(30, 10))
        if i == 9:
            out.append(scroll_view(-3))
            out.append(select(0, 0, 9, 29))
        if i == 14:
            out.append(resize(80, 24))
        if i == 18:
            out.append(t(f"{CSI}?1049h"))
        if i == 20:
            out.append(t(f"{CSI}?1049l{CSI}3J"))
    return b"".join(out)


def images_iterm():
    """iTerm2 OSC 1337: inline files, sizes in cells, pixels and percent,
    names, multipart transfer, and bad arguments."""
    img = png(6, 4, (255, 128, 0, 255))
    parts = [
        f"{OSC}1337;File=inline=1;size={len(img)};name={b64(b'a.png')}:{b64(img)}\x07",
        f"{OSC}1337;File=inline=1;width=10;height=3;preserveAspectRatio=0:{b64(img)}{ST}",
        f"{OSC}1337;File=inline=1;width=50%;height=20px:{b64(img)}\x07",
        f"{OSC}1337;File=inline=0;name={b64(b'download.bin')}:{b64(b'x' * 64)}\x07",
        f"{OSC}1337;MultipartFile=inline=1;size={len(img)}\x07"
        + f"{OSC}1337;FilePart={b64(img[:30])}\x07{OSC}1337;FilePart={b64(img[30:])}\x07{OSC}1337;FileEnd\x07",
        f"{OSC}1337;File=inline=1;size=999999999:{b64(img)}\x07",
        f"{OSC}1337;File=inline=1;width=auto;height=auto;:%%%not base64%%%\x07",
        f"{OSC}1337;File=:\x07{OSC}1337;File={ST}{OSC}1337;File=;;;=;:{ST}",
        f"{OSC}1337;File=name={b64(b'n' * 3000)};inline=1:{b64(img)}\x07",
        f"{OSC}1337;SetMark\x07{OSC}1337;CursorShape=1\x07{OSC}1337;ReportCellSize\x07",
    ]
    out = [t(p) + t("\r\n") for p in parts]
    out.insert(3, resize(40, 12))
    return b"".join(out)


def images_sixel():
    """Sixel: palette, raster attributes, repeats, band moves, background
    select, an image never ended, and text after each."""
    parts = [
        sixel(SIXEL_BAND),
        sixel('#0;2;0;0;0#1;1;120;50;100;50!20~$-#1!20~', "9;1;0"),
        sixel('"1;1;4000;4000#2;2;0;0;100!1000~$-!1000~'),
        sixel("#0;2;100;100;100~~~~$#1;2;0;0;0NNNN-~~~~-~~~~", "0;2;0"),
        sixel("?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~"),
        sixel("#999;2;300;300;300~#0;3;0;0;0~#0;4~!0~!99999999~"),
        f"{ESC}Pq#0;2;100;0;0~~~~",  # never ended
        sixel(SIXEL_BAND),
    ]
    out = [t(p) + t("text after\r\n") for p in parts]
    out.insert(2, resize(20, 8))
    out.insert(5, scroll_view(-4))
    out.insert(7, select(0, 0, 7, 19))
    return b"".join(out)


def seeds_sixel():
    """sixel target: one byte of P1 (low nibble) and P2 (high), then the data."""
    bodies = [SIXEL_BAND, '"1;1;2;6#0;2;0;0;0#0!2~', "#0;1;0;100;50;50~$-~", "!99999~-!99999~"]
    return {f"seed-sixel-{i}": bytes([[0x00, 0x10, 0x21, 0x01][i]]) + t(b) for i, b in enumerate(bodies)}


def seeds_kitty_keys():
    keys = [
        "a=T,f=32,s=2,v=2,i=1,c=4,r=2",
        "a=t,f=100,i=2,p=1,q=2,m=1",
        "a=p,i=3,p=2,x=0,y=0,w=1,h=1,X=2,Y=1,z=-1,C=1",
        "a=d,d=i,i=1",
        "a=d,d=A",
        "a=q,i=31,s=1,v=1,f=24,t=d,o=z",
        "a=f,i=1,r=1,z=40,c=2",
        "I=7,U=1,a=T",
        "a=Z,i=-1,i=2,i=3",
    ]
    return {f"seed-keys-{i}": t(k) for i, k in enumerate(keys)}


def seeds_iterm_args():
    img = png(2, 2, (1, 2, 3, 255))
    bodies = [
        f"inline=1;size={len(img)};name={b64(b'a.png')}:{b64(img)}",
        "inline=1;width=10;height=3;preserveAspectRatio=0:AAAA",
        "width=50%;height=20px;inline=1:",
        "width=auto;height=auto;doNotMoveCursor=1",
        f"name={b64(b'n' * 600)};size=99999999999999999999",
        ";;;=;=;a=b=c:",
    ]
    return {f"seed-iterm-{i}": t(b) for i, b in enumerate(bodies)}


def seeds_inflate():
    """inflate target: one byte (slice size minus one), then a zlib stream."""
    data = [b"", b"hello hello hello hello", bytes(range(256)) * 8, b"\x00" * 5000]
    out = {f"seed-zlib-{i}": bytes([[0, 6, 63, 255][i]]) + zlib.compress(d) for i, d in enumerate(data)}
    out["seed-zlib-truncated"] = bytes([3]) + zlib.compress(bytes(range(256)))[:-6]
    out["seed-zlib-stored"] = bytes([7]) + zlib.compress(b"stored block", 0)
    return out


def seeds_sniff():
    """sniff target: the head of a few small files."""
    return {
        "seed-png": png(3, 5, (9, 8, 7, 255))[:64],
        "seed-png-truncated": png(3, 5, (9, 8, 7, 255))[:20],
        "seed-gif": b"GIF89a\x04\x00\x03\x00\x80\x00\x00" + b"\x00" * 16,
        "seed-jpeg": b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xc0\x00\x11\x08\x00\x03\x00\x04\x03\x01\x22\x00",
        "seed-webp": b"RIFF\x1a\x00\x00\x00WEBPVP8X\x0a\x00\x00\x00\x10\x00\x00\x00\x03\x00\x00\x04\x00\x00",
        "seed-junk": b"<html><body>not an image",
    }


# Seeds for the other targets, by corpus directory.
OTHER_SEEDS = {
    "sixel": seeds_sixel,
    "kitty_keys": seeds_kitty_keys,
    "iterm_args": seeds_iterm_args,
    "inflate": seeds_inflate,
    "sniff": seeds_sniff,
}


SEEDS = {
    "seed-images-kitty": images_kitty,
    "seed-images-iterm": images_iterm,
    "seed-images-sixel": images_sixel,
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
    files = {os.path.join(OUT, name): make() for name, make in SEEDS.items()}
    for target, make in OTHER_SEEDS.items():
        for name, data in make().items():
            files[os.path.join(ROOT, "fuzz", "corpus", target, name)] = data
    stale = []
    for path, data in files.items():
        if check:
            try:
                with open(path, "rb") as f:
                    if f.read() != data:
                        stale.append(os.path.relpath(path, ROOT))
            except FileNotFoundError:
                stale.append(os.path.relpath(path, ROOT))
        else:
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as f:
                f.write(data)
            print(f"{path}: {len(data)} bytes")
    if stale:
        print("stale seeds (run scripts/fuzz-seed.py): " + ", ".join(stale), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
