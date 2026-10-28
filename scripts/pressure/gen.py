#!/usr/bin/env python3
"""Generators for pressure-test inputs. Deterministic (fixed seeds).

usage: gen.py big PATH [BYTES]     # text for flood-1gb (default 100 MB)
       gen.py fuzz PATH BYTES      # escape-sequence soup for fuzz-64mb
       gen.py glyphs PATH          # every CJK/emoji/symbol, for glyphs-30k
       gen.py sync SECONDS         # writes synchronized-update frames to stdout
"""
import os, random, sys

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
