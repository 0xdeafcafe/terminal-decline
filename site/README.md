# Terminal Decline website

Alex selected Screen from round two. The Astro implementation uses TSX
components rendered at build time. The only browser script copies the
visible install command; no framework runtime ships. The headline is
“A peppy terminal for macOS.” All terminal evidence is coloured HTML text,
with a single comparison pane on phones and split panes on desktop.

See DESIGN.md for provenance and validation, BRIEF.md for product claims,
and concepts/round-two/screen for the selected concept and cross-review.

## Local work

Use Node 24 and npm:

```sh
npm ci
npm run check
npm run build
npm run verify
npm run preview
```

Run these inside `site/`. `npm run dev` starts the development server.
Pin assets are generated from the original repo images, resized where
appropriate and encoded as lossless WebP.
Neither generated images nor `dist/` are committed. `public/CNAME` is
`decline.forbes.red`.

To update the terminal preview after changing the dated comparison data,
run `python3 site/scripts/capture-terminal.py` from the repo root on a Mac
with Decline installed. This is a display capture, not a new benchmark.

## Review history

`concepts/index.html` compares the three original concepts, with light,
dark and phone previews. Each has design notes and a 100-item cross-review
by a different model. The original Sol/Terra lanes were unavailable and
Kimi needed its account refreshed, so Opus, Sonnet and Fable produced the
concepts. The production version follows Alex's subsequent hero changes.
Screenshots live in TMPDIR, not Git. Concepts are not copied into the build.

## Publishing

`.github/workflows/site.yml` checks site changes and deploys approved main
pushes using Pages artifacts. Rust CI ignores site-only pushes and PRs.
Publishing, enabling Pages and pushing to main require Alex's approval.
After approval, configure Pages with `build_type=workflow` and the custom
domain via `gh api`, then follow the repo's CI and landing rules.

Alex owns DNS: add CNAME `decline` to `0xdeafcafe.github.io`.
