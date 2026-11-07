# Decline website brief

Prepared 7 November 2026 from checkout a4086db. Website destination: decline.forbes.red. This phase produces concepts for Alex to choose, not a deployed website.

## The product

Terminal Decline is a native macOS terminal written in Rust, with Metal drawing and Core Text fonts. Apple Silicon releases, macOS 14 Sonoma or later, MIT licence. It uses native Mac tabs and split panes. Speed and low footprint are the main reason to try it. Alex confirmed this after reviewing the first screenshots on 7 November.

Lead with measured speed and low footprint: 26 MB idle physical footprint and 0.58 s to `cat` 100 MB in the 2 November comparison. Put the dated hardware/test conditions with the evidence, and avoid an unqualified fastest claim. The opening must fit the viewport without clipped headline or artwork. Completion is supporting detail, not the headline.

Then demonstrate the useful extras. `decline ctl` gives agents real headless terminals they can type into, wait on and read back, including cells, colours and cursor state as JSON. `decline pin rush --dock` gives a command its own Mac app, icon, windows and settings. Tab completion explains flags and subcommands using man pages and `--help`. These are differentiators to demonstrate, not claims that nobody else has ever done them.

## Claim ledger

| Claim | Evidence | Limits |
| --- | --- | --- |
| Agents can run a real terminal without opening a window | README: Driving it from agents and tests; docs/design/agent-api.md; crates/term-ctl | `ctl spawn`, `send`, `wait-for`, `screen --json`, `screenshot`, `kill`. Do not claim an AI assistant is built into the terminal. |
| A command becomes a Dock app | README: Pin a program to the Dock; DESIGN: Dock pins; docs/brand/pins.png | `decline pin rush --dock` requires rush installed. Uses the program's own icon when available, otherwise a generated pin. Do not call a generated sample the installed rush icon. |
| Completion explains flags | README: Completion; docs/img/screenshot-autocomplete.png; docs/design/completion.md | Tab uses zsh matches with descriptions from man pages or help. As-you-type completion is opt-in. Avoid claiming all shells have identical completion. |
| 26 MB idle, 47 MB with four tabs, 0.58 s to cat 100 MB | README: Benchmarks, 2 November 2026 | M1 Max, macOS 27.0, Menlo 13, 100×30 cells, 10k scrollback, physical footprint, medians of 5 interleaved runs, load 3–6. Decline 7849ad4, cat/idle CPU 82c455f. |
| Comparison: Ghostty 60 MB idle, 87 MB four tabs, cat 0.66 s; kitty 99 MB, 104 MB, 1.05 s | Same README table | Ghostty 1.4.0 and kitty 0.49.2, same 2 November conditions. CPU including WindowServer: Decline 0.66 s, Ghostty 2.04 s, kitty 1.63 s. |
| Older comparison: Decline 31.5 MB idle and cat 0.61 s; Ghostty 67 MB and 0.53 s | docs/comparison-2026-10-30.md | 30 October, 4 runs each (Terminal.app 3), 3 runs for four tabs. Decline lost cat wall time to Ghostty. No input latency measured. Warp ran two windows. Never merge these into the newer table. |
| Native text, ligatures, graphemes, images, reflow | Current README, DESIGN and CHANGELOG | Common capabilities, supporting detail only. Older comparison's final statement that Decline lacks shaping/graphemes/reflow is historical. |
| Install with Homebrew | README Install | `brew install --cask 0xdeafcafe/tap/terminal-decline`. No invented curl installer or download version. Link install instructions, no fake copy-button pill. |
| Extras opt-in | README | Completion as you type, output colouring, copy on select. Do not claim every enhancement is off. |

The ctl shell example needs an explicit Enter: `decline ctl send sh --text "make test" --key enter`. An isolated capture on installed build a4086db found the README's quoted `\n` is sent literally, so that example does not submit the command. Use the verified form in website examples; do not reproduce the README bug.

No universal fastest, zero CPU, zero wakeups, guaranteed battery saving, unmeasured latency win or current active-user count. The README's 0.02% idle CPU is not zero. Do not advertise automatic signed updates as currently available: README says the updater has no signed/notarised releases to offer until secrets are set.

## Brand and voice

Source: docs/brand/README.md. The still Long rest Slope, lowercase outlined `decline` wordmark, SF Pro Medium. Spruce #092D2B, mint #B9E5CC, paper #F3F3EB in brand artwork. For the website prefer white to prevent the cream/serif/green anti-slop cliché. Dark can use real spruce without glows or hairline cards. The brand font is deliberately native Mac typography. No Apple font distribution. Menlo is the terminal's default font, distinct from SF Mono used for commands in brand prose.

British English. Short, dry and specific. One small aside earns more than a joke in each heading. No em dashes. Avoid repeating the existing “Less overhead. More terminal.” slogan: the user's checklist bans that shape for new site copy. Do not reuse the README's Epstein joke. The first contact must explain terminal for macOS. Headings and navigation stay sentence case.

## Available visual evidence

