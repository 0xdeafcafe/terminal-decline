# Decline website: Screen

Alex selected Screen from round two, then changed the headline to
“A peppy terminal for macOS.” The selected and cross-reviewed HTML remains
in `concepts/round-two/screen/`. The production implementation is in `src/`.
The earlier Long rest build landed at 14f1c8e; it remains deployed until
Alex approves publishing this replacement.

## Composition

A spruce opening carries the outlined wordmark, visible Homebrew command,
heavy mint title and a plain introduction to the terminal. A wide terminal
window crosses the boundary between spruce and the page ground. Its tab
strip is ordinary page navigation: Comparison, Pins, Agents and Install.
It is not a simulated interactive app. Chrome is aria-hidden where purely
decorative; the navigation remains accessible and keyboard operable.

The desktop window contains the captured comparison at left, documentation
commands at upper right and the completion transcription below them. At
860px and below it retains only the comparison. The commands already occur
in their supporting sections; completion is displayed in its own section on narrow
screens. CSS display:none ensures only one completion view is exposed.

The heading uses the native SF Pro stack at weight 800, with a responsive
44–96px size and .98 line height. The slightly smaller maximum accommodates
Alex’s added “peppy” while retaining the compact selected composition.
Terminal text is Menlo 13px/1.3 throughout. Page prose is 17px/1.6; small notes
are 15px. Native browser text selection is enabled on evidence and commands,
with user-select:none limited to chrome and controls. There is no typing,
blinking or benchmark animation. System light/dark and reduced motion work
through CSS, with no theme switch or browser framework.

The full comparison table follows, then pins, agent commands, completion
and install.
Table figures and bar proportions come directly from comparison.json;
there is no second manually typed set of values. Method conditions and
the earlier Ghostty cat win remain visible. The concept’s flat changes of
ground, dimensions, colour tokens and asymmetry are preserved. The CSS
linear-gradient is a hard boundary between two grounds, not a colour fade.

Terminal backgrounds and capture runs use the app’s real default colours.
The completion and example-command colours are the concept’s explicit web
presentation tokens; they are not claimed as a newly captured session.
The header Copy uses the existing clipboard handler, announces success and
selects the visible command for manual copying if permission is denied.

## Preview provenance

`src/data/comparison.json` transcribes the README's 2 November table.
`comparison-terminal.py` is a display program, not a benchmark runner. It
prints those values and proportional character bars to a real Decline
session. The date is printed inside the preview; its caption links to benchmark
details. The 30 October results remain separate in the method section,
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

Astro and @astrojs/react render TSX at build time. Header and the terminal
window have reusable components. Completion reads completion-preview.json;
its source screenshot hash is verified. CommandLines shares the same agent
commands between the opening and supporting section, adding only syntax
colour to documentation examples. Its DOM preserves real newlines, and flag spans stay unbroken when
commands wrap on phones.
The page has one main landmark containing all product sections, plus a
skip link, header and footer. The missing-page route uses the same header.

The clipboard handler is the only browser script. finalize-build.mjs
rejects islands and unexpected scripts, then removes Astro’s unreferenced
React renderer chunk. No React runtime or terminal emulator is shipped.
The Pages build keeps the CNAME, favicon, canonical metadata and 404 route.
The social image now uses the selected spruce/mint design and new headline.

Six pin source PNGs become lossless WebP files. Large pins are resized to
256px, sufficient for their 112px CSS display on Retina; the original 32px
and 16px examples keep their size. All pin files have shared URLs and load
lazily. No terminal images, fonts, trackers or remote resources are fetched.
The completion screenshot stays in docs/img as evidence, not a web asset.

## Validation

Astro check, TypeScript, static build and build verification pass. Browser
validation uses direct headless Chrome at 1440×900, 768×1024 and a 390×844
iframe, in both schemes with reduced motion forced, plus tall desktop and phone
captures. The lower phone completion view was also inspected separately. The captured terminal’s DOM text is compared with the source JSON;
visible completion count, terminal density, overflow, images and local
anchors are checked. The install Copy success and denied-permission fallback
are exercised with intercepted clipboard writes, never the system clipboard.
Capture copies load images eagerly; production retains lazy loading.

Scripts, images and browser metrics stay in `$TMPDIR/decline-screen-build/`.
Chrome can exit 2 at teardown after writing valid images and DOM metrics;
individual logs retain that result. Light mode was explicitly supplied by
the containing iframe after the host switched to dark; the recorded
matchMedia result confirms each theme. Safari and screen-reader testing are
not claimed. The selected concept’s independent 100-item review remains
under concepts/round-two/screen; its score is historical, not a review of
the component port or revised headline.

## Publication

The existing workflow builds the site and deploys approved main pushes via
upload-pages-artifact and deploy-pages. Site-only changes are ignored by
Rust CI. Pages is configured for workflow builds and decline.forbes.red.
Alex owns the DNS CNAME: decline to 0xdeafcafe.github.io. This Screen revision
is local pending approval to replace the currently deployed Long rest site.

## Copy revision

The opening now explains Decline in plain language rather than presenting
two measurements and their test conditions as a pitch. Benchmark labels
explain that the cat test displays a text file. The method, measured values
and earlier Ghostty win remain available. Captured terminal text is unchanged.
Pins and agent control describe a use before the command example. Completion
is a separate section, and installation explains what to open afterwards.
The social card and search description use the same product language.

Copy revision validation: the static build and existing asset/data checks
passed. Chrome captures at 1440, 768 and 390 px in both colour schemes
reported no horizontal overflow, broken images or unresolved page anchors.
Desktop light and the complete phone dark page were inspected visually.
All captures stay in the session's temporary review folder.
