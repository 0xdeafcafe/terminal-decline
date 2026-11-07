# Terminal Decline website

Alex selected Long rest. The Astro implementation uses TSX components
rendered at build time, with no client JavaScript. The hero has a heavier
title and renders a real captured Decline comparison screen as selectable
HTML. See DESIGN.md for evidence and validation, BRIEF.md for the claims,
and ANTI-SLOP.md for the design checklist.

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
Build assets are generated losslessly from the original images in the repo.
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