- docs/img/screenshot-autocomplete.png: real window, git fetch flag list. A screenshot of the app, not a benchmark. Its development session header is not a measurement claim.
- docs/img/screenshot-settings.png: real settings TUI, Appearance and themes.
- docs/img/bench-light.svg and bench-dark.svg: existing dated benchmark charts. Verify values against the table when reusing.
- docs/brand/wordmark.svg: outlined identity, no font download required.
- docs/brand/icon.svg, icon-dark.svg and PNGs: real Slope artwork.
- docs/brand/pins.png: reference sheet of generated pins at different sizes.
- docs/brand/samples-generated/*.png: real term-icon outputs, 512 px and 16/32 px variants. These are artwork samples, not a screenshot of Alex's Dock. Label accordingly.

Use screenshots unaltered or clearly documented CSS crops. No invented terminal transcript. A command from the docs can appear as a command example, without fake prompt/output. Do not run pin commands against Alex's Dock to make evidence. Any new terminal capture must use an isolated headless session and record exactly what was run.

## Neighbours and the opening to avoid

See RESEARCH.md for sources and observations. “Fast, native, GPU accelerated” identifies a category. It does not tell a visitor why to try Decline. A technical documentation sidebar as the homepage hides the product's useful differences. An AI-platform pitch makes a small Mac terminal seem like an enterprise subscription. Decline should lead with measured speed and low footprint, then explain its other capabilities and installation requirements. Avoiding generic speed adjectives must not bury the actual performance advantage.

## Background direction selected by Alex

After asking for a subtle terminal-themed background and requesting ideation first, Alex chose both proposed motifs: quiet real scrollback excerpts and sparse large terminal glyphs. Studies are in `concepts/backgrounds/`. The combined study uses exact captured `ctl --help` output in Menlo, plus faint box-drawing corners. Apply the idea to the distinct concepts without losing their different layouts. Decoration is static and aria-hidden, kept out of foreground text. Clip the text itself, not just the page. No repeated grid, invented session, blinking cursor or glow. See the study DESIGN.md for provenance.

## Concept assignments

- Dockside: a spacious Mac product spread, now led by measured speed and small footprint. Real Decline app artwork, generous asymmetric composition, with pinning shown later. Neither a fake Dock nor feature cards.
- Fieldnotes: a measured-results working document. A narrow reading column alongside test conditions and sourced comparisons, then practical ctl examples. Typographic and precise, no boxed terminal mockup or decorative mono labels.
- Long rest: a graphic, full-width scroll story about low overhead rooted in the Slope geometry. Flat spruce/mint areas and scale changes. The first viewport shows the performance case with a complete still Slope. Completion moves down the page. No giant mono, parallax or generic centred hero.

The first drafts led with pins, ctl and completion respectively. Alex corrected that hierarchy: speed and low footprint lead all three. Their visual treatments remain different. Original before-review snapshots retain the first drafts.

Each concept is one self-contained index.html with inline CSS and embedded assets, no remote dependencies and no JS needed to read it. Keep a DESIGN.md beside it explaining tokens, source assets, hierarchy and small-screen behaviour. Light and dark follow the system. Cross-review every checklist item by a different model. Screenshots before and after: desktop 1440×900, tablet and mobile (390 px iframe inside Chrome's minimum viewport), both light and dark. All screenshots and temporary copies live under TMPDIR, not Git.

## Later build, after concept choice

Astro with @astrojs/react, TSX components rendered at build time and no framework JavaScript shipped. Scroll-told product page with native scrolling and reduced-motion support. site/public/CNAME contains decline.forbes.red. Pages workflow uses upload-pages-artifact and deploy-pages, main pushes touching site/**. Add site/** to existing Rust CI paths-ignore. Enable GitHub Pages build_type=workflow only after publishing approval. Alex adds DNS CNAME decline to 0xdeafcafe.github.io. Do not change DNS. No publishing or main push before Alex approves.

## Selection and production revision

Alex selected Long rest on 7 November 2026. He asked for a bolder/heavier
hero title and comparison data displayed inside a terminal preview on
the right. Production uses weight 800 and real captured Decline cells
rendered as selectable HTML, with recorded dates visible. See DESIGN.md
for the capture procedure and the distinction between displaying recorded
results and running a benchmark. The original concepts remain unchanged.

Alex then requested installation instructions in the header and more
window chrome around the terminal. The production header shows the Homebrew command directly, with a Copy
button. Alex rejected the earlier disclosure because the command was hidden. The terminal frame is the explicit C038
exception documented in ANTI-SLOP.md; its data remains a real capture.

After Alex rejected the verbose copy, production prose and headings were
cut from about 796 words to 201, including the collapsed benchmark method.
Feature sections have one short explanation. Provenance stays in design
notes; measurement conditions and the earlier loss remain accessible below
the charts. Do not add narrated captions, forced asides or redundant CTAs.

## 7 November: compact hero and text previews

Alex rejected the number-led hero and its unused space after comparing it
with Rush and Sounding. Lead with “A terminal for macOS.”, then the measured
footprint and cat time. Keep the install command visible in the header.
Render terminal views as coloured HTML text, with selectable content and
unselectable window furniture. Preserve real capture/transcription sources
in DESIGN.md. Pin artwork remains imagery.

## Selected round-two implementation

Alex chose Screen (1c), then explicitly changed the headline to
“A peppy terminal for macOS.” Keep Screen’s wide window and page-navigation
tabs, its compact single-pane phone opening and the visible install command.
All terminal evidence stays coloured HTML at 13px/1.3, selectable separately
from chrome. The selected source and cross-review are in
concepts/round-two/screen. New publication requires approval.
