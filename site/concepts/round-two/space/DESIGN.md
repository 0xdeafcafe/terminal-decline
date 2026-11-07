# Space: idle memory drawn to scale

Round-two concept, 7 November 2026, from checkout 14f1c8e. One
self-contained `index.html`, inline CSS and assets, no remote resources,
about 1 KB of vanilla clipboard JavaScript. Not published, not wired into
the Astro site.

## The idea

The page is a product sheet whose one graphic is a scale drawing. Three
squares share a bottom-left corner. Each square's **area** is proportional
to the terminal's idle physical footprint from the 2 November comparison:
kitty 99 MB (outline, full size), Ghostty 60 MB (outline), Decline 26 MB
(the only solid square). Side lengths are the square roots of the ratios:
Ghostty 77.85%, Decline 51.25% of kitty's side. These percentages are
computed from `site/src/data/comparison.json` by the generator, not typed.

It reads like a paper-size or sensor-size diagram: the room each terminal
takes, with Decline's the one you can see as a filled block. The labels
are the only numbers in the drawing and each carries its name and version.
There is no axis, no bar, no arrow and no Slope in the graphic. The Slope
appears only where it belongs: the wordmark and the pin artwork.

Page geometry, top to bottom: header (wordmark, install command with
Copy, GitHub); a one-line heavy title; one key line with the cat time and
idle memory, dated, so it is in the first phone view; then a sheet with the drawing at
left (440px) and a specification list at right; then four sections that
each use a different split (terminal left and prose right; prose left and
pin artwork right; prose left and command block right; completion list
left and prose right); then Install and a short footer. There is no
alternating band colour, no card, and the first viewport ends with the
"How it was measured" heading and the captured terminal visible at
1440×900.

## Tokens

- Ground: white `#FFFFFF` (light); spruce `#092D2B` (dark).
- Ink: spruce `#092D2B` (light); `#E1F3E9` (dark, the brand's tinted
  foreground top). Muted text `#47625E` / `#B4CFC3`, the pair used by the
  background study; both above 4.5:1 on their grounds.
- Solid square: spruce with white label and mint `#B9E5CC` value (light);
  mint with spruce label and value (dark). Outlines are 1.5px in the ink
  colour. Rules are the ink at 28–30% alpha.
- Mint is used for the Decline value on spruce, the Copy button's hover
  and done states, and the dark-mode solid square. It is never body text
  on white.
- No cream, no paper `#F3F3EB`, no serif, no gradient except inside the
  pin artwork, which is the brand's own.
- Type: the Mac system stack for SF Pro (`-apple-system`, "SF Pro Text" /
  "SF Pro Display", `system-ui`), weights 400, 500, 600 and 800 (the title
  only). Sizes: 12px terminal title bar, 13px terminal, 14px captions, Copy and
  spec notes, 16px/1.5 body, 20px key line, 28px h2, title `clamp(2.75rem, 1.6rem + 3.4vw, 4.25rem)`.
  The header install command keeps "SF Mono" at 14px. Everything that is
  terminal content (the captured views, completion descriptions at every
  width, the command specimens in the pin, ctl and Install sections) is
  Menlo 13px / 1.3.
- Spacing: layout gaps, margins and padding are multiples of 8px (8, 16,
  24, 32, 40, 48, 56, 64, 72). The half step, 4px, is used only for the
  completion command margin, stacked rows and the Decline value in the
  drawing; 12px only for the Copy button's horizontal padding. Square
  sizes are percentages from the data and are untouched.
- Radii: 12px on the two terminal frames (the authorised chrome
  exception), 6px on the Copy button. One shadow, spruce tinted, only on
  the terminal frames.
- Motion: the Copy button's 150ms colour transition, inside
  `prefers-reduced-motion: no-preference`. Nothing else moves.
- Measure: prose at most 58ch, the lead 40ch, captions 42ch.

## Truth sources

- Figures in the spec list and the drawing: README "Benchmarks" table,
  2 November 2026 (Decline 7849ad4, `cat` and idle CPU at 82c455f;
  Ghostty 1.4.0; kitty 0.49.2; M1 Max, macOS 27.0, Menlo 13, 100×30,
  10k scrollback, physical footprint, medians of 5 interleaved runs, load
  3–6). Values also in `site/src/data/comparison.json`.
- The comparison terminal: `site/src/data/terminal-preview.json`, the
  7 November capture of a display program printed into a real Decline
  session (38 columns, `#16181c` ground, `#d8dee9` and `#b5bd68` runs, bold
  preserved). Provenance and capture commands are in `site/DESIGN.md`.
  Rendered as HTML spans in Menlo 13/1.3; rows joined without stray
  newlines so `white-space: pre` shows exactly the captured rows. The
  caption says the bars are characters and that nothing runs live.
