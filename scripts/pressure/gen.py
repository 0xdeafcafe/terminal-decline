#!/usr/bin/env python3
"""Generators for pressure-test inputs. Deterministic (fixed seeds).

usage: gen.py big PATH [BYTES]     # text for flood-1gb (default 100 MB)
       gen.py fuzz PATH BYTES      # escape-sequence soup for fuzz-64mb
       gen.py glyphs PATH          # every CJK/emoji/symbol, for glyphs-30k
       gen.py sync SECONDS         # writes synchronized-update frames to stdout
       gen.py images-flood [N]     # N id-less 1600x1000 PNG frames, like mpv (default 500)
       gen.py images-ids [N]       # N kitty images with ids, reusing 64 ids (default 2000)
       gen.py images-tiny [N]      # N 1x1 kitty images, each with its own id (default 20000)
       gen.py images-abandoned [N] # N kitty uploads stopped after the first chunk, then RIS (default 10000)
       gen.py images-scrollback [LINES]  # an image every 20 lines through LINES lines (default 100000)
       gen.py sixel-flood [N]      # N 200x200 Sixel images, then a text marker (default 2000)
       gen.py sixel-huge [N]       # N Sixel images declaring 10000x10000 with little data (default 200)
The image generators write to stdout and are meant for a terminal with
images on (`TD_IMAGES=1`; run.py sets it for them); with images off they are
APC and DCS strings the terminal ignores.
"""
import base64, os, random, struct, sys, zlib

def fuzz(path, size):
    """Random soup of escape sequences, UTF-8 and control bytes."""
    r = random.Random(42)
    parts, n = [], 0
    csi_finals = "@ABCDEFGHIJKLMPSTXZ`abcdefghilmnpqrstu"
    while n < size:
        k = r.random()
        if k < 0.35:
            p = ";".join(str(r.choice([0, 1, 2, 5, 38, 48, r.randint(0, 70000)])) for _ in range(r.randint(0, 6)))
            s = "\x1b[" + r.choice(["", "?", ">", "<", "=", "!"]) + p + r.choice(["", " ", "$", "'"]) + r.choice(csi_finals)
        elif k < 0.45:
            s = "\x1b]" + r.choice(["0", "2", "7", "8", "10", "11", "52", "1337", "999"]) + ";" + "".join(chr(r.randint(32, 0x2FFF)) for _ in range(r.randint(0, 40))) + r.choice(["\x07", "\x1b\\", ""])
        elif k < 0.5:
            s = "\x1bP" + "".join(chr(r.randint(32, 126)) for _ in range(r.randint(0, 60))) + "\x1b\\"
        elif k < 0.6:
            s = "\x1b" + r.choice("78DEHMc=>()#") + r.choice("0B8A")
        elif k < 0.7:
            s = "".join(chr(r.randint(0, 31)) for _ in range(r.randint(1, 4)))
        elif k < 0.8:
            s = "".join(chr(r.choice([r.randint(0x300, 0x36F), r.randint(0x4E00, 0x9FFF), r.randint(0x1F300, 0x1FAFF), 0xFE0F, 0x200D, r.randint(0x2500, 0x259F)])) for _ in range(r.randint(1, 20)))
        else:
            s = "".join(chr(r.randint(32, 126)) for _ in range(r.randint(1, 80)))
        b = s.encode("utf-8", "surrogatepass")
        if r.random() < 0.02:  # invalid UTF-8
            b += bytes([r.randint(0x80, 0xFF)])
        parts.append(b)
        n += len(b)
    with open(path, "wb") as f:
        f.write(b"".join(parts) + b"\x1bc\x1b[0m\r\nALIVE\r\n")

