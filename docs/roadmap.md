# Roadmap to 1.0

**Terminal Decline 1.0** is a signed, notarized app on Homebrew that passes the
daily-driver checklist below, beats Ghostty on the thresholds below, has no UI
of its own beyond what macOS provides (native tabs, a split divider, standard
menus/find bar/alerts), and has Dock pins that survive upgrades.

Revised after an Opus (xhigh) review: the cell/storage redesign comes first
because ligatures, images, graphemes, reflow and most perf work sit on it;
pins and packaging come before the public beta; perf round 2 runs on the
final formats.

Owner key: **me** = main session; **agent** = background agent in its own
worktree, merged by me after fmt/clippy/tests; **you** = needs Alex.
Sizes: S < 1 day, M 1–3 days, L > 3 days (agent time).

## Definition of done

**Daily-driver checklist** (each verified in an integration or smoke test, or
a recorded manual check): zsh, bash, fish; Neovim (undercurl, true colour,
kitty keys); tmux; ssh; Claude Code (Shift+Enter, spinner, notifications);
fzf; lazygit; btop; less/man; delta; yazi with image previews.

**Thresholds** (fixed Mac, same window pixels/font/scrollback bytes as
Ghostty, footprint not RSS, WindowServer CPU included, interleaved runs):
idle CPU 0% and 0 wakeups/s; idle footprint ≤ 35% of Ghostty's; after 10k
lines ≤ 35%; `cat` 100 MB CPU (Terminal Decline + WindowServer) ≤ 45%; 4 tabs and 4
splits footprint ≤ 35%; input latency p99 ≤ Ghostty's.

**Every change** in M2/M2b carries before/after numbers on this standard
set, not only perf work: shaping, images and the updater each cost something.

## Now — in parallel

| # | Work | Owner | Size |
|---|---|---|---|
| N.1 | Finish M0: app library split + session module, integration and smoke tests; parser speed-ups (vendored vte) | agent | running |
| N.2 | All workflows green on GitHub; Dependabot PRs merged or closed | me | S |
| N.3 | Brand: Slope iteration (icons, dark/tinted, pin rule, wordmark) | Astra | running |
| N.4 | Signing secrets (Developer ID, notarization, Sparkle key) — needed early to spike pins | **you** | — |
| N.5 | Latency harness on a fixed Mac (keypress → present p50/p95/p99, idle and under flood); CI tracks event-to-encode time only | agent | M |
| N.6 | Failures you can see: config errors and "could not start program" printed into the window, which stays open (a broken pin no longer vanishes) | me | S |
| N.7 | Crash handling: panic hook → `~/Library/Logs/terminal-decline/` + os_log; dSYMs kept per release; release builds unwind (not abort) so a crash in one pane's parser shows an error in that pane and other windows keep running | me | M |
| N.8 | CI cost while private: macOS nightlies only when `main` changed since last run | me | S |
| N.9 | Pre-beta tags as `0.0.x`; `CFBundleVersion` an integer build number (nightlies too) | me | S |
| N.10 | Rename to Terminal Decline everywhere (app, bundle id, command with `decline` alias, config dir with migration, pins, cask; was E.17), before signed builds go out, P.1/P.2 and Sparkle; steps in order in docs/design/preferences.md | agent | S |

## F — Formats (the foundation)

| # | Work | Size | Done when |
|---|---|---|---|
| F.0 | **Design + Opus review** of the cell/style/run/scrollback model: 8-byte cell + per-screen style table (fg, bg, underline colour/style, overline, link id); grapheme store per row (+ mode 2027); kitty virtual-placement placeholders (U+10EEEE, ids in colours) stored losslessly; per-row change counters; shaping runs break only on font face, code-drawn glyphs, placeholders; ligature drawn as one glyph clipped per cell; glyph key (font id, glyph id) with a font registry; image placement table keyed by line number, freed with scrollback blocks; scrollback v2 (clusters, style fields, per-block link table, prompt marks, logical vs physical lines); APC support in vendored vte | M | reviewed and accepted |
| F.1 | Core: new cells, style table, graphemes, scrollback v2 | L | all conformance/property/fuzz tests green; bytes/line within 10% of today for plain text |
| F.2 | Font: cluster rasterising via CTLine, font registry, glyph-id keys | M | flags / family emoji / combining marks render |
| F.3 | Render: style table resolve, underline styles + colour, overline, new glyph key | M | Neovim undercurl visible once it knows we support it (after F.5) |
| F.4 | Reflow on resize + pull lines back from scrollback when growing (`resize_grow_height_pulls_from_scrollback` passes) | L | 120 Hz divider/window drag with 100k lines of scrollback, frame ≤ 8 ms (reflow lazy/debounced) |
| F.5 | Replies Neovim etc. use to detect features: XTGETTCAP (Smulx, Setulc, Tc, RGB), DECRQSS, OSC 4/104 palette, OSC 10–12 set + 110–112 reset, title push/pop | M | Neovim enables undercurl and true colour with `TERM=xterm-256color` |
| F.6 | Box-drawing, block and Powerline glyphs drawn in code | M | no seams at any size in tmux/htop/fzf/Claude Code |

