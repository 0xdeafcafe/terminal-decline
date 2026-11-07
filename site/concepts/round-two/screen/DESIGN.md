# Round two: Screen

Model: Sonnet 5.5 (claude-sonnet-5-5). Written 7 November 2026.
One file, `index.html`: inline CSS, one small vanilla script (Copy button), inline SVG and base64 pin images. No remote requests, packages or framework scripts.

## Idea

The terminal surface is the page's main composition. A single native-style window spans the full content width and sits on a step: spruce above, page ground below, so the window straddles the edge. Inside it are real split panes, and the title bar's tabs are the page navigation (Footprint, Pins, Agents, Install). The heavy title sits above the window and the page then explains each pane. It is not title-left/terminal-right and has no alternating bands. The closing section uses the Slope large, as the one big brand graphic.

Panes in the window:

1. Recorded results (left): `site/src/data/terminal-preview.json`, captured 2026-11-07, 38 columns, source sha256 efdb3b45…. Spans, colours and bold are reproduced from the data; the text is unchanged.
2. Commands (top right): four commands from the README, shown as commands only, no prompt and no output. `ctl send` uses the verified form with `--key enter` (the README's quoted `\n` is sent literally, per site/BRIEF.md claim ledger).
3. Tab completion (bottom right): `site/src/data/completion-preview.json`, transcribed from docs/img/screenshot-autocomplete.png (sha256 eb698dc9…), truncation retained, `--depth` selected as in the source.

## Real data and sources

- Headline numbers, 26 MB idle and 0.58 s to `cat` 100 MB: `site/src/data/comparison.json` (2 November 2026, Apple M1 Max, macOS 27.0, medians of 5). The table is generated from that file; bars are relative to the largest value in each row.
- Method text (Menlo 13, 100×30, 10k scrollback, one window, load 3–6, Ghostty 1.4.0, kitty 0.49.2, physical footprint, CPU including WindowServer): site/BRIEF.md claim ledger and the current Measured component.
- Earlier loss: 30 October, Ghostty 0.53 s against Decline 0.61 s, no input latency compared (docs/comparison-2026-10-30.md). Kept visible, not merged into the newer table.
- Install: `brew install --cask 0xdeafcafe/tap/terminal-decline`, apple silicon, macOS 14 Sonoma or later, MIT.
- Pins: docs/brand/samples-generated rush, htop, claude, logs (downscaled to 256 px WebP) and rush-32 and rush-16 (PNG). Labelled generated, not the installed rush icon.
- Brand: outlined wordmark paths from docs/brand/wordmark.svg (paper background removed, fill and stroke set to currentColor, viewBox cropped to the artwork). Slope path from docs/brand/icon.svg, unaltered geometry (`M294 400 L484 610 Q497 624 516 624 H738`, stroke 92, round).

## Tokens

- Spruce #092D2B (plate, light ink), mint #B9E5CC (title, Copy button, best bars), deep #051A17 (dark ground, from the brand's dark body colour).
- Light: ground #FFFFFF, ink spruce, muted #4B6762, rule #CFE0D8, wash #EEF6F1. Dark: ground #051A17, ink #E4F0E9, muted #9DBBB0.
- The window uses the captured terminal colours in both modes (#16181c background, #d8dee9 foreground, #b5bd68 for Decline). Flag blue #81a2be, string yellow #f0c674, dim #9aa5b1 and selection #2b3b4d are my choices to colour command text; they are not part of the capture.
- Type: SF Pro through the system stack (the brand font is native), weights 400, 600 and 800, plus 700 only for bold cells reproduced from the capture. Six sizes: display clamp 46 to 112 px at 800, -0.045em; h2 clamp 30 to 48 px at 800; lead clamp 22 to 32 px at 600; body 17/1.6; small 15/1.5 (notes, captions, figcaptions, pin and install notes, Copy); 13 px for terminal text, window chrome and the header command (Menlo 13px/1.3 in the terminal). Radii: 3 (focus, bars), 6 (command chips, install, standalone command and completion blocks), 10 (window).
- Window chrome greys, chosen to sit beside the captured `#16181c`: title bar `#2a2d33`, tab strip `#202329`, dividers `#15171b` and `#343840`, title text `#c9ced6`, tab text `#aab1bb`, neutral window controls `#5b6068`. Plate text `#EAF7EF` and muted `#9CC4B3` on spruce; Copy hover `#D6F0E1`; light link `#0B5F52`; dark bars `#4F7D72`, rule `#1E4741`, wash `#0A2723`. One shadow, on the window only, tinted deep spruce `rgba(5,26,23,.55)`. One easing: `ease`, 150 ms, background colour only. Section spacing varies on purpose: 48, 88, 72 and 104 px above the four sections. The table is capped at 960 px so it does not stretch across the full 1232 px container; the window is the one full-width element.
- Chrome (title bar, tabs, Copy) is `user-select:none`; commands, results and completion rows are `user-select:text`.

## Behaviour

- Header install command is always visible. Copy uses `navigator.clipboard`, falls back to selecting the text and `execCommand`, and shows "Copied" or "Select text" for under two seconds.
- System light and dark through `prefers-color-scheme`. Native scroll. Transitions (150 ms background colour on Copy and tabs) only under `prefers-reduced-motion:no-preference`. No other motion.
- Below 860 px the window shows only the recorded-results pane, so the phone opening is the title, the numbers and the evidence. The commands pane is hidden because the same commands appear in the Pins and Agents sections. The completion pane moves into the Agents section as a standalone block (shown only below 860 px, and `display:none` above it, so assistive tech does not get it twice); below 560 px padding drops to 16 px and the install command wraps beside Copy. The completion grid wraps descriptions instead of clipping.

## Known weaknesses

- The traffic lights and title bar are fake window chrome; explicitly authorised in site/BRIEF.md, but still in conflict with ANTI-SLOP's fake-chrome item.
- The tabs look like native tabs but are anchors, and none is shown selected.
- The numbers table repeats data already in the results pane. It adds the other four measures and the method, but a reviewer may call it duplication.
- Light-mode Decline bars are mint with a spruce edge; they are lighter than the other bars by design, so weaker in contrast against white.
- The closing Slope is large and plain and the install section is a two-column layout.
- Headless Chrome could not render a true 390 px viewport (minimum window); mobile was only checked in a 390 px iframe at one screenshot. Dark mode was viewed once at 1440 wide. Neither was tested on a device or in Safari.
- No checklist scoring was done. A separate reviewer should audit every item.

## Cross-review changes (Opus 5.5, 7 November 2026)

See `SLOP-REVIEW.md`. Source changes: the phone opening now keeps one pane,
with completion moved to the Agents section below 860 px; pane labels were
removed; weights went from seven to three (plus capture bold) and sizes to
six. Copy changes: "Small, and quick to cat." became "Against Ghostty and
kitty"; "Ghostty and kitty beside it" became a plain conditions line; the
fragment drumbeats in Pins and Install were rewritten; the claim that
completion itself is opt-in was corrected to "Completion as you type" (README
line 25); "Idle" became "Idle memory"; the install note names both commands
as the README does; the footer no longer repeats the H1; the Copy fallback
now says "Press ⌘C". The Copy focus ring is mint (it was spruce on spruce, so invisible). "Idle CPU is a percentage of one core" was removed: no source states it. A favicon (docs/brand/icon.svg, unchanged) was added. On desktop the Agents section is mirrored (commands
left) so it does not repeat the Pins layout.
