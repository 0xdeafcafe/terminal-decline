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
terminal-decline ctl spawn --window --name vim --size 100x30 -- vim   # in a real window, in the background
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
- **Wait** for text, a regex, the output to settle (`--idle MS`), the
  program to exit, or (`--command`, `--shell` sessions only) the next
  command the shell finishes, each with a timeout. `--command` reads the
  shell integration's OSC 133 marks (term-core `take_commands`) and
  returns the command's exit `status` (null if the shell gave none), its
  `duration_ms` (from its start mark to its end mark, as the terminal saw
  them, to a batch of output's precision) and its text; it waits for the
  command running now, if one is, else the next one to start. It sleeps on a condition the
  reader thread signals after each batch: no polling.
- **Record** writes asciicast v2 (output, resizes, and input with
  `--input`) as it happens.
- **Stats**: bytes in and out, batches, parse time, the last
  screenshot's build and GPU times, history bytes, pid, exit status, and
  the server's `phys_footprint`.

A program that exits quickly with a failure leaves the app's usual
"[process exited with status N]" in a window; a headless session doesn't
write it (the exit status is in `stats` and `wait-for --exit`).

### 3.1 `--window`: a session in a real window

`spawn --window` runs the session in a window of the app: the app's own
window and pane, drawn as any other, so what is seen is what a user would
see. The window opens in the background, as `background.rs` does for the
smoke tests: it takes no focus and doesn't bring the app forward, and it
opens behind the other windows. Every `ctl` command works on it as on a
headless session: the server reaches the pane's terminal and its input
through what the pane hands back when it starts the session (the
terminal, a sender into the PTY's writer, the process), `resize` sets the
window's size to show the grid, `screen` and `wait-for` read the same
terminal, and `screenshot` draws it headlessly as for any session (so it
is the session's fonts and scale, not the window's pixels).

- **Who serves.** Windows need the app. The running app serves them, in
  its normal foreground state (the window opens behind the others, and
  isn't made key). With no app serving, the client starts one: the same
  executable, with `TERMINAL_DECLINE_CTL_HOSTED=1`, which makes it a
  host: a background (accessory) app with no Dock icon, no window of its
  own, no updater and no restart into a new build, that serves the
  socket and leaves once it is idle, like the headless server. In the
  background, its windows are transparent and behind everything, as the
  smoke tests' are; `DECLINE_TEST_FOREGROUND=1` in the client's
  environment turns that off, to watch (the app then comes to the front
  once as it starts). Headless sessions can be spawned in the same app.
- **A headless server** (one that an earlier headless `spawn` started
  and which is still running) can't host a window: `spawn --window` is
  refused, with the reason. `hello` says whether the server hosts
  windows (`"windows": true`).
- **Closing and exiting.** Closing the window (by hand, or `kill`) hangs
  up on the program, and the session shows as exited like any other
  (`wait-for --exit` works; `kill` forgets it). A program that exits
  closes its window as in any window, except that one that fails within
  a second leaves the window open with its status, until a key is
  pressed. A program that can't be started is an error from `spawn`, with
  no window left behind.
- The window takes the app's config with the request's font, theme,
  appearance and size on top (`--size` is the window's grid). The
  program, directory and environment are the request's, as for a
  headless session; `--shell` gets the shell integration.

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

Measured on Alex's Mac (Apple Silicon, busy with other agents' builds,
so medians), headless server, release build, 30 Oct 2026:

| | |
|---|---|
| Headless server, no sessions | 2.1 MB `phys_footprint` |
| 50 idle `/bin/sh` sessions, 80×24 | 9.2 MB: **~146 KB per session** (the shells are their own processes) |
| `spawn` in the server | 3–6 ms median (max 23 ms) |
| `ctl spawn`, the whole call | 22–34 ms (a client process each time) |
| First output after spawn | 25–33 ms |
| `ctl hello` round trip | 13 ms; a cold start of the server 27 ms (up to 370 ms on a binary's first run) |
| 20 MB of text through one session | parse 116–157 ms (130–175 MB/s) |
| Screenshot 120×40 at 2× (1940×1300 px) | first 36 ms frame build, 4 ms GPU, 19 ms PNG; again 1–6 ms GPU, 8 ms PNG |
| `screen --json` 120×40 (230 KB) | 23–29 ms call |

## 6. Left for later

- A window opened by hand can't be turned into a session, and a session
  can't be given a window afterwards; `--window` is a choice at `spawn`.

## 7. Landed

31 Oct 2026: `--window` (§3.1) and `wait-for --command` (§3). The
`window` module in `ctl` (the app's way of opening a window, the hooks
that watch a window's session, the hosting app's environment), a
`Launch::adopt` in the window view, and `Session::input`. Tests: a
headless server refuses windows (`tests/ctl.rs`); the smoke tests
`smoke_ctl_windows_are_sessions_in_a_background_app` and
`smoke_ctl_window_sessions_mark_commands` run an app that hosts windows
(CI only: they open windows). `wait-for --command` is tested against a
headless `--shell` session.

30 Oct 2026: everything above but `--window`. The `ctl` module in
term-app (`protocol`, `client`, `server`, `headless`, `keys`, `screen`,
`shot`); `Hooks::on_output`, `Hooks::quiet_exit` and `Session::child`
in `session`; the `agent_api` setting. Tests: protocol round trips,
key encoding (legacy and kitty levels), screen reading and CLI parsing
as unit tests; `crates/term-app/tests/ctl.rs` runs the real binary
against a headless server in a directory of its own (sessions end to
end, keys, the socket's permissions, screenshots with text, program
colours and a kitty image), checking the server is no app (Launch
Services doesn't know it). The window smoke test for program-set
colours moved there.
