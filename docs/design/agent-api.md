# Agent API: driving the terminal from agents and tests

Status: **approved** (30 Oct 2026). Alex's decisions: build it; headless
by default, a window only on request; any of the user's own processes
may use it, through a socket, on by default in the app (off in pins;
`agent_api = false` turns it off).

What it is for: agents and tests start CLI and TUI programs in a real
Terminal Decline terminal, type into them, read the screen, wait for
output, take GPU-rendered screenshots and measure them, without opening
windows.

Modules named here:
- **term-app**: the new `ctl` module (the `terminal-decline ctl` command,
  the protocol, the server, headless sessions, screenshots), with a
  small hook added to the `session` module (each batch of output and
  its parse time) and the `agent_api` setting in `config` and the
  preferences' schema.

## 1. Shape

```
terminal-decline ctl spawn --name top --size 100x30 -- htop
terminal-decline ctl wait-for top --text "Load average" --timeout 5000
terminal-decline ctl send top --key F6 --text "q"
terminal-decline ctl screen top --json
terminal-decline ctl screenshot top /tmp/top.png
```

Each call is a client: it connects to the server's Unix socket, sends one
request as a line of JSON, prints the reply (one line of JSON; `screen`
without `--json` prints the screen's text) and exits with a documented
status (§4). The server is the running app when there is one (role App,
`agent_api` on), otherwise a **headless server** the client starts on
demand: `terminal-decline ctl serve`, the same binary with no
`NSApplication` at all, so no Dock icon, no menu bar and no windows. It
exits once its last session has ended and nothing has connected for a
short idle (10 s; `TERMINAL_DECLINE_CTL_IDLE_MS`).

## 2. Socket and who may use it

- `$TMPDIR/terminal-decline-$UID/ctl.sock` (`ctl socket` prints it);
  `TERMINAL_DECLINE_CTL_DIR` moves it, which the tests use so they never
  meet Alex's running app.
- The directory is made `0700`, and refused if it belongs to someone else
  or others can enter it; the socket is `0600`.
- Every connection's peer is checked with `getpeereid`: another uid is
  refused before anything is read.
- One server per directory: it holds an exclusive `flock` on `ctl.lock`
  while it serves. A second server (the app starting while a headless
  server runs) finds it held and doesn't serve; clients keep using the
  first. A stale socket (nobody holding the lock) is removed.
- Pins never serve and refuse `ctl` like any other argument (pins.md
  §3.6). `agent_api = false` in the config stops the app serving and the
  headless server starting.

## 3. Sessions

A session is the app's own `Session` (term-core's parser and screen,
term-pty's PTY, the reader and writer threads, images) with no view: its
terminal is made as a window's pane makes it (the config's scrollback,
ambiguous width, theme colours, cell pixel size from the fonts), so
programs see what they see in a window. Nothing is drawn until a
screenshot asks: the frame is built with term-render's renderer into an
offscreen BGRA texture, read back and written as PNG, with the session's
fonts, theme and scale (default: the config's, scale 2).

- **Keys** go through the app's own key encoding (`keys::translate`): a
  key name becomes the `KeyInput` the view would build from an `NSEvent`
  on a US layout, so xterm sequences, DECCKM, the keypad and every kitty
  keyboard protocol level the program asked for come out as typed keys
  do, releases included when it asked for them. Alt is Meta. Text is
  typed key by key; `--paste` goes as a paste (bracketed when asked).
- **Mouse** events use the app's mouse encoding (X10 or SGR, as the
  program asked); they are refused while it hasn't asked for any.
- **Screen** reads the visible rows (and history with `--scrollback N`);
  `--json` adds each cell's text, resolved colours and attributes, the
  cursor, title, alternate screen and modes.
- **Wait** for text, a regex, the output to settle (`--idle MS`) or the
  program to exit, each with a timeout. It sleeps on a condition the
  reader thread signals after each batch: no polling.
- **Record** writes asciicast v2 (output, resizes, and input with
  `--input`) as it happens.
- **Stats**: bytes in and out, batches, parse time, the last
  screenshot's build and GPU times, history bytes, pid, exit status, and
  the server's `phys_footprint`.

A program that exits quickly with a failure leaves the app's usual
"[process exited with status N]" in a window; a headless session doesn't
write it (the exit status is in `stats` and `wait-for --exit`).

`--window` (watching a session in a real background window) is left for
later (§6).

## 4. Protocol and exit codes

Requests are JSON objects with a `cmd` (`hello`, `spawn`, `send`,
`screen`, `wait-for`, `resize`, `screenshot`, `stats`, `record`, `list`,
`kill`), one per line; each gets one line back: `{"ok": true, …}` or
`{"ok": false, "error": "…"}`. A connection may send several.

Exit codes of `terminal-decline ctl`: 0 done (for `wait-for`: matched);
1 the server refused or failed the request; 2 bad arguments; 3
`wait-for` timed out; 4 no server could be reached or started; 5
`wait-for` text or regex: the program exited without it showing.

## 5. Costs

Measured on Alex's Mac (M-series), headless server, release build: see
§7 once landed.

## 6. Left for later

- `--window`: a background window (as `background.rs` does under the
  test control) showing a session, for watching. In the app, sessions
  are headless too until then.

## 7. Landed

(Filled in as it lands.)
