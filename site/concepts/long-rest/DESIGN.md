# Long rest: concept notes

Concept for decline.forbes.red, 7 November 2026, from checkout a4086db.
One file, `index.html`, inline CSS, embedded assets, no JavaScript, no
remote requests. Originally written by Claude Fable 5.1 (`claude-fable-5-1`) against `site/BRIEF.md`,
`site/ANTI-SLOP.md`, `site/RESEARCH.md` and `docs/brand/README.md`.
Cross-reviewed by Claude Sonnet 5.5 against the anti-slop checklist; see `SLOP-REVIEW.md`.

## The idea

A full-width scroll story in flat bands of white, spruce and mint, with
the Slope as the structural device. It is left-aligned: the first view
holds the headline "26 MB at idle.", one short lede, a line of
conditions and the two measurements that matter, idle memory and a
`cat` of 100 MB, as plain bars for Decline, Ghostty and kitty. The whole
Slope sits at bottom right at 220 px, its level rest on the top edge of
the band below. In light mode the mark is spruce, so the rest fuses with
the spruce band under it (hence the name); in dark mode it is mint.

Order, after Alex's steer that speed and footprint are the point: the
opening, the rest of the measurements and the method (spruce band),
pins (mint band), completion with the real screenshot (white), the
agents section (spruce) and install (white). Nothing is a card. No
eyebrows, no slogan, no fake chrome.

## Background (Alex's decision: both, applied 7 November 2026)

Real captured terminal text plus a very few box-drawing corners. Source
and tokens match `site/concepts/backgrounds/DESIGN.md`:

- Text: lines from `terminal-decline ctl --help` of the installed
  build, captured 7 November 2026 (SHA-256 of the capture
  `09c36b7d...874a53`). Two excerpts: the `screen`/`wait-for`/`resize`/
  `screenshot` lines in the opening's top right, and the `socket`/
  `hello`/`spawn` lines under the measurements heading. The `send`
  excerpt is not used, because its `\n is Enter` help line is wrong in
  practice (Opus's capture on a4086db).
- One `┘` in Menlo at the foot of the install band, 200 px at 8%.
- Menlo 14/1.6 for text at 10% opacity, in the page ink (white on the
  spruce band). No colour of its own.
- Decorative: `aria-hidden`, `pointer-events:none`, selection off,
  `overflow:hidden` so long lines clip at the margin. In flow beside or
  below content, so nothing sits under meaningful text. Not animated.
- Hidden below 900 px.
- Explicit exception to the checklist's texture and contrast rules,
  authorised by Alex: subtle real terminal and glyph texture, below 4.5:1
  because it is not content.

## Tokens

All colours sRGB, all from `docs/brand/README.md` unless noted.

| Token | Value | Why |
| --- | --- | --- |
| spruce | `#092D2B` | Brand ink. Light-mode text, the mark, the completion and agent bands. Dark-mode ground. |
| mint | `#B9E5CC` | Brand mint. The pins band in both modes; dark-mode mark, secondary text, and the lead bars on spruce and dark grounds. |
| white | `#FFFFFF` | Light ground. Chosen over brand paper `#F3F3EB` as the brief asks, to stay away from the cream-and-serif skin. |
| deep | `#051A17` | Dark-mode band colour, from the brand's dark icon body bottom. Keeps the dark page in two spruces rather than spruce plus black. |
| moss | `#245950` | Brand icon body top. Secondary text on white (7.9:1) and the dark-mode rule colour. |
| code-bg light | `#EEF4F0` | Mint laid at about 25% over white. Code blocks on the white ground. |
| code-bg on spruce | `#143B34` | Brand dark icon body top. Code blocks on spruce bands. |
| code-bg on mint | `#D8F0E3` | Mint lightened one step. Code blocks on the mint band. |
| rule | `#D5E2DB` light, `#245950` dark | One hairline under each chart group title, the only rule on the page. |
| bar, comparison | `#A3B8AF` on white, `#3A6A60` on spruce | Spruce-tinted greys for the Ghostty and kitty bars, so the chart has one lead colour and one neutral. On the spruce measurement band the lead bar is mint in both modes. |
| bar, Decline | spruce light, mint dark | The one highlighted series. |
| dark text | `#F2F7F4` | Near-white with a mint cast, not pure white, on spruce. |

Contrast: spruce on white 16.9:1, mint on spruce 10.4:1, spruce on mint
10.4:1, moss on white 7.9:1, mint on deep about 12:1. All text is above
4.5:1. Muted text uses mint on dark bands and moss on light; nothing is
grey-on-grey.

The chart colours were run through the dataviz skill's validator. Its
lightness and chroma checks fail, as expected: they are for categorical
palettes that identify series by hue, and this chart does not. Every bar
carries its name and value in text, so identity never rests on colour.
CVD separation passed (ΔE 57 light, 45 dark). The neutral bars are below
3:1 against the ground; the relief the validator asks for is the direct
labels, which are on every bar.

### Type

| Role | Face | Size / line height | Weight |
| --- | --- | --- | --- |
| Body | SF Pro Text via `-apple-system` | 18 px / 1.55 (17 px under 520 px) | 400 |
| Lead | SF Pro | clamp 1.15 to 1.333 rem / 1.45 | 400 |
| Small | SF Pro | 0.889 rem / 1.5 | 400 |
| H1 | SF Pro Display | clamp 2.6 to 5.2 rem / 1.02, tracking -0.03em | 500 |
| H2 | SF Pro Display | clamp 2 to 3.6 rem / 1.05, tracking -0.015em | 500 |
| H3, chart group | SF Pro | 1.333 rem / 1.3 | 500 |
| Commands | SF Mono, then ui-monospace, Menlo | 0.889 rem / 1.6 | 400 |
| Background text | Menlo | 14 px / 1.6, 10% opacity | 400 |

Six sizes, two weights. SF Pro is the brand's own choice (native Mac
typography, `docs/brand/README.md`); it is not a fallback. The body is
18 px rather than the brand's 16/24 product prose because a wide graphic
page reads better one step up; the brand's 16 px is for the app's own
prose. No Apple font is distributed: the wordmark is the outlined SVG
and the headings come from the visitor's system. On a non-Mac the stack
falls through `system-ui` to Helvetica, which is documented here as the
accepted degradation rather than loading a webfont that is not the
brand's.

