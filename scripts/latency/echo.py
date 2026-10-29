#!/usr/bin/env python3
"""Echo each key straight back, for latency measurement.

    echo.py [--flood-first MARKER]

Puts the terminal in raw mode (no kernel echo, no line buffering), then
reads whatever bytes arrive and writes them back at once. Every 64 keys it
starts a new line so the screen scrolls like a shell would. `q` quits.

With `--flood-first MARKER`, the first window to run this (MARKER doesn't
exist yet: it is created) runs `yes` instead, and later ones echo. A
terminal started with this as its command then floods in its first window
and echoes in the next one (Cmd+N), the same way in any terminal.
"""

import os
import sys
import termios
import tty


def main():
    args = sys.argv[1:]
    if args[:1] == ["--flood-first"]:
        try:
            os.close(os.open(args[1], os.O_CREAT | os.O_EXCL | os.O_WRONLY))
            os.execvp("yes", ["yes"])
        except FileExistsError:
            pass
    fd = sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    tty.setraw(fd)
    out = sys.stdout.fileno()
    os.write(out, b"\x1b[2J\x1b[Hready\r\n")
    n = 0
    try:
        while True:
            data = os.read(fd, 1024)
            if not data or data == b"q":
                break
            n += len(data)
            os.write(out, data + (b"\r\n" if n % 64 == 0 else b""))
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old)


if __name__ == "__main__":
    main()
