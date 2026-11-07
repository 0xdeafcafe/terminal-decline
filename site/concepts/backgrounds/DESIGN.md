# Background studies

Alex requested terminal-themed backgrounds, asked for ideation first, then chose both proposed directions. These three isolated opening studies compare scrollback, glyphs and a restrained combination. They do not modify the three full-page concepts, select one of them, or publish anything.

All three use the same foreground: 26 MB idle physical footprint and 0.58 s for cat 100 MB. The table comes from the README's 2 November comparison; the adjacent source link gives full test conditions. These are measured cases, not guaranteed runtime requirements or a universal fastest claim.

## Evidence

The background is actual stdout/stderr captured from this read-only command on 7 November 2026:

```sh
"/Users/afr/Applications/Terminal Decline.app/Contents/MacOS/terminal-decline" ctl --help
```

Captured output SHA-256: `09c36b7d5fbef3c79a84b86a66c57b8ed9e7ebeaa8b77b3b4ab67058cb874a53`. Exact line slices 10–24 and 29–44 are embedded as HTML text. No prompt or output was invented. A plain help invocation does not create a terminal session. These excerpts are decoration, not a demonstration of ctl behaviour.

## Tokens and rationale

White / spruce #092D2B grounds, spruce / paper #F3F3EB foregrounds, #245950 / mint #B9E5CC accents come from the Slope identity and built-in Slope themes. Muted body #47625E / #B4CFC3 remains readable. SF Pro via the Mac system stack for the product copy; Menlo for captured text and the real Unicode box-drawing glyphs. No distributed Apple fonts.

Background text has 9–12% opacity, the corners 6.5–8.5%, with a stronger short strip on mobile. These are intentionally decorative and aria-hidden, not meaningful text subject to the body contrast target. Pointer events and selection are disabled. Nothing animates, blinks, types, follows a cursor or changes scroll speed. Reduced motion is explicit.

The content width is 1040px with a 720px evidence table. Background excerpts live in the margins, with one short strip above the mobile opening. Corners are sparse, not a repeated grid, frame or invented app chrome. No shadows, radii or gradients are applied. The intact outlined wordmark is the source artwork with its paper rectangle removed, correct brand colour inherited and viewBox trimmed to the artwork.

This is a background choice study only: no social card, 404 page, full page story or production audit. The existing concept audits remain unfinished/out of date until the selected background is applied and reviewed.
