# Roadmap to 1.0

"Done" means **decline 1.0**: a signed, notarized app on Homebrew that you can
use all day instead of Ghostty, with measured lower memory and CPU, no UI of
its own, and Dock pins that survive upgrades. Each milestone ends with tests
green, numbers recorded in DESIGN.md, and a tagged release from 0.2 on.

Owner key: **me** = the main session; **agent** = a background agent in its
own worktree, merged by me after fmt/clippy/tests pass; **you** = needs
Alex.

## M0 — Finish what's running (in progress)

| # | Work | Owner | Done when |
|---|---|---|---|
| 0.1 | App split into library + `session` module; key/mouse/layout/frame-policy modules; hermetic pins/config | agent | view.rs is AppKit glue only |
| 0.2 | Headless integration tests (shells, vim, less, curses, queries, paste, resize, reader/main-thread races) | agent | pass 5× in a row |
| 0.3 | Real-app smoke tests via `DECLINE_TEST_DUMP` + private OSC | agent | run in CI, required |
| 0.4 | Parser speed-ups: bulk ASCII runs (vendored vte), SWAR scan, faster scrollback encoding, ring screen | agent | ≥ 2× plain, no conformance/property/fuzz regressions |
| 0.5 | First green run of every new workflow (CI, fuzz, bench, nightly props, weekly pressure, coverage); fix runner issues (Metal device, window server) | me | all green on GitHub |
| 0.6 | Merge or close open Dependabot PRs | me | none open |

## M1 — Performance, round 2

Ordered by the reviews' win-per-risk.

| # | Work | Done when |
|---|---|---|
| 1.1 | Stop each row after its last non-blank cell when building frames (B3) | sparse frame ≤ 15 µs at 203×50 |
| 1.2 | Build GPU instances outside the terminal lock; rasterise new glyphs outside it | lock held only for a cell copy; typing never waits on a glyph |
| 1.3 | Cache decoded scrollback rows while scrolled back | no decode per frame when the view hasn't moved |
| 1.4 | Memory-pressure dispatch source: on warning free spare drawables, purge textures, compress scrollback | handler tested by calling it directly |
| 1.5 | Fast path for CSI/SGR parsing | coloured output ≥ 1.5× |
| 1.6 | `CAMetalDisplayLink` instead of display link + blocking `nextDrawable` | main thread never blocks on a drawable; latency measured before/after |
| 1.7 | Small items: 16-byte glyph instances, colour texture created on first emoji, mask texture starts 512², alt screen allocated on first use, reuse freed scrollback blocks, skip presenting identical frames | each measured, kept only if it wins |
| 1.8 | Compress sealed scrollback blocks when idle/hidden (LZ4), two-block decode cache | 100k-line scrollback ≤ 3 MB |
| 1.9 | Fair comparison report vs Ghostty (same window pixels, font, scrollback bytes; interleaved runs; footprint not RSS; WindowServer CPU included; energy via powermetrics) | table in DESIGN.md + reproducible script |
| 1.10 | Input latency measurement (keypress → present, idle and under load, p50/p95/p99) | numbers in DESIGN.md, no regressions in CI trend |

## M2 — Everyday correctness and features

| # | Work | Done when |
|---|---|---|
| 2.1 | Wider cells or a style table: grapheme clusters (combining marks, ZWJ emoji), underline styles (curly/dotted/dashed, SGR 4:x) and underline colour (SGR 58), overline | Neovim undercurl, flags/family emoji render correctly; scrollback format carries them |
| 2.2 | Box-drawing, block and Powerline glyphs drawn in code | no seams in tmux/htop/fzf/Claude Code borders at any size |
| 2.3 | OSC 8 hyperlinks + Cmd-click to open; Cmd-click plain URLs | links open in the browser; hover shows underline |
| 2.4 | Reflow on resize; pull lines back from scrollback when growing | Kimi's ignored grow test passes; selection survives reflow |
| 2.5 | Keyboard: Ctrl/Meta on non-Latin layouts (ASCII-capable layout lookup), kitty protocol beyond flag 1 (report events/alternates as far as Neovim/Helix use) | Ctrl+C works on Russian layout; kitty's own test program passes the levels we claim |
| 2.6 | Shell integration: OSC 133 prompt marks → jump between prompts (Cmd+↑/↓), select command output | works with zsh/bash/fish snippets in docs |
| 2.7 | Notifications: OSC 9 / 777 and bell → macOS notification when the window isn't focused | Claude Code's "done" bell shows a notification |
| 2.8 | Remaining replies/modes: title push/pop (CSI 22/23 t), DECRQSS, XTGETTCAP, mode 2031 colour-scheme updates + light/dark theme pair in config, mode 1016 pixel mouse | Neovim/`delta`/`bat` detect correctly |
| 2.9 | Selection polish: auto-scroll while dragging past the edge, rectangular selection (Option-drag), copy on select (config) | — |
| 2.10 | Config reload on file change (FSEvents, no polling); config reference doc generated from the struct | edit → applied within a second, zero idle wakeups |
| 2.11 | Secure Keyboard Entry (menu item + automatic for password prompts? menu only) and basic VoiceOver text | VoiceOver reads the screen |
| 2.12 | Known small bugs: VS16 with autowrap off widens the wrong cell; CSI M on the top row and line numbers | regression tests |
| 2.13 | vttest and esctest2 runs; fix what matters | results recorded; no crashes |

## M2b — Tabs, splits, ligatures, images, updates (in 1.0 by decision)