## P — Pins and packaging (before the beta)

Design: docs/design/pins.md.

| # | Work | Size | Done when |
|---|---|---|---|
| P.1 | Bundle layout: `Terminal Decline.app` = launcher + `libterminal_decline.dylib`; inside-out signing (no `--deep`); hardened runtime; usage-description strings + entitlements (camera, microphone, Apple Events, location…) so programs in the terminal can ask | M | notarized build runs; a program asking for the mic gets a prompt |
| P.2 | Pin launcher: stub never changes after creation; versioned ABI (frozen at 1.0); verifies the library's signature (Apple anchor + team ID) before loading; finds Terminal Decline.app by its bundle id, `red.forbes.terminal-decline` (LaunchServices; config override for dev); clear message if missing; one-time migration for cloned pins; bare `cargo install` pins keep the clone method | L | upgrade needs no pin changes; pin permissions persist across upgrades (spiked early with real signing) |
| P.3 | Generated pin icons from the Slope rule (stable across upgrades; check how macOS 26/27 shows flat runtime `.icns`) | M | 8 sample names give distinct (initial, hue); brand doc's check value holds |
| P.4 | `pin --list` status, `--edit` (external config only), repair if Terminal Decline.app moved; pins from Finder/Spotlight/Login Items | S | smoke-tested |
| P.5 | Check App Management prompts when writing pins into `~/Applications` | S | documented behaviour |

## E — Everyday (then the public beta)

| # | Work | Size |
|---|---|---|
| E.1 | Native tabs: enable window tabbing, menu items, Cmd+T / Cmd+1…9; grid shrinks when the tab bar appears; background tabs give back memory | S |
| E.2 | OSC 8 links + Cmd-click; plain URLs | M |
| E.3 | Keyboard: non-Latin layouts for Ctrl/Meta; kitty protocol levels Neovim/Helix use (table tests from the spec + real-app checks); `keybind` config (remap/unbind/send text; per pin) | M |
| E.4 | Fonts: fallback list, bold/italic overrides, Nerd Font icon fitting, line height / cell width adjust | M |
| E.5 | Shell integration (OSC 133) via documented snippets for zsh/bash/fish (not injected): ~~jump between prompts, select output, Option-click to move the cursor~~ (all done) | M |
| E.6 | Notifications: OSC 9/777 and bell when unfocused (named: what Claude Code sends); guarded for the bare binary; Dock progress (OSC 9;4) | S |
| E.7 | Search: Cmd+F with our own native-style find bar (standard search field, previous/next, count, options; Edit > Find, Cmd+G/Cmd+E and the shared find text work as usual) over all of history; in-view hits first, then history outwards, using fast block search (SIMD substring search plus a per-block filter so most blocks are skipped) | M |
| E.8 | Selection: auto-scroll past the edge, rectangular (Option-drag), copy-on-select option | S |
| E.9 | Config: reload on change (FSEvents), list of live settings, `theme =` + a small built-in theme set + themes dir, `terminal-decline import ghostty` / `import iterm`, deprecation policy for renamed keys | M |
| E.10 | macOS: drag files in → escaped paths; folder on Dock icon / `open -a` → window there; Services "New Terminal Decline Window/Tab at Folder"; right-click menu when mouse reporting is off; confirm before closing a running program (config off) | M |
| E.11 | Secure Keyboard Entry (menu) | S |
| E.19 | Conformance fixes: the `to-fix` entries in docs/conformance-failures.md (OSC 4/10/11 colours, DECRQSS, DEC graphics glyphs, REP, DECALN, DECID, DECXCPR) | M |
| E.14 | Small bugs: ~~VS16 with autowrap off~~ (fixed by cells design PR 6); ~~CSI M on the top row~~ (already right since the cells design; pinned by a conformance test) | S |
| E.18 | Preferences: Cmd+, opens a tab running the built-in settings TUI (hand-rolled, its own process; sectioned list, live preview, keeps comments in config.toml), with the change to the reload's notices and chord recording through the app; the Pins section after P.2; design in docs/design/preferences.md | M |
| E.16 | Release: make repo + tap public, **then** tag `0.1.0`; changelog (git-cliff); cask caveats + `zap`; smoke on macOS 14 | S |

