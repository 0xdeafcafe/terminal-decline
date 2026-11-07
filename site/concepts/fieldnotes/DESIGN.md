# Fieldnotes: design record

Concept: a measured-results working document for decline. It leads with speed
and low footprint, with the test conditions in the right margin beside the
table they qualify. `decline ctl` comes next as the main supporting feature,
then completion, pins and install. Original author: claude-sonnet-5-5. Reworked
in cross-review by claude-opus-5-5 after Alex put speed and footprint first
(see SLOP-REVIEW.md).

`index.html` is self-contained: inline CSS, no JS, no remote requests, 1.6 MB
with images embedded as base64.

## Hierarchy

1. **Opening.** H1 "26 MB at idle." The lede says what decline is and gives
   0.58 s for a `cat` of 100 MB. Then the 2 November table, with conditions,
   the input latency limit, the 30 October result and install links in the
   margin. All of the table is in the first viewport at 1440×900, 768×1024 and,
   for the memory and `cat` rows and more, 390×844.
2. **ctl.** Keeps the original H1 as an H2. Real `ctl screenshot` PNG, then
   cells, finished commands, replacing `sleep`, and `--window`.
3. **The everyday terminal,** with a cropped real screenshot.
4. **Pins,** with the brand reference sheet.
5. **Install,** source links, the Slope at rest, a one-line footer.

Sections differ by what's in the margin: conditions, commands, captured JSON or
plain settings. Three figures break the two columns.

## Tokens

| Token | Light | Dark | Provenance and reason |
| --- | --- | --- | --- |
| ground | `#FFFFFF` | `#092D2B` | BRIEF: white for light, real spruce for dark, no glow. |
| ink | `#092D2B` | `#EEF5F1` | Brand spruce. Dark ink is a white tinted towards spruce, chosen, not brand paper. |
| muted | `#3F5C58` | `#A8C5BB` | Spruce and mint tints. About 7.3:1 and 8:1. |
| link | `#245950` | `#B9E5CC` | Icon body top in light, brand mint in dark. |
| rule | `#C9DDD4` | `#2B514C` | Table rows only. |
| slope | `#092D2B` | `#B9E5CC` | Brand inverse lockup. |
| mint | `#B9E5CC` | same | Text selection ground, with spruce text. |
| `--trace` | .09 | .1 | Opacity of captured text. Study value .09, dark nudged to stay visible on spruce. |
| `--corner` | .07 | .08 | Opacity of box-drawing corners, close to the study's .065. |
| crop backing | `#0E1117` | same | Behind the screenshot crop while it decodes, close to the window colour, not sampled. |

Type: `-apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui` for prose
and headings, the brand's deliberate native Mac choice, with no Apple font
shipped. Commands use `ui-monospace, "SF Mono", SFMono-Regular, Menlo`.
Background text and corners use `Menlo, monospace`. Weights 400, 500, 600.

Scale: 16 (notes, commands, captions, table), 18 body, 22 lede (20 under 30rem),
30 H2 (26 under 30rem), H1 clamp(48 to 68). Inline code is .92em in prose and
1em in notes, so nothing meaningful is under 16 px. Only the aria-hidden
background is smaller (14 px, 12 px on the strip). Body line height 1.6. The
brand's 14/21 for commands is raised to 16 for reading.

Space: 8 pt scale. Prose 36rem, margin 32rem, gap 4rem, reduced to 34, 28 and 3
between 68rem and 87.5rem so the right band stays open. Gutter clamp(20px, 6vw,
104px). Radius 3px, on focus rings, the skip link and the margin rule. No
shadows, no gradients outside real artwork. Motion is a 150 ms `ease` link
colour change inside `prefers-reduced-motion: no-preference`.

## The margin

Each margin block starts with a short rounded rule from the Slope's rest
segment, 3rem by 6px. Commands are plain mono with a hanging indent, and flags
don't break across lines. Captured output is labelled "Captured" in the note
above it and set in the muted colour, so it can't be read as a command or an
example. A command with no such note is an example.

## Background (authorised by Alex, the "both" choice)

Applied from `site/concepts/backgrounds/combined`, whose DESIGN.md has the
source: real `ctl --help` output captured on 7 November 2026, SHA-256
`09c36b7d5fbef3c7…`. Line slices are reused exactly.

