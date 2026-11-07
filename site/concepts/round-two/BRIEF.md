# Terminal Decline: second design round

Alex authorised landing the existing website, but called it bland and asked
for a new round. This is concept work. Do not change the Astro site yet.

Read site/BRIEF.md, site/ANTI-SLOP.md, docs/brand/README.md and README.md.
The current production implementation is a baseline to beat, not a template.
The new page must have a recognisable visual idea beyond two columns of SF
Pro, white space, a black window and alternating green bands. Do not solve
this with extra marketing copy, three cards, a CSS texture or novelty motion.

What Alex has already settled:
- Say what it is immediately: a terminal for macOS.
- Speed and small footprint are the main features: 26 MB idle, 0.58 s cat of
  100 MB, dated 2 November 2026, M1 Max, medians of five runs.
- A header install command you can see and copy:
  brew install --cask 0xdeafcafe/tap/terminal-decline
- Terminal evidence must be HTML text, not screenshots. Use existing real
  src/data/terminal-preview.json and completion-preview.json. Preserve source
  provenance, values and dates. Never invent a session or running benchmark.
- Terminal type is 13px / 1.3. Chrome has user-select:none; commands and
  evidence user-select:text. Native-looking chrome is explicitly authorised.
- A heavy title, a compact opening, and no clipping. At 1440x900 the complete
  opening and the start of the next section should be visible. At 390px
  there must be no page overflow or cropped meaningful text.
- System light/dark; native scroll; reduced motion; no framework JS.
- Spruce #092D2B, mint #B9E5CC, the exact Slope and outlined wordmark are real
  brand assets. The Slope is not a chart arrow or a download icon.
- Keep prose short, British, specific, no em dashes. No forced cheeky slogans.
- Pins and ctl support the speed/footprint story. Tab completion is not the
  headline. Keep earlier Ghostty cat win and method limits in method details.

Make a complete but concise concept, not just a hero. Use real pin artwork
as needed, inline all assets so index.html is self-contained. Small vanilla
clipboard JS is permitted; do not import packages or remote assets.
Record design choices and evidence in DESIGN.md, not in public-facing prose.
No commits, pushes, publication or edits outside your assigned directory.

Review process: parent takes before screenshots in TMPDIR, another model
reviews every checklist item and fixes the page, then parent takes after
screenshots. Do not claim checklist scores without the full item evidence.
An honest failure is better than the previous meaningless 99/100 scores.