## B — Betas to 1.0 (0.x releases)

| # | Work | Size | Done when |
|---|---|---|---|
| B.1 | Ligatures: shape at draw time per style run with Core Text, cache per row counter, skip for fonts without ligatures, `font_features` | L | Fira/JetBrains Mono ligatures; frame build and `cat` measured Menlo vs JetBrains Mono |
| B.2 | Splits: one CAMetalLayer per window, plain NSViews per pane for input/IME/a11y, cached instances per pane, panes in sync-update reuse last frame, one display link per window, one glyph atlas per process | L | 4 panes all flooding: frame p99 and WindowServer+Terminal Decline CPU vs one pane; footprint per extra pane recorded |
| B.3 | Inline images: kitty graphics (APC; transmit/place/delete; virtual placements), iTerm2 OSC 1337; per-process memory budget (default 32 MB, Alex's decision), volatile while hidden; pixel caps, file/temp-file transmission limits; fuzz targets for graphics commands; design in docs/design/images.md | L | `kitty +kitten icat`, `imgcat`, yazi previews |
| B.4 | Updater: Sparkle 2 for everyone (the cask declares `auto_updates true`), in Terminal Decline.app's main process only (never pins), silent driver, no system profiling, daily check (DESIGN's "no timers" updated), EdDSA key backed up; design in docs/design/updater.md | M | update applied without prompts; signature checked |
| B.5 | Sixel (in 1.0), fuzzed decoder, same image budget; design in docs/design/images.md | M | `chafa --format sixel` |

## R — Performance round 2 (on the final formats)

| # | Work | Done when |
|---|---|---|
| R.1 | Frame build outside the lock (snapshot: cells, styles in use, grapheme strings, image placements, row counters); glyph rasterising outside the lock | p99 key latency under a 30k-glyph flood ≤ idle p99 + 2 ms |
| R.2 | Per-row instance caching (uses row counters) + row-end blank skip | sparse frame ≤ 15 µs at 203×50 |
| R.3 | Scrollback row cache while scrolled back | no decode per frame when the view doesn't move |
| R.4 | CSI/SGR parse fast path | coloured ≥ 1.5× vs the commit before R.4 |
| R.9 | Win back scrollback v2's costs (merged over the gate on purpose, 29 Oct): parse plain +6–16% and seq +6% (per-line store bookkeeping), unicode +9% (encoder per non-ASCII cell), CJK/ls history decode +12–20%; re-measure on a quiet Mac first | parse within 3% of before PR 7 |
| R.5 | Memory-pressure handler (spare drawables, purge textures, compress scrollback) | simulated warning: footprint drop then correct redraw |
| R.6 | `CAMetalDisplayLink` | kept only if N.5 numbers improve |
| R.7 | Idle scrollback compression (LZ4, two-block cache) | 100k lines of `ls`/log output ≤ 3 MB; decode ≤ 100 µs per block |
| R.8 | Small items (lazy colour texture, smaller first mask texture, lazy alt screen, block reuse, skip identical frames) | each kept only if footprint or CPU improves by ≥ 2% on the standard set |

## Z — 1.0

| # | Work | Done when |
|---|---|---|
| Z.5 | Tag `1.0.0`; consider submitting to homebrew/cask | — |

## Decisions (28 Oct 2026)

1. Brand: Slope, iterating (round with Astra).
2. Repo and tap go public at the 0.1.0 beta.
3. Signing: Alex adds the Apple signing secrets.
4. Homebrew: `0xdeafcafe/homebrew-tap` (created, private for now); cask only,
   no formula (a formula builds the bare binary: no pin launcher,
   notifications or updater).
5. 1.0 includes tabs, splits, ligatures, inline images and an in-app updater.
6. Not in 1.0 (29 Oct): VoiceOver/accessibility, a privacy statement, and
   the pre-1.0 gates (beta period, 14 clean fuzz nights, written docs, a
   published Ghostty comparison). 1.0 is tagged when the remaining work is
   done and the thresholds are met.

## How the work runs

- Phases run in order (Now → F → P → E → B → R → Z); items inside a phase
  run as parallel agents in worktrees, never two on the same crate.
- Every design goes through an Opus (xhigh) review before it's built; code
  reviews by Astra/Kimi at the end of each phase.
- Every change: conventional commit, fmt, clippy `-D warnings`, tests;
  numbers on the standard set where relevant; DESIGN.md kept current.
