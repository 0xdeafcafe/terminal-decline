# Dockside: design notes

Concept for decline.forbes.red, from checkout a4086db. One self-contained
`index.html`: inline CSS, no JavaScript, no remote requests, about 5.3 MB
because the PNGs are embedded byte for byte.

Written by Claude Opus 5.5 as a pin-led spread; revised by Claude Fable 5.1
in the cross-review of 7 November 2026 after Alex changed the priority to
measured speed and footprint, then chose the "both" background direction.
The original is preserved as the immutable before snapshot in the review
folder. `SLOP-REVIEW.md` beside this file has the item-by-item audit.

## Idea

A spacious Mac product spread, white and native type, that opens on one
measured fact and the table that backs it. The first viewport is the whole
case: "26 MB at idle." on the left, the dated comparison table on the
right, and below the content, bleeding off both edges, faint captured
output from `terminal-decline ctl --help` with two still box-drawing
corners. The texture is the terminal showing through the page, never under
a word of the copy.

Everything else is supporting. The everyday terminal, Tab completion, pins
and agents follow in that order, and the page closes on install. All three
concepts now lead with the measured speed and footprint; Dockside differs
from Fieldnotes (a narrow working document) and Long rest (full-width
graphic bands, the Slope as geometry) in treatment, not in what comes
first. Here the Slope appears only in the wordmark and inside the pins.

## Hierarchy and composition

No two neighbouring sections share a structure.

| Section | Composition |
| --- | --- |
| Opening | 6/6. One-line H1, lede and links left; the measurement table and its one-line note right, top-aligned. 17 rem of air beneath, which is where the background text lives |
| How it was measured | 5/7. A sentence-length heading at the sub size, three paragraphs of conditions, the CPU and idle figures, and the 30 October run Decline lost |
| The everyday terminal | Whole settings window left, its edge on the page margin, short prose right, vertically centred |
| Tab tells you | Heading alone, a wide crop running off the right edge, prose hung under it at the right |
| Any command can be its own Mac app | 7/5. Heading, pitch, command and the four pin commands left; the sample Rush pin right, starting above the heading's top line |
| When there's no icon | 4/8. Prose and the 128/32/16 px ladder with a 6× loupe left, bottom-aligned to a 4×3 specimen of named pins |
| Agents | The page turns over: full-bleed inverse ground, heading across, command list 7/5 prose |
| Install | Display-size heading, the Homebrew line large, source routes in a 5-column side column |

The hand-set moments a template would not make: the captured help text
bleeding off both page edges, the crop running off the right edge, and
the 16 px pin enlarged with `image-rendering: pixelated` beside its real
size.

## Tokens

Colour. One dominant (spruce), one accent (mint), one neutral (white or
paper). Pin hues are artwork, not page palette.

| Token | Light | Dark | Why | Contrast |
| --- | --- | --- | --- | --- |
| `--ground` | `#FFFFFF` | `#092D2B` | Brief: white on the web, real spruce in dark | |
| `--ink` | `#092D2B` | `#F3F3EB` | Brand ink and inverse paper | 14.77, 13.24 |
| `--quiet` | `#47625E` | `#B4CFC3` | Spruce-tinted, not default grey | 6.61, 8.90 |
| `--rule` | `#C9D9D1` | `#2C5550` | Tinted hairlines between sections and table rows only | decorative |
| `--mark` | `#245950` | `#B9E5CC` | Link underline, focus ring, the corner glyphs. Light value is the brand body top | |
| `--swap-ground` | `#092D2B` | `#B9E5CC` | Agents section inverts against the theme | |
| `--swap-ink` | `#F3F3EB` | `#092D2B` | | 13.24, 10.65 |
| `--swap-quiet` | `#B4CFC3` | `#2F5450` | | 8.90, 6.05 |
| `--ghost` | .09 | .09 | Opacity of the captured text, from the combined background study | decorative |
| `--shape` | .065 | .065 | Opacity of the two corners, from the same study | decorative |
| crop backing | `#0E1117` | same | Behind the screenshot crop while it decodes. Close to the window colour, not a sampled value | |

The background text and corners are decorative and `aria-hidden`, with
pointer events and selection off, so the body contrast floor does not
apply to them. The mobile strip runs at .12 so two lines still register
at 10 px.

Type. Six sizes, weights 400, 500 and 600.