Measure: paragraphs are capped at 34em (about 65 characters at 18 px).

### Shape and space

- Corner radius: 0 everywhere on the page. The only rounded things are
  the artwork (the Slope's round caps, the pins' own corners) and the
  4 px data end of each chart bar, which is the dataviz mark spec. A
  flat page lets the artwork be the only soft thing on it.
- No shadows, no borders, no glass. One hairline rule under chart group
  titles.
- Spacing: an 8 pt scale (0.5, 1, 1.5, 2, 2.5 rem), band padding
  `clamp(56px, 9vw, 136px)`, horizontal gutter `clamp(20px, 5vw, 96px)`,
  page max 1560 px.
- Links: underlined, 1.5 px, offset 0.18em, coloured by the band's
  secondary colour and going solid on hover. The top nav and the
  opening links are weight 500, no boxes, no arrows.
- Focus: 3 px solid outline in the band's ink, offset 4 px.
- Motion: none. The only transition is the underline colour on hover,
  150 ms by the browser default, and `prefers-reduced-motion` disables
  even that. Scrolling is native.

## Assets and provenance

| On the page | Source | Treatment |
| --- | --- | --- |
| Wordmark, top and footer | `docs/brand/wordmark.svg` | The seven outlined glyph paths and the mark path copied verbatim, the paper rectangle dropped, fills changed to `currentColor` so the lockup is spruce on white and near-white on spruce (the brand's inverse lockup). ViewBox tightened to `50 80 600 120` around the lockup's own bounds (mark x 59 to 210, text x 254 to 640). |
| The Slope in the opening | `docs/brand/icon.svg` | The path `M294 400 L484 610 Q497 624 516 624 H738`, stroke 92, round caps and joins, exactly as specified. ViewBox is the stroke's bounds, `248 354 536 316`, so the SVG's bottom edge is the rest segment's bottom edge. No body, depth, light or rim: the brief's flat treatment. |
| Favicon | same geometry | Flat spruce rounded square with the mint Slope, as a data URI. |
| Completion screenshot | `docs/img/screenshot-autocomplete.png`, 2058×1574 | Embedded as base64, unaltered. Shown whole on screens wider than 720 px. On narrower screens a CSS crop shows the region x 110 to 1510, y 745 to 1165 (the prompt line and the completion list) in a 1400:420 box; the full image stays in the DOM for the alt text. The window's own macOS chrome is in the screenshot and nothing is drawn around it. |
| Pins | `docs/brand/samples-generated/rush.png`, `htop.png`, `claude.png`, `logs.png` (512 px), `rush-32.png`, `rush-16.png` | Embedded unaltered. The 512 px renders are displayed at up to 336 px (Rush) and 128 px (the row), so they are never upscaled. The 32 and 16 px renders are shown at 32 and 16 CSS px, which on a Retina screen means they are drawn at twice their pixel size; the caption says they are the Dock-size renders. Labelled as artwork samples, not a Dock. |
| Benchmark bars | README Benchmarks table, 2 November 2026 | Four groups, values typed from the table and checked against `docs/img/bench-light.svg` (0.58 / 0.66 / 1.05 s, 0.66 / 2.04 / 1.63 s, 47 / 87 / 104 MB) plus idle memory 26 / 60 / 99 MB. Bar widths are value over the group's maximum. Built in HTML so they follow the page's colours in both modes. |

The page is generated from a template by base64-encoding those files;
the template and the two-line Python that does it live in the author's
`$TMPDIR` and are not part of the deliverable. Re-embedding is a matter
of replacing the eight `data:` URIs.

Size: about 2.8 MB, almost all of it the screenshot (675 KB PNG) and
four 512 px pins (about 190 KB each). For the Astro build these become
files, and the pin row would use 256 px renders.

## Copy and claims

Every claim traces to the brief's ledger or the README at a4086db:

- Completion: zsh matches, descriptions from man pages or `--help`,
  grey text and Right, Ctrl+Space, `completion_as_you_type` off by
  default (README Completion; `docs/design/completion.md` §1). The bash
  and fish line is from completion.md's goal section.
- Pins: `decline pin rush --dock`, `~/Applications/rush.app`, icon
  lookup order, `--icon`, `--list`, `--edit`, `--remove`, pins load the
  installed app (README, Pin a program to the Dock). rush's install line
  is the README's. The ten hues and the name hash are
  `docs/brand/README.md`.
- Benchmarks: the 2 November table and conditions, the idle CPU and
  wakeups, and the 30 October run kept separate with its own numbers,
  including the one Decline lost (README Benchmarks;
  `docs/comparison-2026-10-30.md`).
- Agents: `ctl` commands as README examples, JSON output and exit
  codes, headless server, socket, `agent_api = false`, 30-minute idle
  kill (README, Driving it from agents and tests). The page says there
  is no assistant in the terminal, which the ledger requires.
- Install: the Homebrew cask line, Apple Silicon, macOS 14, the two
  command names, the source routes, MIT, and that the updater has
  nothing signed to offer yet (README Install and Features).

Not claimed: zero CPU, any latency win, user counts, automatic updates,
or anything about shells the terminal does not do. "Less overhead. More
terminal." does not appear. No em dashes. British spelling throughout.

## Small screens

- Below 900 px every two-column grid collapses to one column; the Slope
  moves under the headline, right-aligned at up to 60vw, still resting
  on the band below.
- Below 720 px the screenshot switches to the documented crop so the
  completion list stays legible at 390 px.
- Below 520 px the body drops to 17 px, the nav gap tightens, and the
  big Rush pin stacks above its 32 and 16 px renders.
- The agent command block and the install line scroll horizontally
  inside their `pre` rather than wrapping a command, so no line is
  broken into something that would not run.
- Chart rows stay three-column (name, bar, value) at every width; the
  name column is 5.5em and the value column 4.5em.

## Anti-slop notes for the reviewer

Deliberate choices that could look like tells, and why they are not:

- The pins' gradients, rims and depth are the brand's own artwork,
  rendered by term-icon. Nothing on the page itself has a gradient or
  shadow.
- The screenshot has macOS traffic lights because it is a real window.
  No chrome is drawn.
- The chart is a dated measurement with its conditions and script
  linked, and includes a run Decline lost. It is not a stats row.
- Lowercase `decline` appears only in the wordmark. Headings are
  sentence case.
- The command blocks are labelled as examples from the README, not
  captured output. There is no prompt character and no invented output.
- The scanner pass (`rounded-2xl`, `backdrop-blur`, `bg-clip-text`,
  `indigo-`, `violet-`, `shadow-lg`, `data-aos`, `whileHover`, `lucide`,
  em dash) returns nothing.

## Known limits of the concept

- No social card, 404 page or per-page titles: this is one page. These
  are deferred to the Astro build.
- The chart has no hover tooltip or table view because the page ships
  no JavaScript; the values are already in text on each row.
- The 32 and 16 px pins are drawn at twice their size on Retina. The
  honest alternative is `image-rendering: pixelated`, which would make
  the point about Dock sizes better but look worse; left as a decision
  for Alex.
- Screenshots of the concept at 1440, 768 and 390 in both schemes are
  taken centrally by `$TMPDIR/decline-site-review/capture.py`, not here.


## Changes after review (7 November 2026)

- Content hierarchy follows Alex's correction: speed and low footprint first. The H1 is the measured result, "26 MB at idle.", with 0.58 s for `cat` in the lede and both measurements as bars in the first view. At 390x844 the headline, lede, conditions and both charts fit before the links and the Slope.
- Below 900 px the opening's children are reordered by CSS (`display:contents` on the text and chart wrappers): charts, then the Install and source links, then the Slope on the band.
- The agents example is `--text "make test" --key enter`; the README's `\n` form was verified by Opus not to press Enter. README untouched.
- The Homebrew line is sized to sit near one line on desktop and wraps on mobile.
- Chart rows: the lead row's class is `best` (it had been inheriting the lede's styles). On the spruce band sub-labels use mint and the lead bar mint.
- Public copy carries no commit hashes, term-icon or file paths. Provenance kept here: Decline measured at 7849ad4, with `cat` and idle CPU at 82c455f, script `scripts/compare/run.py`; screenshot `docs/img/screenshot-autocomplete.png`; pins `docs/brand/samples-generated`.
- Input latency: Decline's in-app latency is measured in the repo's DESIGN.md, but no head-to-head comparison exists, so the page says only that none of the runs shown compared it.
- No bash or fish completion claim; the page says the matches come from zsh.
- Warp's 30 October time carries its two-window caveat.

## Known limits

- No 404 page or social card (anti-slop C100 stays a fail): deferred to the Astro build.
- The page is about 2.8 MB, mostly the unaltered screenshot and four 512 px pins.
- Screenshots are taken centrally by `capture.py`; reviewer notes and links are in `SLOP-REVIEW.md`.
