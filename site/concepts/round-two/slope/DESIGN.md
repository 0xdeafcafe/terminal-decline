# Slope: round-two concept notes

Concept for decline.forbes.red, 7 November 2026, from checkout of
`rush/decline-site-round-two`. Written by Claude Opus 5.5
(`claude-opus-5-5`) against `site/concepts/round-two/BRIEF.md`,
`site/BRIEF.md`, `site/ANTI-SLOP.md`, `docs/brand/README.md` and
`README.md`. One file, `index.html` (about 275 KB), with inline CSS,
inline SVG and base64 PNGs, about 0.6 KB of clipboard script, and no
remote requests. Links go to GitHub only.

## The idea

The page takes its shape from the wordmark lockup and enlarges it. In
`wordmark.svg` the Slope's level centreline sits on the type baseline.
Here the real Slope is drawn at about 400 × 235 px at 1440 wide,
and the heavy title "A terminal for macOS." stands beside it. Its last
baseline sits on the rest's centreline, as the brand spec requires for
the lockup. The mark is ink, not background: spruce on mint in light
mode and mint on spruce in dark mode, with no texture, glow or repetition.

The descent and the rest set the page's two axes:

- **Drop column** (`--colA`, the Slope's width, at least 342 px): the
  Slope, then the captured terminal hanging directly under its rest. Lower
  down, every section heading sits in this column.
- **Rest column**: the title, then the two primary numbers and a short
  product description. Lower down, all section content hangs from this
  same left edge, so the rest of the page lies level along one line.

The page reads as one heavy drop at the top on the mint field, then a long,
level run on white. All content hangs from the same rest axis, with no
alternating bands. It closes on the mint field with installation. The
section order is: opening, the full 2 November measurements, ctl, pins and
completion. Completion comes last because the brief makes it supporting
detail. Sections vary: the measurement table with collapsed method, a
command block, a row of real pin artwork and a small terminal window.

This avoids the production layout: there is no text-left/terminal-right
hero and no alternating green bands. There are no cards, bento grid,
eyebrows, numbered steps or stats row.

## Brand geometry (exact)

- Slope path copied verbatim: `M294 400 L484 610 Q497 624 516 624 H738`,
  `stroke-width 92`, round caps and joins, no fill. The viewBox is the stroked
  bounds `248 354 536 316` (path bounds ± 46), so nothing is clipped and
  the aspect ratio is fixed. Width is derived from height in CSS
  (`--sw: calc(--sh * 536/316)`); no non-uniform scaling. Flat ink only:
  the icon's gradients, depth plane and lighting are icon-material features
  and are not used on the page.
- Lockup alignment: `.slope` has `margin-bottom: calc(F*.14 − sh*46/316)`.
  46/316 is the gap between the rest centreline (y624) and the bottom of the
  stroked bounds (y670). 0.14 em is the estimated gap between SF Pro's last
  baseline and the bottom of its line box at `line-height:1`. This is a
  metric estimate, not measured. In the one scratch view before capture was
  stopped, the rest centreline and baseline agreed within about 5 px at
  1440. With a fallback font they will drift slightly.
- Header wordmark: the outlined paths from `docs/brand/wordmark.svg`
  (sha256 246a0d8c…a21d21a797) are unchanged. The paper `<rect>` and
  `<title>` were removed, text fill became `currentColor` and the mark
  stroke a themed class. The viewBox is cropped to `50 84 600 116`, which
  keeps the full mark and name. No Apple font is shipped.
- Favicon: `docs/brand/icon.svg` as a base64 data URI, unchanged.

## Tokens

| Token | Value | Reason |
| --- | --- | --- |
| Spruce | `#092D2B` | Brand ink. Light-mode text, the Slope and wordmark on mint. Dark-mode opening ground. |
| Spruce top | `#245950` | Icon body top. Muted text in light mode (≈ 7.9:1 on white, ≈ 5.7:1 on mint). |
| Mint | `#B9E5CC` | Brand mint. Light-mode opening and closing fields, the Decline column in the table, the Slope in dark mode. |
| White | `#FFFFFF` | Light ground for the long run (brief prefers white to paper). |
| Deep | `#051A17` | Dark icon body bottom. Dark-mode ground below the opening. |
| Paper | `#F3F3EB` | Dark-mode text only, as in the spec's inverse lockup. Never a light ground. |
| Rules | spruce 18% / mint 20% | Table and command rules, tinted rather than grey. |
| Terminal | `#16181c` bg, `#d8dee9` fg, `#b5bd68` green, `#2e3a4f` selection | From the capture and production DESIGN.md. |
| Completion dim | `#a9b0bd` | Description and hint grey. **Approximated** by eye from the screenshot's dim text, not sampled. Production used the full foreground instead. |
| Cursor | `#e5b567` | **Approximated** from the screenshot's amber block cursor, not sampled. |
| Window controls | `#ff5f57 #febc2e #28c840` | Native-looking chrome, explicitly authorised in round-two BRIEF.md. aria-hidden and not interactive. |

Type is the native system stack (`-apple-system`, SF Pro), which the brand
deliberately chooses, and no font file is shipped. Title: weight 800,
`clamp(46px, 6.4vw, 96px)`, line-height 1, tracking −0.03em. Other sizes:
h2 28–40px/700, lead 24–32px/700, body 16/1.6, small 14/1.5. Commands use
`ui-monospace`/SF Mono at 14/21. The terminal uses Menlo 13px / 1.3 at
every width, with ligatures off. Radii: 10px on the terminal frames and 50%
on the control dots, nothing else. There is one shadow, used only on the
terminal frames: `0 12px 32px` spruce at 16% (black at 30% in dark mode).
There is no motion, and reduced motion is honoured anyway. The page scrolls
natively.

## Truth sources

- **Opening terminal**: `site/src/data/terminal-preview.json`
  (captured 2026-11-07, source sha256 efdb3b45…f1163c42). Its 14 rows are
  rendered verbatim as HTML text at build time, with adjacent styled runs,
  colours and bold kept. Production DESIGN.md documents the capture
  procedure. It displays recorded results; nothing runs.
- **Table and opening figures**: README Benchmarks, 2 November 2026,
  matching `site/src/data/comparison.json`. Idle 26/60/99, 10k lines
  19/56/130, four tabs 47/87/104, cat 0.58/0.66/1.05 s, cat CPU incl.
  WindowServer 0.66/2.04/1.63 s, idle CPU 0.02/0.03/0.12%, wakeups
  0.5/0.5/24. The table is transposed (metrics as rows) so it fits 390 px
  without horizontal scroll.
- **Method details**: README conditions and versions (7849ad4, 82c455f,
  Ghostty 1.4.0, kitty 0.49.2, load 3–6). The 30 October run is kept
  separate, from `docs/comparison-2026-10-30.md` lines 25, 167–168 and
  249: Ghostty 0.53 s against Decline's 0.61 s, idle 31.5 MB against
  67 MB, four runs each with Terminal.app three. Warp 3.6 s and
  Terminal.app 4.0 s come from the README. Input latency is noted as
  unmeasured.
- **Completion**: `site/src/data/completion-preview.json`, which
  transcribes `docs/img/screenshot-autocomplete.png` (sha256 eb698dc9…).
  The eight rows, ellipses, selected `--depth`, `➜  ~` prompt and the
  `⏎ put in · esc close` hint are as they appear in that screenshot. The
  window furniture is reconstructed, and the screenshot's folder listing
  and status line are omitted.
- **ctl**: README "Driving it from agents and tests", using the verified
  `--text "make test" --key enter` form from site/BRIEF.md, not the
  README's literal `\n`. The headless server with no Dock icon that leaves
  once idle is from the same README paragraph.
- **Pins**: `docs/brand/samples-generated/` (term-icon `slope-pins-2`).
  `rush-32.png` and `rush-16.png` are embedded unaltered. rush.png,
  htop.png, claude.png and logs.png (512 px; sha256 1f6f8409…,
  9ce57c8c…, 6309933e…, a73568a2…) were downscaled to 256 px with
  `sips -Z 256` for weight and are shown at 128 and 64 CSS px. They are
  labelled as generated artwork, not as Alex's Dock or rush's installed
  icon. The pin text ("own icon if it ships one, otherwise draws one", "keep
  working after updates") is from the README.
- **Install**: the README cask command, its two command names, macOS 14+,
  Apple Silicon releases and MIT.

## Copy and selection

The page has about 330 words of prose including method details. There is
one aside at most, no slogan and no em dashes. The Homebrew command is
plain text in the header with a text Copy button (an underline, not a
pill). The button uses `navigator.clipboard`, says "Copied" in a polite
live region, and if the clipboard is unavailable or refused, selects the
command and says to press ⌘C. The header command also has
`user-select: all`. In the terminals, chrome, prompt, cursor, marker and
hint are `user-select: none`; cells, flags and descriptions are
`user-select: text`.

## Small screens (not visually verified)

Below 860 px the two axes collapse to one column. The Slope moves above
the title at `clamp(110px, 30vw, 190px)` tall with its exact aspect, and the
title is 44–72px and may wrap. Below 560 px, completion descriptions move
under their flags and pin artwork shrinks. The opening terminal needs 38
Menlo columns: about 298 px of text plus 32 px of padding and a 2 px
border, so about 332 px. That fits a 390 px width with 20 px margins
(350 px). Commands wrap with `pre-wrap` and `overflow-wrap: anywhere`, so
long ctl lines break mid-token rather than overflow. The terminal `pre`
has `overflow-x: auto` only as a guard. If it ever scrolls, that is a bug
to report, not intended behaviour.

## Known weaknesses and uncertainties

- Only one scratch render was seen: light mode at 1440×900, before the
  final `&nbsp;` line-break tweaks. Before the stop request, a 1024 and a
  dark capture were attempted, but neither file was written or viewed.
  Tablet, 390 px and the whole dark design are unseen. Dark mode is the
  token swap above.
- At 1440 the rest column's content stops at about 1100 px, leaving a
  plain right margin beside the lead text. This is intended as the level
  "rest", but a reviewer may read it as unused space.
- The lockup baseline offset is a font-metric estimate (see above).
- The opening's lower tier is still two side-by-side blocks: the terminal
  under the Slope and the facts under the title. It is a lockup grid
  rather than production's columns, but it is the nearest thing to the
  old structure.
- Completion dim and cursor colours are approximated, not sampled.
- Pins and ctl only partly support the speed and footprint story: the ctl
  paragraph mentions the idle-exiting headless server. No footprint claim
  is made for pins because none has been measured.
- There is no social card or 404, which is acceptable for a concept file.
- Build: the template and `build.py` that inline the data live in this
  session's TMPDIR and are not committed. To regenerate, edit
  `index.html` directly or ask for the script.

## Review changes (cross-review by Claude Sonnet 5.5, 7 November 2026)

See `SLOP-REVIEW.md`. Changes made after the before captures:

- Phone opening recomposed (860 px and below): the title and a small Slope share one row, as in the wordmark lockup (`--sh: clamp(54px, 15vw, 96px)`, rest centreline still offset to the title's last baseline). The figures and the one-line description now come before the terminal. The Slope is no longer a separate 110 to 190 px spacer. Desktop composition is unchanged.
- Opening copy trimmed to "Native Mac tabs and split panes." Metal, Core Text and Rust are dropped as implementation narration. The macOS 14, Apple Silicon and MIT requirements stay with Install.
- Type scale: sans text under 16 px (small notes, captions, figcaptions, footer, Copy, status) is now 16 px, and the terminal title is 13 px. Sizes are now the clamped title, h2, lead, 16 body, 14 mono commands, 13 terminal.
- Terminal chrome greys are now tokens, tinted towards the capture's blue-grey: `--t-edge #3a4150` (frame border), `--t-bar #232730` (title bar), `--t-line #2f3440` (bar rule), `--t-rule #4c566a` (completion list border). The dark-mode shadow is `#020d0c` at 30% instead of pure black. Each stays a window-furniture value, not an accent.
- Spacing nudged onto the 8 pt scale (18 to 16, 9 to 8, 14 to 16).
- Pins now break out of the content column across both axes, and the Rush pin is 192 px. The agents section has less bottom padding. This varies the section rhythm that was previously four identical heading-left, content-right blocks.