| Token | Size | Use |
| --- | --- | --- |
| `--s-note` | 14 px | Captions, conditions, masthead, table headers |
| `--s-body` | 17 px / 1.6 | Prose, the Mac reading size. Brand says 16/24; bumped by 1 px for a page rather than an app |
| `--s-lede` | 21 px | Opening paragraph |
| `--s-sub` | 20 to 28 px | Large commands, table figures, the two sentence-length headings |
| `--s-head` | 40 to 72 px | Section headings |
| `--s-big` | 48 to 112 px | The H1 and the install heading |

Faces: `-apple-system` first (SF Pro on a Mac, resolved by the OS and never
shipped), then Helvetica Neue and Helvetica elsewhere. Commands are
`ui-monospace` then SF Mono then Menlo, at .94em of their context. The
background text and corners are Menlo first, the terminal's own default
font, 14 px on a 2.0 line. SF Pro is the brand's deliberate choice, so the
anti-slop item on system stacks does not apply. Display tracking is
-0.028em, -0.035em at `--s-big`.

Layout. `--edge` is 20 to 64 px, spacing on an 8 pt base, section padding
varies on purpose. No container: text columns come from the grid and prose
stops at 62ch. Command lines wrap only at spaces: each flag and the
repository name sit in a `.w` span with `white-space: nowrap`, so
`terminal-decline` never breaks at its hyphen.

Radius. `--radius` 12 px, used once, on the left corners of the crop. Pins
and windows carry their own.

Shadow. None added. The settings window's shadow is in the screenshot.

Motion. One kind: link underlines thicken in 160 ms, removed by
`prefers-reduced-motion`. The original's fade-and-settle on the hero image
was removed in review as a reflex reveal. No scroll effects, nothing
blinks or types.

## Background texture

Authorised by Alex on 7 November 2026 as a brand exception after the
isolated studies in `site/concepts/backgrounds/`; "both" meant the
combined study. It is applied only to the opening section.

- Two `pre` blocks hold exact slices of the captured `ctl --help` output
  recorded in `site/concepts/backgrounds/DESIGN.md` (lines 10 to 24 and 29
  to 44 of the capture, SHA-256 `09c36b7d…`). They are copied verbatim from
  the combined study's `index.html`, HTML entities included. Nothing was
  typed in by hand, and no prompt or output was invented.
- Both `pre`s have `overflow: hidden` and fixed widths, so long lines clip
  to the margin instead of widening the page. The left one starts 300 px
  off the left edge, the right one ends 260 px off the right edge, and
  both sit in the 17 rem band under the opening's content, so no line is
  under copy.
- The corners are the real Unicode characters U+250C and U+2518 in Menlo
  at 180 px and 280 px, placed at bottom left and bottom right of the
  band, in `--mark`. Two glyphs, not a frame, not a grid.
- Up to 1100 px the opening is one column; the band shrinks to 12 rem,
  the `pre`s to 7 rem tall, the corners to 140 and 180 px.
- Up to 560 px the right `pre` is hidden, the left one becomes a 26 px
  strip of 10 px text above the H1, and the corners drop to 80 and 100
  px. The strip sits in the gap between the masthead and the heading.

## Evidence and provenance

Every image is the original file base64-encoded without recompression. The
first 12 hex characters of each SHA-256 are below, so a reviewer can decode
and compare.

| Asset | SHA-256 | Use |
| --- | --- | --- |
| docs/brand/samples-generated/rush.png | 1f6f84099479 | Pin section, 128 px ladder, specimen |
| rush-32.png | cc5cf82f5a7c | Ladder |
| rush-16.png | ab196ea08044 | Ladder, 6× loupe |
| claude.png | 6309933e0b5c | Specimen |
| htop.png | 9ce57c8cee2e | Specimen |
| lazygit.png | 2f8621e91877 | Specimen |
| btop.png | 4aecd2eec8a6 | Specimen |
| nvim.png | 6962730942a3 | Specimen |
| ssh-prod.png | 3382a698e2ec | Specimen |
| yazi.png | 1b468672bf69 | Specimen |
| logs.png | a73568a25201 | Specimen |
| vim.png | 194a4b1b6fab | Specimen |
| kube.png | f3473ab413db | Specimen |
| docker.png | 83b483d66fb4 | Specimen |
| docs/img/screenshot-autocomplete.png | eb698dc994d6 | CSS crop, Tab section |
| docs/img/screenshot-settings.png | 8e5f3e79ba71 | Whole, unaltered |
| docs/brand/wordmark.svg | 246a0d8cfea6 | Masthead, see below |
| docs/brand/icon.svg | f0725cb620e7 | Favicon, as a data URI |

Wordmark: paths are the originals. The paper `rect` and title are removed,
the viewBox is tightened, and fills use `--mark` and `--ink`, which matches
the brand's inverse lockup in dark.

