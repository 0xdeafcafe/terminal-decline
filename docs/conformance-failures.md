# Conformance failures

What vttest and esctest2 find wrong (see [conformance.md](conformance.md) for
how they run). Each section is a group in
`tests/conformance/accepted-failures.txt`, which lists the tests one by one.
Measured on 2026-10-29 against main at 67608bf: vttest 59 menus, 178
screens, 126 right; esctest2 567 tests, 265 pass, 41 skipped as known xterm
bugs, 261 fail.

Severity: **high** breaks programs people run; **medium** visible in some
programs; **low** only test programs or rare ones notice.

All the to-fix bugs are in term-core except where noted.

## To fix

### dec-graphics: missing DEC Special Graphics characters (low)

`ESC ( 0` then `b c d e h i _`. Expected (xterm, VT100): ␉ ␌ ␍ ␊ ␤ ␋ and a
blank for `_`. Actual: the plain letters and `_`. The line-drawing and other
characters are right. vttest `charsets-vt100:01`. Fix in
`dec_line_drawing` in `crates/term-core/src/term.rs`.

### rep: REP repeats after another control sequence (low)

`+ CSI 1 b CSI 10 b`. Expected: `++`; REP repeats the last character only
when it comes straight after it, so the second REP does nothing (xterm,
ECMA-48). Actual: twelve `+`. ncurses sends REP straight after the
character, so real programs don't hit this. vttest `iso6429-rep:01`.

### decaln: DECALN leaves the margins and the cursor (low)

`CSI 4;5 r` then `ESC # 8`. Expected: the screen fills with `E`, the
top/bottom margins reset to the whole screen and the cursor goes home.
Actual: only the fill; margins stay and the cursor stays where it was.
esctest `DECALN_ClearsMargins`, `DECALN_MovesCursorHome`.

### decid: no reply to DECID (low)

`ESC Z`. Expected: the same reply as primary DA, `CSI ? 62 ; 22 c`.
Actual: nothing. esctest `DECID_Basic`.

### decxcpr: no reply to the extended cursor position report (low)

`CSI ? 6 n`. Expected: `CSI ? row ; column ; 1 R`. Actual: nothing (plain
`CSI 6 n` works). esctest `DECDSR_DECXCPR`.

### decreqtparm: no reply to DECREQTPARM (low)

`CSI 0 x` and `CSI 1 x`. Expected: a parameters report such as
`CSI 2 ; 1 ; 1 ; 128 ; 128 ; 1 ; 0 x` (3 for `CSI 1 x`). Actual: nothing,
vttest shows "Bad format". VT100-only; nothing current uses it. vttest
`reports-decreqtparm:01`.

### decrqss: no DECRQSS replies (low to medium)

`DCS $ q m ST` (SGR), `DCS $ q SP q ST` (cursor style, DECSCUSR) and
`DCS $ q r ST` (margins, DECSTBM). Expected: `DCS 1 $ r <setting> ST`, for
example `DCS 1 $ r 0 m ST`. Actual: nothing, DCS strings are ignored. Vim
asks for the cursor style this way and waits for a reply that never comes
(it times out and carries on). esctest `DECRQSS_SGR`, `DECRQSS_DECSCUSR`,
`DECRQSS_DECSTBM`. The other DECRQSS settings are under not planned.

### osc-colors: colour palette changes and queries (medium)

- `OSC 4 ; n ; ? ST` (query palette colour n): expected
  `OSC 4 ; n ; rgb:rrrr/gggg/bbbb ST`; actual: no reply.
- `OSC 4 ; n ; #rrggbb ST` (set palette colour n): ignored.
- `OSC 10 / 11 / 12 ; #rrggbb ST` (set foreground, background, cursor):
  ignored, so a later `OSC 10 ; ? ST` reports the configured colour, not the
  one set.
- `OSC 104`, `OSC 110 / 111 / 112` (reset them), `OSC 5` (special colours):
  ignored.

Theme scripts (base16-shell and the like) and programs that set their own
colours do this. 26 esctest tests in `ChangeColor`, `ChangeDynamicColor`,
`ChangeSpecialColor`, `ResetColor`, `ResetSpecialColor`. The same tests with
X11-only colour formats (CIE, TekHVC, RGBI) are under not planned.

### Fixed with this harness: Return in newline mode (term-app)

`CSI 20 h` (LNM), then the Return key. Expected: CR LF; actual was CR. The
key translation in the app's key module now sends CR LF in newline mode.
vttest `reports-lnm` passes.

## Not planned

### lr-margins: left/right margins (DECLRMM, DECSLRM)

`CSI ? 69 h` then `CSI l ; r s`. Expected: margins that limit the cursor,
wrapping, scrolling, insert/delete and origin mode. Actual: `CSI ? 69 h` is
ignored and `CSI s` saves the cursor. 50 esctest tests (BS, CR, CUB, CUF,
CUP, CHA, CHT, CNL, CPL, DCH, ICH, IL, DL, SU, SD, IND, RI, LF, NEL, FF, VT,
REP, SM IRM, HPR, VPR, DECSET, SCORC). Safe in practice: programs ask with
`CSI ? 69 $ p` first, and we answer "not recognised". Medium if a program
ever needs it.

### deccolm: 132-column mode (DECCOLM)