Zero UI still holds: tabs are macOS's own window tabs, splits are just a
divider line, and the updater shows nothing unless an update is applied.

| # | Work | Done when |
|---|---|---|
| 2b.1 | **Tabs** via native macOS window tabbing (Cmd+T, Cmd+Shift+[ ], Cmd+1…9, drag tabs out to windows); titles from OSC 0/2; no custom tab bar | behaves like Terminal.app tabs; each tab its own session |
| 2b.2 | **Splits**: Cmd+D / Cmd+Shift+D, 1 px divider only, drag to resize, Cmd+Option+arrows to move focus, zoom a pane (Cmd+Shift+Enter); layout tree per window; each pane its own session and renderer target in one Metal layer | 4-pane layout at 120 Hz with no extra idle cost; closing a pane reflows the rest |
| 2b.3 | **Ligatures / text shaping**: shape runs of same-style text with Core Text (`CTLine`/`CTRun`), cache shaped runs per row; glyph cache keyed by font + glyph id; `font_features` config (e.g. `-calt`); cursor and selection still per cell | Fira Code / JetBrains Mono ligatures render; shaping cost cached so plain-text throughput drops < 10% |
| 2b.4 | **Inline images**: kitty graphics protocol (transmit, place, delete; virtual placements), iTerm2 OSC 1337 inline images, Sixel; images as GPU textures with a memory budget and eviction; placements move with scrollback | `kitty +kitten icat`, `imgcat`, `chafa --format sixel` work; image memory capped (default 64 MB) |
| 2b.5 | **In-app updater**: Sparkle 2 with an EdDSA-signed appcast on GitHub releases; checks daily in the background, downloads silently, installs on quit; turned off automatically for Homebrew installs (cask declares `auto_updates true`) and by config | update applied without any window or prompt; signature verified |

These depend on M2's cell redesign (2.1): images and shaped runs both need
the new cell/run model, so 2.1 is designed once with all three in mind.

## M3 — Dock pins that survive upgrades

| # | Work | Done when |
|---|---|---|
| 3.1 | Pin launcher: small stub executable + `libdecline.dylib` inside `Decline.app`; pin bundles hold only the stub, Info.plist, icon; stub finds Decline.app by bundle id and loads it | upgrading decline needs no `pin --update`; macOS permissions granted to a pin persist |
| 3.2 | Generated pin icons from the brand template (initial + hue from name), used when `--icon` isn't given; icons follow macOS 26/27 shape | pins tell apart in the Dock at 16 px |
| 3.3 | `decline pin` polish: `--list` shows status, `--edit` opens the pin's config, repair command if Decline.app moved | — |
| 3.4 | Pins opened from Finder/Spotlight/Login Items behave like the Dock | smoke-tested |

## M4 — Brand, packaging, public beta

| # | Work | Owner | Done when |
|---|---|---|---|
| 4.1 | Brand round 2 with Astra, then pick a direction | agent / **you** | decided |
| 4.2 | App icon (.icns, all sizes, macOS 26/27 layered style) in `Decline.app`; README header, wordmark | agent | icon in the bundle |
| 4.3 | Make the repo and tap public at the 0.1.0 beta | me (at 4.8) | public |
| 4.4 | Apple Developer ID secrets in GitHub: `APPLE_CERT_P12`, `APPLE_CERT_PASSWORD`, `APPLE_TEAM_ID`, `APPLE_ID`, `APPLE_APP_PASSWORD`; plus Sparkle `SPARKLE_ED_PRIVATE_KEY` | **you** | release workflow signs and notarizes |
| 4.5 | Homebrew tap: repo created (private until the beta); add `HOMEBREW_TAP_TOKEN` (fine-grained PAT, contents write on the tap) | **you** (token) | `brew install --cask 0xdeafcafe/tap/decline` works |
| 4.6 | Changelog from conventional commits (git-cliff) and release notes in the release workflow | me | tag → release with notes |
| 4.7 | `cargo install --git` path verified on a clean machine (bare binary without bundle) | me | documented and tested in CI |
| 4.8 | Tag **0.1.0** (public beta) | me, after 4.3–4.5 | release + cask published |

## M5 — 1.0 hardening

| # | Work | Done when |
|---|---|---|
| 5.1 | Daily-driver period: use it for a week; triage issues | no open crash/data-loss bugs |
| 5.2 | Fuzzing clean for 14 nightly runs; property tests clean at 50k cases | — |
| 5.3 | Docs: README, config reference, keyboard reference, pins guide, `decline --help`/man page, CONTRIBUTING, AGENTS.md current | — |
| 5.4 | Final fair benchmark report vs Ghostty and Warp published in README | numbers reproducible from the repo |
| 5.5 | Tag **1.0.0** | — |

## Decisions (28 Oct 2026)

1. Brand: another round with Astra before choosing.
2. Repo and tap go public at the 0.1.0 beta.
3. Signing: Alex adds the Apple signing secrets.
4. Homebrew: `0xdeafcafe/homebrew-tap` (created, private for now).
5. 1.0 includes tabs, splits, ligatures, inline images and an in-app
   updater (M2b).

## How the work runs

- Milestones run in order, but independent items inside a milestone run as
  parallel agents in worktrees (never two agents on the same crate).
- Every design or plan goes through an Opus (xhigh) review before it's built;
  code reviews by Astra/Kimi at the end of each milestone.
- Every change: conventional commit, fmt, clippy `-D warnings`, tests; perf
  changes carry before/after numbers.
- After each milestone: pressure run, DESIGN.md numbers updated, tag.