Screenshot crop: the file is whole in the page. CSS windows it to the prompt
and flag list, about x 123 and y 748 in source pixels, 21% of the image width
tall. The reason is legibility, and it also leaves out a development header
reading "149 MB" that is not a measurement and would sit badly by the 26 MB
claim. The caption says it is cropped and links the whole window.

Pins are labelled as sample pins drawn by Decline. The caption on the big
Rush pin says it is what a pin gets when the program ships no icon, and
that a program which ships one keeps it. Nothing calls one the installed
rush icon.

Commands. Nothing on the page was run. All are from the README or
DESIGN.md, and each block says it is an example. Agent commands use
`decline` for `terminal-decline`, which the page states. `screen top
--json` joins the README's `screen top` with the flag from its comment.
No prompts, no output. The README's `send sh --text "make test\n"` line is
not on this page; Opus verified on the installed build that it sends a
literal backslash-n rather than Enter, so any future shell example must
use `--text "make test" --key enter`.

## Claims and their sources

| Claim on the page | Source |
| --- | --- |
| 26 MB idle, 47 MB four tabs, 0.58 s cat; Ghostty and kitty rows; conditions | README, Benchmarks; values checked against its table and docs/img/bench-light.svg |
| CPU with WindowServer 0.66 / 2.04 / 1.63 s; idle 0.02% and 0.5 wakeups | README, Benchmarks |
| 30 October: 0.53 s against 0.61 s, 31.5 MB, Warp 3.6 s and Terminal.app 4.0 s, 4 runs, latency not measured | README; docs/comparison-2026-10-30.md; BRIEF ledger |
| Rust, Metal, Core Text, native tabs, draggable splits, config.toml, `decline settings` from any terminal | README, Features |
| Extras off until wanted; no account, no assistant | README |
| Completion: zsh matches, man page or `--help`; as-you-type opt-in | README, Completion; BRIEF ledger |
| Own Dock icon, windows, settings; opens straight into it; `~/Applications/rush.app` | README, Pin a program to the Dock |
| Icon order: program's own, `--icon`, generated | README, same section |
| Ten colours from the name; shared colours | docs/brand/README.md; samples README |
| 32 and 16 px drawn at size | samples README |
| Pin does not change after creation; permissions kept across upgrades; loads the installed app | DESIGN.md, Dock pins; docs/design/pins.md |
| ctl verbs, JSON line, exit codes, `--window`, 30 minute idle kill | README, Driving it from agents and tests |
| Install routes, requirements, MIT, updater has nothing to offer yet | README, Install and Features |
| Background text is real `ctl --help` output | site/concepts/backgrounds/DESIGN.md |

The two benchmark runs stay separate: different paragraph, dated, and never
in one table.

## Small screens

- Up to 1100 px the opening is one column: heading, lede, links, then the
  table. All four columns stay visible.
- Up to 900 px the other spreads become one column. The sample pin stays
  beside its heading at 176 px.
- Up to 560 px the nav is dropped (install is linked in the opening and
  the footer), the pin takes its own row at 56% right-aligned, the
  specimen is 3 across, and the opening table drops to 16 px with tighter
  cell padding so all four columns fit in 390 px without scrolling.
- The crop keeps a legible 1040 px image scale and scrolls sideways
  natively in a focusable region.
- The install command drops to the body size at 560 px so the cask name
  fits on one line.

Checked by eye in headless Chrome at 1440×900, 768×1024 and a 390 px
iframe, light and dark, plus 1440×8600 and 500×7000 full-page captures in
the reviewer's `TMPDIR`. The central review captures are listed in
`SLOP-REVIEW.md`.

## Exceptions and known limits

- Pin gradients, rounded icons and the screenshots' traffic lights are real
  artefacts, not page decoration.
- The background texture is a brand exception Alex authorised, applied
  once, from captured output, under no copy.
- `rush.png` is embedded three times and the page weighs 5.3 MB. Fine for a
  concept; the Astro build should use one optimised file each.
- A 512 px PNG at up to 420 CSS px is under 2× on Retina, so the sample pin
  may look slightly soft. A 1024 px `term-icon` render would fix it, which
  needs a build, so it is left for the implementation.
- The 32 and 16 px files are shown at CSS size, so Retina upscales them.
  The loupe is there partly for this reason.
- `#0E1117` is an eyeballed backing, normally fully covered by the image.
- No social card or 404: deferred until a concept is chosen.
- The build template lives in the reviewer's `TMPDIR`; edits are made to
  `index.html` directly.