- Two `pre` blocks, one beside the opening and one beside the ctl section, in
  the empty band to the right of the document. `overflow: hidden` on the `pre`
  and its wrapper clips long lines at the viewport edge.
- Two Menlo box-drawing corners: `┌` above the first block and `┘` at the page
  end, level with the Slope. Still, not repeated, not a frame.
- `aria-hidden`, `pointer-events: none`, unselectable, `z-index: -1`.

By width:

| Width | Treatment |
| --- | --- |
| Over 75rem | Band, both text blocks, both corners |
| 68 to 75rem | No band, `┘` only |
| 42 to 68rem | Opening block and `┌` beside the single column; the second block is hidden because figures run full width there; `┘` |
| Under 42rem | One line of help text in the gap above the H1, and a smaller `┘` |

A text-rectangle check found no background over any text or image at 1600,
1440, 1280, 1150, 768 and 390 px.

## Assets and provenance

| Asset | Source | Treatment |
| --- | --- | --- |
| Wordmark | `docs/brand/wordmark.svg` paths | Paper rectangle dropped, recoloured with `--ink` and `--slope`, viewBox cropped. |
| Favicon | `docs/brand/icon.svg` | Unchanged data URI. |
| Slope at rest | Brand path, width 92, round caps | Inline SVG, still. |
| `ctl` PNG | Captured, below, SHA-256 `8e1c5f8d1b70bf10` | Unaltered bytes, 1624×954. |
| Completion screenshot | `docs/img/screenshot-autocomplete.png` | Unaltered bytes, CSS crop to x 113 to 1564, y 748 to 1170 in source pixels. The crop leaves out the header reading "149 MB", a development session figure that would contradict the headline if read as a measurement. Caption says it's cropped and links the whole window. |
| Pins sheet | `docs/brand/pins.png` | Unaltered, captioned as artwork. |

### Captured ctl session

Run 7 November 2026 with the installed app, 0.0.1-dev+a4086db, through
`env -i` with a temporary `HOME` and `TMPDIR`, so it had its own headless
server, no windows and no user config. `HOME` held an empty `.zshrc`.

```text
terminal-decline ctl spawn --name man --size 100x30 -- man ls
terminal-decline ctl wait-for man --text SYNOPSIS --timeout 5000
terminal-decline ctl screen man --json        # 144,704 bytes
terminal-decline ctl screenshot man $T/man.png
terminal-decline ctl kill man
terminal-decline ctl spawn --name sh --shell
terminal-decline ctl send sh --text 'sleep 2; ls /nonexistent' --key enter
terminal-decline ctl wait-for sh --command --timeout 10000
terminal-decline ctl kill sh
```

Every command exited 0. The page writes `decline` for `terminal-decline`,
which the cask installs under both names, and `man.png` for the temporary path.
The `wait-for --command` reply on the page is whole, and three pieces of the
`screen --json` reply are quoted exactly.

Not run: `--window`, `--idle`, `--exit`, the `top` and `vim` sessions, every
`pin`, `settings`, `import` and install command. Those are examples from the
README, or for `--idle` and `--exit` from `ctl --help`.

README problem found on the way, not fixed here: in zsh, `--text "make test\n"`
as printed typed a literal backslash-n and pressed no Enter, so the
`wait-for --command` that follows timed out. A real newline, or `--key enter`,
works, and the page uses `--key enter`.

## Claims and sources

Measurements: README Benchmarks, 2 November 2026, checked against its table,
and `docs/comparison-2026-10-30.md` for the separately stated 30 October
result. "These runs did not compare input latency" is scoped to these runs on
purpose. ctl: README, `docs/design/agent-api.md`, `ctl --help` and the capture
above. Completion, tabs, opt-in extras, pins, install and the updater limit:
README, BRIEF ledger and the brand README.

Not claimed: fastest, a win on latency, battery, user counts, built-in AI,
identical completion in every shell.

## Known limits

- No 404 page or social card: deferred until a concept is chosen.
- At 390 px the `ctl` PNG and pin sheet are small enough that fine text is hard
  to read. The completion crop scrolls sideways there instead.
- The crop region is a tab stop at every width, though it only scrolls on small
  screens.
- Off the Mac, type falls back to the platform UI font and background text to
  the default monospace.
- Between 68rem and 75rem there's no room for margin text.
- The screenshot is embedded in full, 1.6 MB; a real build would serve it as a
  file.
- The build script lived in `TMPDIR`. Later edits go to `index.html` directly.