`CSI ? 3 h` / `CSI ? 3 l`. Expected: 132 (or 80) columns, the screen
cleared, margins reset. Actual: ignored, as in xterm by default. 33 vttest
screens (marked `!unknown`: their expected text isn't written down) and 4
esctest tests.

### double-size: double-width and double-height lines (DECDWL, DECDHL)

`ESC # 6`, `ESC # 3`, `ESC # 4`. Expected: the line drawn twice as wide,
holding 40 columns. Actual: ignored; text past column 40 stays and tabs go
past column 40. 5 vttest screens.

### vt52: VT52 mode

`CSI ? 2 l`, then VT52 sequences (`ESC Y`, `ESC A`…). Actual: VT52 mode
isn't entered and its sequences print. 3 vttest screens.

### s8c1t: 8-bit replies (S8C1T)

`ESC SP G`. Expected: replies start with C1 bytes (0x9B for CSI). Actual:
replies stay 7-bit, which is what UTF-8 programs want. vttest
`vt220-8bit-controls:01`.

### charsets-96: 96-character sets and shifts

`ESC - A` (Latin-1 into G1), locking shifts into GR (`ESC ~`, `ESC }`,
`ESC |`), single shifts (`ESC N`, `ESC O`). Actual: ignored; G1 stays
ASCII. Pointless with UTF-8. 4 vttest screens.

### sl-sr: scroll left and right

`CSI n SP @` (SL) and `CSI n SP A` (SR). Actual: ignored. 2 vttest screens.

### protection: protected areas

`CSI 1 " q` (DECSCA), `ESC V` / `ESC W` (SPA/EPA), then `CSI ? J`,
`CSI ? K`, `CSI $ {` (selective erase). Expected: protected characters
survive. Actual: protection is ignored and selective erase erases
everything. 24 esctest tests, 1 vttest screen.

### rectangles: rectangular area operations

DECCRA (`CSI … $ v`), DECERA (`CSI … $ z`), DECFRA (`CSI … $ x`), DECSERA
(`CSI … $ {`), DECIC (`CSI n ' }`), DECDC (`CSI n ' ~`). Actual: ignored.
VT420. 30 esctest tests.

### decbi-decfi: back and forward index

`ESC 6`, `ESC 9`. Actual: ignored. 8 esctest tests.

### reverse-wrap: reverse wraparound

`CSI ? 45 h` (and xterm's `?1045`). Expected: backspace at the left edge
goes to the end of the line above. Actual: the mode is ignored. 12 esctest
tests.

### decrqm: modes we don't know

`CSI n $ p` / `CSI ? n $ p` for modes decline doesn't have (KAM, SRM, ERM,
DECSCLM, DECSCNM, DECCOLM, DECNKM, DECBKM, DECLRMM…). Expected (xterm): 2
(reset) or 4 (permanently reset). Actual: 0 (not recognised), which is a
correct answer for a mode a terminal doesn't have. One worth noting:
reverse video (DECSCNM, `CSI ? 5 h`) isn't supported at all, which vttest's
screen features show. 24 esctest tests.

### decrqss: other settings

DECRQSS for DECSCA, DECSCL, DECSLPP, DECSLRM, DECSNLS, DECSSDT, DECSACE,
DECSASD. 8 esctest tests.

### dec-dsr: other device status reports

`CSI ? 15 n` (printer), `? 25 n` (UDK), `? 26 n` (keyboard), `? 55 n` /
`? 56 n` (locator), `? 75 n` (data integrity), `? 85 n` (multiple
sessions), `? 62 n` (macro space), `? 63 n` (memory checksum). Actual: no
reply. 9 esctest tests.

### identity: what the terminal says it is

Primary DA answers `CSI ? 62 ; 22 c` (a VT220 with colour) and secondary
DA `CSI > 1 ; 10 ; 0 c`. esctest at VT level 5 expects a VT525 (`? 65 …`,
`> 64 …`). By design. 4 esctest tests.

### decscl: conformance levels

`CSI 6n ; 1 " p`. Ignored. 2 esctest tests.

### window-ops: window manipulation

`CSI 1 / 2 t` (iconify), `3 t` (move), `4 / 8 t` (resize), `9 t`
(maximise), `10 t` (full screen), `11 / 13 / 15 / 19 t` (reports),
DECSLPP (`CSI 10 t`). Actual: ignored; programs don't get to move or resize
the window. 18 esctest tests.

### title-report: reporting the title

`CSI 20 t` / `CSI 21 t` (report icon label / window title): no reply, on
purpose (a program could read back a title another program set). The
title stack, `CSI 22 t` / `CSI 23 t` (push and pop), isn't supported
either; esctest can't test it without the report, but Vim uses it to put
the title back on exit, so it is worth adding (low). 11 esctest tests.

### xtsave: saving private modes

`CSI ? n s` / `CSI ? n r` (XTSAVE, XTRESTORE). Actual: ignored. 2 esctest
tests.

### morefix

`CSI ? 41 h` (xterm's fix for `more(1)` with tabs). 1 esctest test.

## Not checked

- Colours and attributes (vttest compares text only; esctest can't see
  them).
- DECRQCRA in origin mode: the harness answers DECRQCRA itself and ignores
  origin mode (esctest `DECSET_DECOM_DECRQCRA` also needs left/right
  margins, so it fails either way).