def big(path, size):
    """About `size` bytes of plain and coloured lines, like a big log or build
    output: 4096 distinct lines repeated with increasing line numbers."""
    r = random.Random(1)
    words = "the quick brown fox jumps over lazy dog error warning info debug build test src main lib".split()
    lines = []
    for _ in range(4096):
        text = " ".join(r.choice(words) for _ in range(r.randint(0, 24)))
        k = r.random()
        if k < 0.1:
            text = f"\x1b[1;31merror\x1b[0m: {text}"
        elif k < 0.2:
            text = f"\x1b[38;5;{r.randint(0, 255)}m{text}\x1b[0m"
        lines.append(text)
    written, n = 0, 0
    with open(path, "w") as f:
        while written < size:
            out = "".join(f"{n + i:09} {line}\n" for i, line in enumerate(lines))
            f.write(out)
            written += len(out.encode())
            n += len(lines)

def glyphs(path):
    """Every CJK ideograph, emoji and symbol block: fills the glyph textures."""
    ranges = [(0x4E00, 0x9FFF), (0x3040, 0x30FF), (0xAC00, 0xB7FF), (0x1F300, 0x1FAFF), (0x2190, 0x2BFF), (0x0400, 0x04FF), (0x0370, 0x03FF)]
    out = []
    for lo, hi in ranges:
        line = []
        for c in range(lo, hi + 1):
            line.append(chr(c))
            if len(line) == 60:
                out.append("".join(line) + "\r\n")
                line = []
        out.append("".join(line) + "\r\n")
    data = "".join(out)
    with open(path, "w") as f:
        f.write(data + "\x1b[1m" + data + "\x1b[0m")

def sync_flood(seconds):
    """Full-screen coloured redraws wrapped in synchronized-update mode."""
    import time
    r = random.Random(7)
    end = time.time() + seconds
    w = sys.stdout.buffer
    while time.time() < end:
        frame = ["\x1b[?2026h\x1b[H"]
        for row in range(60):
            frame.append(f"\x1b[{row+1};1H\x1b[38;5;{r.randint(0,255)};48;5;{r.randint(0,255)}m" + "".join(chr(r.randint(65, 90)) for _ in range(200)))
        frame.append("\x1b[0m\x1b[?2026l")
        w.write("".join(frame).encode())
        w.flush()

