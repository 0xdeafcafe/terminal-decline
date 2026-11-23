# Hand-off: programs keep running across a restart

Status: part 1 landed on the `keep-programs-on-restart` branch (23 Nov
2026), part 2 designed only. Decided with Alex on 23 Nov 2026.

Landed in part 1: `Terminal::save_live` / `Term::restore_live`
(`term-core`), `Pty::hand_over` / `Pty::adopt` and the socket
(`term-pty::handoff`), `Session::hand_over` / `Session::adopt`, the
offer and the taker (`term-app::handoff`), `live` in the saved window
list (`term-restore`), and `!again` in `decline.zsh`. Tested up to two
sessions in one process; a restart between two real builds has not
been run yet (the smoke test of §7 isn't written: under the test
control the new build isn't started).

## 1. The problem

A restart into a new build (a dogfood install, Restart to Update, and
Sparkle's Quit and Install) ends every program: the old process closes
its PTYs, the shells get SIGHUP, and the new process starts new shells,
shows the saved history above them and types the command that was
running at the prompt, unrun (`term-restore`). So the app puts a
restart off while anything runs (`reload::busy`), and an update can
wait for days in a terminal that always has something running.

With a hand-off the new process takes over the old one's PTYs. The
programs never notice: same process, same terminal device, same
session. A restart no longer has to wait.

## 2. Decisions at a glance

| Question | Decision | Why |
|---|---|---|
| What is handed over | each pane's PTY master (a file descriptor) and a snapshot of its terminal | the master is the only thing keeping the session alive; the kernel doesn't care which process holds it |
| How | a Unix socket in `~/Library/Caches/terminal-decline` (folder 0700, socket 0600), descriptors with `SCM_RIGHTS` | the two processes are different builds, started by LaunchServices: nothing is inherited |
| Who serves | part 1: the old app, until the new one has taken everything. Part 2: a small keeper process, for restarts where the old app must exit first (Sparkle) | one protocol for both |
| The terminal's state | history and the primary screen as the scrollback-v2 blocks `restore_scrollback` already saves, exact this time; modes and the rest as **escape sequences** the new terminal parses | both are formats that already have to stay readable across builds; no struct is shared between two builds |
| A full-screen program | its screen isn't sent. The new terminal enters the alternate screen with the program's modes and the PTY's size is changed and put back, so the program repaints | every full-screen program repaints on SIGWINCH; its cells may be private (a password manager) |
| Exit status of an adopted program | unknown (the new process isn't its parent): its pane closes as after a clean exit | `waitpid` is for children only, and so is kqueue's `NOTE_EXITSTATUS` |
| When it fails | exactly today's restart: new shells, saved history, the command typed at the prompt | the hand-off is tried first and nothing else changes |
| Old build → older build | the saved window list's new field is ignored, the socket is never taken, and the old app gives up after `TAKE_TIMEOUT` | downgrades restart as today |

## 3. What happens

Old app, in `restart_into_new_build`:

1. Each pane with a program is given an id. The socket is made and
   listened on, and the window list is saved as now, each pane with
   `live: {socket, id}` beside its history file and command, which stay
   as the fallback.
2. The new build is started with `--restore`, as now.
3. The old app waits up to `TAKE_TIMEOUT` (10 s) for it to connect. Its
   windows are frozen meanwhile. Nothing has been given up yet: with no
   taker (an older build, one that didn't start) it quits as it does
   today.
4. With a taker, every pane's reader thread is stopped between two
   batches, so the snapshot has everything read and nothing more is
   taken off the PTY, and its `Pty` is disarmed: dropping it no longer
   hangs up on the program.
5. Each pane gives a `Live`: the master (duplicated), the program's pid,
   the terminal device's name, the size, the report token given to its
   shell, and the terminal's snapshot (§4). They are sent (a
   length-prefixed JSON header and the snapshot bytes, the descriptor in
   the same message's ancillary data); the old app waits for `taken`
   and quits.
6. A hand-over that fails part way: it quits anyway. The masters close
   with it and the programs get SIGHUP, as today.

New app, given `--restore`:

1. Before any window opens, if any pane names a socket: connect, read
   every `Live`, answer `taken`. The peer must be the same user
   (`getpeereid`).
2. A pane whose `live.id` was received adopts it (`Session::adopt`): the
   terminal is built from the snapshot, the reader and writer threads
   start on the master. No history file is read and nothing is typed.
3. The PTY is given the pane's real size. When that is the size it had,
   it is changed by a row and put back: a full-screen program repaints,
   a shell redraws its prompt in place.
4. A `Live` no pane claimed is dropped: its master closes and its
   program gets SIGHUP, as if its pane had closed.

`reload::busy` stays as the answer to "may this restart happen now"
for restarts that can't hand off (a pane whose snapshot failed, part 2
not built); with a hand-off ready the restart doesn't wait.

## 4. The terminal's snapshot

Two parts, both from `term-core` (`Terminal::save_live`,
`Terminal::restore_live`):

- **Blocks**: history and every row of the primary screen, as
  `save_history` writes them (styles, links, prompt marks, wrapped
  rows), with the number of screen rows. Restored into a terminal of the
  same size, the screen is cell for cell what it was. A different size
  reflows, as a resize does.
- **Replay**: bytes for the new terminal's parser, written in this
  order: the scroll region, tab stops, character sets, the saved
  cursor, private modes (cursor keys, keypad, origin, autowrap, insert,
  bracketed paste, focus events, the mouse modes, grapheme clusters,
  colour-scheme reports), the kitty keyboard flags of both screens, the
  cursor's shape, the title, the folder (OSC 7), the pen (SGR), the
  alternate screen (`CSI ? 1049 h`) if it was active, and last the
  cursor's position. Everything a program can set and expects to still
  be set.

Not carried: the alternate screen's cells (repainted, §2), images and
their placements, the selection, the scroll position, a find in
progress, synchronized-update mode (it ends), a partly received escape
sequence (lost; the program's next output starts clean), colours set
with OSC 4/10/11 (part 2).

The app's own per-pane state rides in the header: whether it runs the
user's shell or the window's program, its title override, and the token.
The shell's environment report (what completion knows) was sent once
and isn't again, so completion in an adopted shell lacks its commands
until its `PATH` or aliases change. `decline.zsh` takes `!again` on its
`ESC [ 7701 ~` widget and reports the environment and the line afresh.
The app doesn't type it yet: it may only be typed to a shell at its
prompt, and whether an adopted shell is at its prompt isn't carried
(part 2).

## 5. Shells outlive builds

After a hand-off a shell runs the `decline.zsh` of the build that
started it, for as long as it lives. So from now on:

- what the app types to a shell's widget must be ignored by an older
  snippet (as an unknown `ESC [ 7701 ~` payload is), never misread;
- a feature that needs a newer snippet must work without it, or know
  (the header carries the features the shell was given).

## 6. Part 2: Sparkle

Sparkle's installer waits for the app to exit, replaces the bundle and
starts the new one. Between the two nothing of ours runs, so the
masters need a holder: `terminal-decline-pin --keep <socket>` (the stub
already in `Contents/Helpers`, running from its old inode while the
bundle is replaced), started by the old app, which sends it every
`Live` and quits. The new app finds the socket in the saved window list
the same way and takes from the keeper, which then exits; untaken for
five minutes, it exits and the programs end.

With that, "Quit and Install" stops listing the programs it will end,
and a background update can install when the app is idle instead of
only at quit (updater.md §1, "When updates install"). Both are
decisions for after part 1 has been used.

## 7. Tests

- `term-core`: `save_live` then `restore_live` gives the same screen,
  cursor and modes, as a property over random input (the reflow
  property's generator, plus mode sequences).
- `term-pty`: a descriptor and header sent over the socket arrive; a
  program started on one `Pty`, released and adopted by another, keeps
  running and its output and input still flow; a disarmed `Pty`
  dropped doesn't end its program; an adopted one whose program exits
  reports the end.
- `term-app` (`pipeline`): a session with a shell running a program is
  handed to a second session in the same process: screen equal, the
  program still running, typing reaches it, the prompt redrawn once.
- Smoke (CI only): a dogfood restart with `sleep` running in a pane;
  after it the pane's program is the same pid.

## 8. Plan

1. `term-core`: `save_live` / `restore_live`.
2. `term-pty`: stoppable reader, `Pty::hand_over`, `Pty::adopt`, the
   socket.
3. `term-app`: `Session::hand_over` / `Session::adopt`; the restart and
   restore paths; `!again`.
4. A dogfood restart with programs running, and the smoke test.
5. Part 2, after part 1 has run in dogfood for a while: the keeper,
   typing `!again`, and restarts that no longer wait for `reload::busy`.
