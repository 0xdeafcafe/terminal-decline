# Decline website

Alex chose Long rest on 7 November 2026, then asked for a heavier hero title
and a comparison displayed inside a terminal preview on the right. The
reviewed concepts remain in `concepts/`; the production source is `src/`.

## Changes from the chosen concept

The hero uses SF Pro at weight 800, two lines on desktop and one when it
fits on a phone. It keeps the white/spruce/mint palette, flat bands, native
scrolling and the exact still Slope geometry. The system selects light or
dark. The comparison remains in Decline's actual default dark palette in
both modes, as product evidence.

The title is 3.5 to 7rem on desktop and 2.55 to 4rem below 900px. Its line
height is .98 and tracking is -.045em. Other type and spacing retain the
chosen concept's tokens. The terminal uses Menlo, 13 to 19px with a 1.55
line height, `#16181c` background, `#d8dee9` foreground and ANSI green
`#b5bd68` for Decline. These come from `Theme::default` and the captured
cells, not the website's brand palette. The 13px phone specimen matches
the app's default font size; surrounding prose remains at least 16px.

Alex subsequently asked for more terminal chrome and installation
instructions in the header. The preview now has an HTML Mac-style title
bar labelled Terminal Decline, decorative window controls, a 12px frame
radius, a 1px `#3b3d42` border and a restrained spruce-tinted shadow. The
controls are aria-hidden and do not pretend to operate a native window.
This is an explicit exception to the original no-chrome rule. There is
no prompt, typing animation or invented benchmark run.

The header uses a native details/summary disclosure, labelled How to
install, containing the Homebrew command, macOS and hardware requirements,
and a link to the source instructions. It works without JavaScript.

The preview is selectable browser text: real terminal cells
rendered as HTML at build time. It is not a running Rust terminal emulator
and does not execute benchmarks on the visitor's machine. The recorded
measurement date is visible inside and beside it. No client JavaScript is
needed, including framework runtime code.

The subdued captured-help background remains decorative and aria-hidden.
Nothing runs or blinks on scroll. Reduced motion is respected by the CSS.
The no-texture and near-black checklist exceptions apply only to the
approved background and this actual terminal evidence, respectively.

## Preview provenance

`src/data/comparison.json` transcribes the README's 2 November table.
`comparison-terminal.py` is a display program, not a benchmark runner. It
prints those values and proportional character bars to a real Decline
session. The website caption says these are recorded results displayed in
Decline. The 30 October results remain separate in the method section,
including Ghostty's faster cat wall time and the input-latency limitation.

The 7 November capture used the installed app, no user config, a temporary
HOME and TMPDIR, no window and a 38×16 screen. Commands:

```sh
terminal-decline ctl spawn --name website-comparison --size 38x16 -- /opt/homebrew/bin/python3 site/scripts/comparison-terminal.py --hold
terminal-decline ctl wait-for website-comparison --text 'Lower is better.' --timeout 5000
terminal-decline ctl screen website-comparison --json
terminal-decline ctl screenshot website-comparison screen.png
terminal-decline ctl kill website-comparison
```

Paths were absolute when run. The transcript, raw JSON and PNG are in
`/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/tc/` for the first capture; the reproducible script was then run successfully under
`/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/dc-oc9bd7tx/`.
`terminal-preview.json` keeps the raw capture's SHA-256 and the exact text,
colours and bold attributes, grouped into adjacent identical-style runs.
Trailing empty cells are omitted. Temporary paths, process IDs and other
session metadata are not included in the website data.

To reproduce, run `python3 site/scripts/capture-terminal.py` from this
checkout on a Mac with Decline installed. It creates its own short temporary
path (Unix socket path lengths matter), records a transcript and PNG there,
and writes only the grouped JSON to this repo. It never runs a Rust build.

## Implementation and assets

Astro with `@astrojs/react`; TSX components have no client hydration
directives. Astro 7 emits an unreferenced React renderer chunk even in
this mode. `finalize-build.mjs` verifies that no page includes a script or
hydrated island, then removes unreferenced runtime files. The subsequent
verification rejects any remaining JavaScript asset. The built site has
no script tags or JavaScript assets.
Astro's [React integration](https://docs.astro.build/en/guides/integrations-guide/react/)
provides the build-time rendering.

Seven original repo PNGs are converted losslessly to WebP at build time,
with a single shared URL for repeated uses. The full original autocomplete
PNG is also retained as a source asset. Image dimensions are declared and
below-fold images load lazily. No font files, trackers or remote resources.
The full completion screenshot and its mobile CSS crop retain the chosen
concept's framing; the caption describes the crop.

The social image is generated as a 1200×630 PNG from explicit SVG typography
and the actual Slope path. It names the measurement date and hardware.
Its system font fallback can differ between macOS and Linux; no Apple
font is distributed. A distinct 404 page, canonical URL, metadata and
SVG favicon complete the concept's formerly deferred C100 items.

## Validation

`npm run check`, `npm run build`, and `npm run verify` pass. The verification
checks local assets and links, absence of client scripts/islands, CNAME,
404 metadata and agreement between README figures and the captured text.

Direct headless Chrome captures at 1440×900, 768×1024 and a real 390×844
iframe passed in both themes, with reduced motion forced. All eight
captures exited 0. No page overflow, terminal horizontal scrolling,
broken internal anchors or duplicate H1s. The terminal ends at y703 on
desktop and y663 on a phone. The header disclosure was activated in the browser at 390px and 1440px;
its contents stayed inside the viewport and its summary had a visible
focus outline. The full cask argument is kept on one line. Parent inspected desktop and phone light/dark,
and the full light page. Captures and metrics are under
`$TMPDIR/decline-site-build/review/`, outside Git.

The full-height captures are for composition; their larger viewport changes
viewport-relative sizing. No Safari, physical phone or screen-reader run.
The original 100-item cross-review is in `concepts/long-rest/SLOP-REVIEW.md`.
The new title and terminal are reviewed against its typography, genuine
output, foreground contrast, no fake chrome, motion and overflow items.

## Publication

The workflow builds on matching branch pushes and PRs, and deploys only
main. It uploads `site/dist` using `actions/upload-pages-artifact` and uses
`actions/deploy-pages`; concept files are not published. Rust CI ignores
site-only changes on pushes and PRs. A workflow change can still run Rust
CI once, as expected.

Publication needs Alex's explicit approval. Before the approved main push,
set GitHub Pages to `build_type=workflow` using `gh api` and set the custom
domain to `decline.forbes.red`. Alex adds the DNS CNAME `decline` pointing
to `0xdeafcafe.github.io`. No Pages settings, DNS or remote branch has been
changed while preparing this build.