def _png(w, h, seed):
    """An RGBA PNG of a gradient (cheap to make, small to send) that differs per seed."""
    r = random.Random(seed)
    variants = []
    for _ in range(16):
        k = r.randint(0, 255)
        variants.append(b"\x00" + b"".join(bytes(((x // 16 * 8 + k) & 255, (x // 16 + k) & 255, 255 - x // 16 * 2 & 255, 255)) for x in range(w)))
    raw = b"".join(variants[y % 16] for y in range(h))
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 1)) + chunk(b"IEND", b"")

def _kitty(keys, payload=b""):
    body = keys + (";" + base64.b64encode(payload).decode() if payload else "")
    return f"\x1b_G{body}\x1b\\".encode()

def _emit(chunks):
    w = sys.stdout.buffer
    for c in chunks:
        w.write(c)
    w.flush()

def images_flood(n):
    """mpv-like: no image id, a new 1600x1000 PNG frame drawn at the top left
    each time, so each replaces the one before and the pool has to let go."""
    frames = [_png(1600, 1000, i) for i in range(8)]
    _emit([b"\x1b[2J"] + [b"\x1b[H" + _kitty("a=T,f=100,q=2", frames[i % 8]) for i in range(n)] + [b"\x1b[0m\r\nALIVE\r\n"])

def images_ids(n):
    """Kitty images with ids, 64 ids reused, transmitted and placed in a grid
    of cells over and over, so each replaces an older image of the same id."""
    frames = [_png(640, 400, 100 + i) for i in range(8)]
    out = [b"\x1b[2J"]
    for i in range(n):
        out.append(b"\x1b[%d;%dH" % (i % 20 + 1, (i * 7) % 60 + 1) + _kitty(f"a=T,f=100,i={i % 64 + 1},c=12,r=4,q=2", frames[i % 8]))
    out.append(b"\x1b[0m\r\nALIVE\r\n")
    _emit(out)

def images_tiny(n):
    """Many 1x1 images, each its own id: the pool's bookkeeping, not its bytes."""
    px = bytes([200, 100, 50, 255])
    out = [b"\x1b[2J"]
    for i in range(n):
        out.append(b"\x1b[%d;%dH" % (i % 50 + 1, i % 100 + 1) + _kitty(f"a=T,f=32,s=1,v=1,i={i + 1},c=1,r=1,q=2", px))
    out.append(b"\x1b[0m\r\nALIVE\r\n")
    _emit(out)

def images_abandoned(n):
    """Uploads that never finish: the first chunk of a chunked transfer, then
    the next upload. The budget must be back at zero after RIS."""
    chunk = bytes(random.Random(5).getrandbits(8) for _ in range(3000))
    out = [_kitty(f"a=t,f=32,s=1000,v=1000,i={i + 1},m=1,q=2", chunk) for i in range(n)]
    out.append(b"\x1bc\x1b[0m\r\nALIVE\r\n")
    _emit(out)

def images_scrollback(lines):
    """An image every 20 lines through `lines` lines of text, so images scroll
    into history and are dropped with the lines that hold them."""
    frames = [_png(160, 100, 200 + i) for i in range(4)]
    out = []
    for i in range(lines):
        if i % 20 == 0:
            out.append(_kitty("a=T,f=100,c=8,r=3,q=2", frames[(i // 20) % 4]))
        out.append(f"{i:07} line of text\r\n".encode())
    out.append(b"\x1b[0m\r\nALIVE\r\n")
    _emit(out)

def sixel_flood(n):
    """Sixel images of 200x200 in a few colours, drawn one after another."""
    r = random.Random(11)
    out = [b"\x1b[2J"]
    for i in range(n):
        pal = "".join(f"#{c};2;{r.randint(0, 100)};{r.randint(0, 100)};{r.randint(0, 100)}" for c in range(8))
        bands = []
        for _ in range(34):  # 34 bands of 6 rows
            bands.append("".join(f"#{c}!{r.randint(5, 40)}{chr(63 + r.randint(1, 63))}" for c in range(8)) + "$")
        out.append(f"\x1b[H\x1bPq\"1;1;200;200{pal}{'-'.join(bands)}\x1b\\".encode())
    out.append(b"\x1b[0m\r\nALIVE\r\n")
    _emit(out)

def sixel_huge(n):
    """Sixel images that declare 10000x10000 and send a few rows: refused or
    cropped, with no huge allocation."""
    out = [b'\x1bPq"1;1;10000;10000#0;2;100;0;0#0!100~-!100~\x1b\\text\r\n' for _ in range(n)]
    out.append(b"\x1b[0m\r\nALIVE\r\n")
    _emit(out)

if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "fuzz":
        fuzz(sys.argv[2], int(sys.argv[3]))
    elif cmd == "big":
        big(sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 100_000_000)
    elif cmd == "glyphs":
        glyphs(sys.argv[2])
    elif cmd == "sync":
        sync_flood(float(sys.argv[2]))
    elif cmd == "images-flood":
        images_flood(int(sys.argv[2]) if len(sys.argv) > 2 else 500)
    elif cmd == "images-ids":
        images_ids(int(sys.argv[2]) if len(sys.argv) > 2 else 2000)
    elif cmd == "images-tiny":
        images_tiny(int(sys.argv[2]) if len(sys.argv) > 2 else 20000)
    elif cmd == "images-abandoned":
        images_abandoned(int(sys.argv[2]) if len(sys.argv) > 2 else 10000)
    elif cmd == "images-scrollback":
        images_scrollback(int(sys.argv[2]) if len(sys.argv) > 2 else 100000)
    elif cmd == "sixel-flood":
        sixel_flood(int(sys.argv[2]) if len(sys.argv) > 2 else 2000)
    elif cmd == "sixel-huge":
        sixel_huge(int(sys.argv[2]) if len(sys.argv) > 2 else 200)
    else:
        sys.exit(f"unknown command {cmd}")