- The completion list: `site/src/data/completion-preview.json`, a
  transcription of `docs/img/screenshot-autocomplete.png` (eight rows,
  ellipses retained, `--depth` selected). Shown with the command
  `git fetch -` and no prompt marker, footer hint or folder listing,
  because those were not transcribed. Selected row `#2e3a4f` and
  foreground-coloured descriptions follow the production choice.
- Earlier results: `docs/comparison-2026-10-30.md` (Ghostty 0.53 s to
  Decline 0.61 s, 31.5 MB to 67 MB idle, 4 runs, no latency measured) and
  the README's Warp/Terminal.app `cat` times, kept in the method text and
  never merged with the 2 November table.
- ctl commands: README "Driving it from agents and tests", with the
  shell example in the verified form `--text "make test" --key enter`
  rather than the README's literal `\n`. Labelled as examples with no
  output shown.
- Pin commands and copy: README "Pin a program to the Dock" and
  `docs/brand/README.md` (fixed palette, initial plus Slope, own icon if
  shipped).
- Artwork: `docs/brand/wordmark.svg` paths with the paper rectangle
  removed, `currentColor`, and the viewBox `50 80 600 120` from
  `site/src/components/Wordmark.tsx`; `docs/brand/pin-template.svg` (the
  Rush example, index 5) with ids namespaced; the real 32 and 16 px
  samples for rush, htop, Claude and Logs from
  `docs/brand/samples-generated/` inlined as base64 PNG (about 1–2 KB
  each). The 512 px PNGs were not inlined (190 KB each). The favicon is
  the icon geometry as a data URI.

## Chrome and selection

The two terminal views have a Mac-style title bar labelled Terminal
Decline, 12px radius, 1px border and a tinted shadow, the exception Alex
asked for. The three window controls are drawn grey, as macOS draws an
unfocused window, so they are not traffic lights and do not pretend to
work; the bar is `aria-hidden` and `user-select: none`. The cells, the
install command, the command blocks and all prose are selectable.

## Small screens

Below 900px the sheet stacks: drawing, then spec list. The drawing is
`aspect-ratio: 1` at the content width, so at 390px the kitty square is
358px and Decline's 183px; the labels keep their font size because they
are HTML, not SVG text. Below 460px the spec rows stack label over value.
Below 860px the header is two rows: wordmark and GitHub, then the install
command, which wraps (`overflow-wrap: anywhere`); every two-column section
stacks.
Command blocks scroll horizontally inside their own box. The completion
list puts descriptions under their flags below 520px. Stacked grids use
`minmax(0, 1fr)` with `min-width: 0` on the items, so a long `pre` scrolls
inside its own box instead of widening the page (the first version
measured 555px at 390px).

## Known weaknesses

- The spec list is a table of numbers beside a graphic; a reviewer could
  read it as a stats block. Every value is dated and attributed in the
  same list and the caption, which is the defence.
- Only idle memory is drawn. `cat` time and four-tab memory are text
  only. A second drawing for time was rejected as a bar chart in disguise.
- The ctl section shows commands with no output, by rule, so it is the
  least visual section.
- No background terminal-text treatment was applied; it was optional and
  would compete with the drawing.
- No social card or 404 page: concept only.
- Headless checks were made only to confirm the 1440×900 fit; the parent
  takes the reviewed captures. Chrome's minimum headless window makes a
  bare 390px capture misleading, so the 390px claim rests on the CSS
  above until the iframe capture is taken.
- The Copy button relies on the Clipboard API; when denied it selects the
  command and changes its label to "Select and copy".

## Review changes (cross-review, 7 November 2026)

- Phone overflow fixed at the grid: single-column tracks were `1fr`
  (min `auto`), so unbreakable `pre` lines set the track width. Now
  `minmax(0, 1fr)`; no body `overflow: hidden`.
- Key line under the title: `0.58 s to cat 100 MB, 26 MB idle. M1 Max,
  2 November 2026.` The spec list keeps the comparisons.
- Header GitHub link shares the wordmark's row on small screens.
- Removed the lead sentence about Rust and Metal and "written in Rust"
  from the meta description: implementation, not what a user gets. The
  stack is in the README.
- Removed the CSS section-label comments; provenance stays here.
- Type sizes collapsed to 12 (chrome), 13 (terminal), 14, 16, 20, 28 and
  the title.
- Spacing moved onto the 8px scale above; terminal text everywhere is 13px/1.3 (the completion descriptions had dropped to 12px on phones and the command blocks were 14px/1.65).
