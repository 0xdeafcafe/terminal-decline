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

The header shows the full Homebrew command directly, with a Copy button.
Alex rejected the earlier disclosure because it hid the command. The
button uses the browser clipboard API, reports success, and selects the
command for manual copying if permission is denied. No install pill or
copy icon. Requirements and source installation are linked below.

Alex also rejected the verbose copy. Prose and headings were reduced from
about 796 words to 201, including the collapsed method notes. Feature
sections have one sentence; screenshot narration, forced asides and
repeated CTAs were removed. The measurement conditions and 30 October
loss are retained under Method and earlier results.

The preview is selectable browser text: real terminal cells
rendered as HTML at build time. It is not a running Rust terminal emulator
and does not execute benchmarks on the visitor's machine. The recorded
measurement date is visible inside it. The terminal preview needs no
JavaScript. The only browser script is the roughly 1 KB clipboard handler.

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
this mode. `finalize-build.mjs` rejects hydrated islands and any script
other than the inline clipboard handler, then removes unreferenced runtime
files. Verification rejects external JavaScript assets and checks that each
page has only the expected clipboard handler. No React runtime ships.
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
checks local assets and links, absence of framework scripts/islands, CNAME,
404 metadata and agreement between README figures and the captured text.

Direct headless Chrome captures cover 1440×900, 768×1024 and a real
390×844 iframe in both themes with reduced motion forced. The install
command and Copy control were also checked at 320px. Browser checks
intercepted clipboard writes (without touching the system clipboard) and
verified the exact command, success feedback, focus outline and the
permission-denied text-selection fallback.

Captures and metrics are under `$TMPDIR/decline-site-build/review/`.
Some runs exited 2 after writing their PNG and metrics because Chrome's
teardown watchdog expired; this is recorded in their logs. The completion
image is deliberately cropped on phones. Its crop container fits the
viewport and clips the oversized image; it is not page overflow. Capture
copies load images eagerly so headless screenshots include below-fold
images; production keeps lazy loading. No Safari or screen-reader run.

The original 100-item cross-review is in
`concepts/long-rest/SLOP-REVIEW.md`. Its score is historical, not a claim
that it prevented the later copy problem. Alex's rejection prompted this
shorter copy and the directly visible header command.

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
