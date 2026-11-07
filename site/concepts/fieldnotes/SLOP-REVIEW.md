# Fieldnotes: anti-slop review

Reviewer: claude-opus-5-5. Original author: claude-sonnet-5-5. Date: 7 November 2026.

Before is the immutable original at `fieldnotes/before/snapshot.html`. After is
the current `index.html`, SHA-256 `129398c99d2c105a…`, byte-identical to
`fieldnotes/after/snapshot.html`.

**Result: 90 of 100 PASS before, 99 of 100 PASS after.** The one remaining
FAIL is C100: the social card and 404 page are deferred, not done.

This also covers a change of direction. The reviewer rewrote the page during
review, so the after column includes judgements of their own edits. The Codex
parent subsequently read all 100 verdicts, inspected the final desktop light
and full light captures and the phone layouts in both schemes, and verified
that the final source matches the captured snapshot. This independent read
found no further blocker to concept selection. It does not replace the
accessibility and browser checks listed below.

## What changed

- Alex's priority applied: the page leads with "26 MB at idle.", 0.58 s and the
  2 November table, with conditions in the margin. ctl moved below.
- Alex's "both" background applied: real `ctl --help` text and two corners, in
  the empty right band only.
- Real evidence added: a `ctl screenshot` PNG and JSON replies from an isolated
  session, recorded in DESIGN.md.
- Parent's notes fixed: the tricolon heading, the repeated "not run here", the
  backend detail, the wait for evidence and the wrong "Two days earlier".
  Input latency is scoped to "These runs".
- Own findings fixed: 14 px text, the "149 MB" header beside a 26 MB claim (now
  cropped), flags breaking across lines, the privacy section, a colon reveal.
- One finding outside this concept: the README's `--text "make test\n"` doesn't
  press Enter when typed in zsh. Details in DESIGN.md.

## Screenshots

| Viewport | Mode | Before | After |
| --- | --- | --- | --- |
| desktop | light | [before](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/before/desktop-light.png) | [after](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/after/desktop-light.png) |
| desktop | dark | [before](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/before/desktop-dark.png) | [after](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/after/desktop-dark.png) |
| tablet | light | [before](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/before/tablet-light.png) | [after](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/after/tablet-light.png) |
| tablet | dark | [before](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/before/tablet-dark.png) | [after](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/after/tablet-dark.png) |
| mobile | light | [before](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/before/mobile-light.png) | [after](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/after/mobile-light.png) |
| mobile | dark | [before](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/before/mobile-dark.png) | [after](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/after/mobile-dark.png) |
| full | light | [before](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/before/full-light.png) | [after](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/after/full-light.png) |
| full | dark | [before](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/before/full-dark.png) | [after](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/after/full-dark.png) |

`full` is 1440×5600, and the page is 6058 px tall. Below-fold slices at 390 and
768 in both modes, plus the end of the page at 1440, are in
[`after/slices/`](file:///Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/fieldnotes/after/slices/).

## Validation and limits

- All eight captures came from direct headless Chrome at 1440×900, 768×1024 and
  a 390 px iframe, plus `full`, light and dark, with exit code 0. On earlier
  runs Chrome exited 2 with a teardown-watchdog message after writing its file;
  those files were replaced. The final logs have no watchdog line.
- Dark mode is forced with `--force-dark-mode`, not a real system setting.
- The 390 capture is an iframe, not a device. Nothing was checked in Safari or
  on a phone.
- 1600, 1280 and 1150 px were measured by DOM only, not looked at. The 22 slices
  and eight captures total 30 images. Of those, the final revision differs from
  the one fully inspected only by the install spacing at 42 to 68rem and below
  and by one full stop. The reviewer read a sample of slices (390 and 768, both
  modes) and all eight first-view captures on the previous revision, not every
  slice on the final one.
- Measurements are the README's, not re-run. Contrast figures are from DESIGN.md,
  and only the background layer was checked by eye.
- Links were not fetched. They use repository paths confirmed in this checkout.
- Keyboard use and screen readers were not tested.
- The checklist has judgement calls. C046 is the weakest PASS, and C051 passes
  because BRIEF allows documented commands as examples.

## All 100 checks

| ID | Section | Check | Before | After | Evidence |
| --- | --- | --- | --- | --- | --- |
| C001 | Type | No unchosen default fonts: Inter, Geist, Roboto, Arial, Open Sans, Lato or the system stack used only because they were the default. | PASS | PASS | SF Pro through the system stack is the brand's deliberate choice (documented exception). |
| C002 | Type | No "tasteful free" fonts used by reflex either: Space Grotesk (the model's favourite once Inter is banned), Instrument Serif, Fraunces, DM Sans, Cabinet Grotesk, Rajdhani, Share Tech Mono. | PASS | PASS | None used. Grep clean. |
| C003 | Type | No Instrument Serif (or any serif) headline paired with a mono caption, the 2026 "tasteful AI" default. | PASS | PASS | Sans headings only. |
| C004 | Type | No serif headline with one italic, accent-coloured word ("Software that *feels* alive"). | PASS | PASS | No serif, no accent word. |
| C005 | Type | No small uppercase, tracked eyebrow or mono caption above every heading ("01 / GET INTO IT", "AN INDEPENDENT APP", "FEATURES"). | PASS | PASS | No eyebrows. Headings stand alone. |
| C006 | Type | No all-caps, wide-tracked, mono section labels used as the page's chrome (about 42% of builder-made pages). | PASS | PASS | No uppercase labels. |
| C007 | Type | No monospace face at H1 size (JetBrains Mono, Geist Mono, IBM Plex Mono) as the only way to say "developer". | PASS | PASS | H1 is SF Pro. Mono stays at body size. |
| C008 | Type | Not lowercase-everything as an affectation: headings and buttons in plain sentence case unless the brand voice truly is lowercase. | PASS | PASS | Headings are sentence case. Lowercase decline is the brand name (documented exception). |
| C009 | Type | No very light weights (200 to 300) on grey text; weights limited to a few (for example 400, 500, 600). | PASS | PASS | Weights 400, 500, 600. |
| C010 | Type | One deliberate type scale with real contrast: 5 or 6 sizes at most, body at least 16 px, line height about 1.6 for body, tight tracking only on big display sizes. | FAIL | PASS | Before: Commands, captions and table sub-labels were 14 px, under the 16 px floor set for this review; eight size tokens. After: Five desktop sizes: 16, 18, 22, 30, H1. DOM check: no meaningful text under 16 px at six widths. |
| C011 | Type | Prose measure no wider than about 65ch, and no full-container-width paragraphs. | PASS | PASS | Prose 36rem at 18 px, about 64ch. Margin 32rem at 16 px. |
| C012 | Type | A font declared in CSS must actually load (no silent fallback to system sans). | PASS | PASS | `-apple-system` resolves in the captures. Off the Mac it falls back, recorded as a limit. |
| C013 | Type | No gradient text (`bg-clip-text text-transparent`) on "the one word that matters", and no one-word accent colour in the headline. | PASS | PASS | Grep clean for bg-clip-text. No accent word. |
| C014 | Type | No Title Case On Every Heading, button and bullet. | PASS | PASS | Sentence case throughout. |
| C015 | Type | No em-dash-heavy copy, and no decorative middle dots or `//` as separators in headers ("Home // Work", "A · B · C" meta strings). | PASS | PASS | Em dash grep count 0 on page. No decorative separators. |
| C016 | Colour | No purple, violet, indigo or blue gradients, and no Tailwind `indigo-*`, `violet-*`, `#6366f1`, or `--primary` hue 255 to 280. | PASS | PASS | No page gradients. Violet and indigo appear only inside the real pin sheet (documented exception). |
| C017 | Colour | No "tasteful default" cluster: warm cream or parchment ground (`#faf8f5`, `#f4f1ea`, `bg-stone-50`) plus serif plus terracotta (`#d97757` and neighbours), sage or forest green. Any two together is a strong signal. | PASS | PASS | White ground, no serif, no cream. Spruce is brand (documented exception). |
| C018 | Colour | No near-black ground with a single acid-green, vermilion or neon accent as the "dev tool" look (Linear-style `#08090a` canvas with `#23252a` hairlines). | PASS | PASS | Dark ground is spruce with mint (documented exception). The near-black PNG is real output. |
| C019 | Colour | No glowing blob, aurora, mesh gradient or radial bloom behind the hero or behind the product frame, and no coloured `shadow-[0_0_*]` glows. | PASS | PASS | None. |
| C020 | Colour | No untouched shadcn palette: `components.json` not left on `baseColor: "slate"` with unedited CSS variables, no `rounded-lg border bg-card shadow-sm` on everything, no slate or zinc greys by default. | PASS | PASS | No shadcn. Tokens are hand-set. |
| C021 | Colour | One dominant colour, one accent (the label red or yellow) and one neutral. No palette-cycling cards (red, blue, green, orange), and no emerald or teal fallback. | PASS | PASS | Spruce, mint, white. Pin hues live in artwork only. |
| C022 | Colour | No permanent dark mode by reflex. Follow the system, with a real light design. | PASS | PASS | Follows `prefers-color-scheme`. Both modes captured. |
| C023 | Colour | Grey tints chosen to match the palette, not default grey (also tint shadows, not pure black or grey). | PASS | PASS | Greys are spruce tints. No shadows. |
| C024 | Colour | Contrast of at least 4.5:1 everywhere, especially muted text on dark grounds. | PASS | PASS | Foreground tokens about 7:1 or more. Background text is aria-hidden decoration under Alex's exception, so not held to 4.5:1. |
| C025 | Colour | Not a flat, empty ground: if a texture is used it has a reason (see Layout). | FAIL | PASS | Before: Flat ground with no identity in it. Alex then asked for a terminal-themed background. After: Real captured `ctl --help` text and two corners in the empty right band, authorised by Alex. |
| C026 | Layout and components | No pill or badge above the hero headline ("✨ New", "AI-powered", "v1.0 is here", "Not on the App Store"), and no pulsing or blinking status dot in a corner. | PASS | PASS | None. |
| C027 | Layout and components | Not a centred hero with a subhead and two buttons, plus a logo strip (about half of builder-made pages centre everything). | PASS | PASS | Left-biased, no buttons, no logo strip. |
| C028 | Layout and components | Not the stock section order: hero, logos, three features, stats, testimonial, pricing, FAQ, CTA band, four-column footer. Order follows the story, not the template. | PASS | PASS | Order follows the story: numbers, ctl, typing, pins, install. |
| C029 | Layout and components | No row of three (or four, or six) identical icon-topped feature cards, and no bento grid of uneven icon boxes under a "FEATURES" label. | PASS | PASS | No cards. |
| C030 | Layout and components | No icon inside a rounded square tile, no Lucide-only icon set, and no sparkle, lightning bolt or brain icon to signal "AI". | PASS | PASS | No icons. |
| C031 | Layout and components | No emoji as icons, bullets or sidebar items (rocket, bolt, lock, check, sparkles). | PASS | PASS | No emoji. |
| C032 | Layout and components | No check-mark list under the hero buttons ("No credit card, free forever, cancel any time") and no check-mark feature lists in cards. | PASS | PASS | None. |
| C033 | Layout and components | No "1 · 2 · 3" numbered steps, no decorative "01 / 02 / 03" section numbering, no stats row, and no A/B/C lettered lists. | PASS | PASS | No numbering, and a dated table with conditions is not a stats row. |
| C034 | Layout and components | No stat banner with unsourced round numbers ("10M+", "99.9% uptime", "500x faster", "24/7") and no count-up animation. | PASS | PASS | All figures dated and conditioned. |
| C035 | Layout and components | No card around every block of content, no nested cards, and no coloured left or top border strip (3 to 4 px) on cards. Group with whitespace, proximity and type instead. | PASS | PASS | No cards. The margin rule is a detached mark, not a border strip. |
| C036 | Layout and components | No glassmorphism or backdrop-blur sticky nav by reflex, and not the same `rounded-2xl`, `shadow-lg`, `p-6` on everything. | PASS | PASS | No blur, no sticky nav, one 3px radius. |
| C037 | Layout and components | No "glowing" or gradient hairline borders (1 px light border with a gradient or a top radial glow on cards). | PASS | PASS | Table hairlines only, flat. |
| C038 | Layout and components | No fake window chrome: traffic-light dots (red, yellow, green), a fake title bar, a fake URL bar or a vertical rotated label. | PASS | PASS | Before: real window chrome in the screenshot only (documented exception). After: cropped out, no chrome at all. |
| C039 | Layout and components | No grid or dot-pattern background, noise or grain overlay, or "blueprint" crosshair marks used as a default finish. If texture exists it is part of the identity (the label, the paper), not an overlay. | PASS | PASS | Before: none. After: real, non-repeating help text and two corners, part of the identity by Alex's decision, not a pattern. |
| C040 | Layout and components | No marquee or infinite-scroll strip of logos, tags or quotes, and no "Trusted by" row of logos that are not real. | PASS | PASS | None. |
| C041 | Layout and components | No testimonial carousel, and no testimonials with placeholder avatars, "Jane Doe" names or round-number praise. | PASS | PASS | None. |
| C042 | Layout and components | No three-tier pricing with a ringed "Popular" middle card, unless there really is pricing (rush may have none). | PASS | PASS | No pricing. |
| C043 | Layout and components | No giant footer link grid duplicating the nav, with the same links in every column. | PASS | PASS | One line of footer. Links are a short list in install. |
| C044 | Layout and components | No decorative captions ("A GOOD CONNECTION.", "Design preview"), badges, page counts or extra stats nobody asked for. | FAIL | PASS | Before: Twelve "not run here" notes and repo file paths in captions: page-making commentary, not product. After: All removed. Captions describe the product or label capture. `not run here` count: 0. |
| C045 | Layout and components | Consistency: one corner radius (2 or 3 radii in total, and not `rounded-2xl` or `rounded-3xl` or `9999px` pills everywhere), one shadow rule, one spacing scale (8 pt), and no uniform section padding that makes every section the same weight. | PASS | PASS | One radius, one spacing scale, section weights differ. |
| C046 | Layout and components | Section rhythm varies: not every section the same heading, subhead, then grid structure. | PASS | PASS | Not every section matches, but the four ctl sections after the first share heading, prose, margin. It is the concept's form, with different margin content. Weakest pass here. |
| C047 | Layout and components | No full-width content stretched to a wide container (centred `max-w-7xl` 12-column grid with everything the same width); use a narrower measure and let some things break the grid. | PASS | PASS | Left-biased columns, figures capped at 72rem. |
| C048 | Layout and components | Focus states are visible, alt text exists, favicon, title and meta description are real and differ per page. | PASS | PASS | Focus ring on links, alt on images, favicon, title and description real. |
| C049 | Layout and components | Nothing overflows or overlaps: text fits its buttons and containers at every width, edges align, icons and arrows are optically centred. | PASS | PASS | DOM: scrollWidth equals viewport at six widths, no element outside it, no background over text or images. |
| C050 | Dev-tool pages | No terminal-window mockup with traffic-light dots wrapped around invented output. Real rush screens, real captured text, or no mock at all. | PASS | PASS | No mockup. Real PNG and real screenshot only. |
| C051 | Dev-tool pages | No fake or "perfect" demo output: round numbers, tidy timings, a `$` prompt in front of a command the tool does not take, or commands that were not run. | PASS | PASS | No invented output. Commands not run are README or `--help` examples, as BRIEF allows, and captured output is labelled. |
| C052 | Dev-tool pages | No typewriter animation typing the terminal output or the headline letter by letter. | PASS | PASS | None. |
| C053 | Dev-tool pages | No hero install line styled as a pill with a copy button and a `npx` or `curl / sh` that is not the real install route. | PASS | PASS | Plain selectable text, the real cask. |
| C054 | Dev-tool pages | No GitHub stars badge, "Open source" badge, or version pill used as decoration, and nothing that implies stats we do not have. | PASS | PASS | None. |
| C055 | Dev-tool pages | No row of `kbd` keycap chips as decoration ("⌘K", "Ctrl+Enter") unless each is a real rush key shown in context. | PASS | PASS | No keycaps. |
| C056 | Dev-tool pages | No "Built for developers", "Built for teams", "Built for speed" or "The AI-powered X for modern Y" mad-lib headline. Say what rush does for whom, in plain words. | PASS | PASS | Before: says what it does. After: a measurement. |
| C057 | Dev-tool pages | No "Linear clone" look: near-black canvas, hairline borders, radial glow over a dashboard frame, gradient headline word, glass cards. | PASS | PASS | Tinted spruce, no glow, no glass. |
| C058 | Dev-tool pages | No abstract "dashboard" or 3D blob as the hero image in place of the actual product. | PASS | PASS | Real product images. |
| C059 | Dev-tool pages | No product name in lowercase mono with a blinking cursor as the logo as the only identity. | PASS | PASS | Outlined brand wordmark. |
| C060 | Dev-tool pages | No generic "Get started" or "Learn more" CTA, and no "→" arrow glued to every link and button; labels say the exact action ("Install rush", "Read the config docs"). | PASS | PASS | "Install with Homebrew", "read the source". |
| C061 | Dev-tool pages | No Aceternity or Magic UI stock effects (spotlight, beams, meteors, shimmer buttons, animated borders) on a page that is not about them. | PASS | PASS | None. |
| C062 | Dev-tool pages | No privacy or "runs locally" reassurance that was not asked for or cannot be backed by a link to the code. | FAIL | PASS | Before: A whole section, "Who can reach it", gave socket and peer-check reassurance nobody asked for. After: Section deleted, along with its socket detail. Only the 30 minute kill stays, as behaviour. |
| C063 | Dev-tool pages | No leftover artefacts: "Vite + React" titles, default platform subdomains, builder badges, stale model names, an old footer year, section-label HTML comments, `Hero`/`Testimonials` class names, old product names in copy. | PASS | PASS | Grep clean. No artefacts. |
| C064 | Dev-tool pages | The page names its actual agents, keys, files and commands (real names from the Rust port) rather than category words ("workflow", "agents", "automation"). | PASS | PASS | Names `ctl`, `spawn`, `wait-for`, `man ls`, `pin`, `cat`, versions and commits. |
| C065 | Motion | No fade-and-slide-up on every section (`data-aos="fade-up"`, identical staggered reveals), and no motion library reveals as the only motion. | PASS | PASS | None. |
| C066 | Motion | No `whileHover={{ scale: 1.05 }}` or lift-with-shadow on every card, especially cards that are not clickable. | PASS | PASS | None. |
| C067 | Motion | No bounce or spring easing for its own sake, no count-up numbers, no auto-typing headlines, no glowing blob that follows the cursor. | PASS | PASS | None. |
| C068 | Motion | Motion must mean something: a row floating up, a chirp landing, the bottle. Two or three animation types at most, used consistently. | PASS | PASS | Only a link colour transition. |
| C069 | Motion | No scroll-jacking, parallax or carousels, and nothing that alters native scroll speed. | PASS | PASS | Native scroll. |
| C070 | Motion | No animation that restarts when the cursor moves, and `prefers-reduced-motion` is honoured. | PASS | PASS | Transition sits inside `prefers-reduced-motion: no-preference`. |
| C071 | Motion | Most interactions 150 to 300 ms; none over about 500 ms that feel like lag. | PASS | PASS | 150 ms. |
| C072 | Motion | No pointless hover effects, and no buttons that look clickable but go nowhere. | PASS | PASS | Only real links change on hover. |
| C073 | Motion | No loading text that narrates the build ("Initialising workspace...") in place of a real state. | PASS | PASS | None. |
| C074 | Copy | No adjective-stacked subtitles, "Elevate your...", "Supercharge", "Unleash", "Transform your", "Unlock the power of", "X, reimagined", "Say goodbye to X", or vague SaaS wording ("seamless", "powerful", "robust", "streamline", "cutting-edge", "effortless", "in just a few clicks"). | PASS | PASS | None. |
| C075 | Copy | No "No X. No Y. Just Z." negation list, and no "Less X, more Y" or "Forget X. Focus on Y.". | PASS | PASS | None. "no assistant of its own and no account" is one plain statement. |
| C076 | Copy | No "It's not X, it's Y" or "Not because X. Because Y." contrast reveal. | PASS | PASS | None. |
| C077 | Copy | No fragment drumbeat or tricolon headline ("Fast. Simple. Done.", "Build. Ship. Scale.", "Fast, secure and scalable"), and not three of anything by reflex. | FAIL | PASS | Before: H2 "Start it, wait for it, read it". After: Replaced. H1 is one short sentence, H2s are not lists. |
| C078 | Copy | No rhetorical question then answer ("The result? Calm."), colon reveal ("The best part: ...") or "Here's the thing" setup. | PASS | PASS | Before: none strong. After: the one colon reveal found was changed to a full stop. |
| C079 | Copy | No wisdom-shaped aphorisms ("Speed is a feature.") or performed cleverness ("Spreadsheets with a pulse."). | PASS | PASS | None. |
| C080 | Copy | No slogan on every section (one wink per section at most) and no tidy closer ("The future of X starts now."). | PASS | PASS | One dry aside at most per section, no closer. |
| C081 | Copy | Headline says the result or the thing, not the technology, and could not sit unchanged on a competitor's site. | PASS | PASS | Before: an agent result. After: "26 MB at idle.", a specific measurement. |
| C082 | Copy | No header, eyebrow, H1, description and footer all saying the same thing. | PASS | PASS | Each says something different. |
| C083 | Copy | No bold label on every bullet ("**Seamless integration:** ...") and no parallel bullet openers ("Ensures... Provides... Enables..."). | PASS | PASS | No bold-label bullets. |
| C084 | Copy | No synonym cycling: one name for one thing (agent, session, card) everywhere. | PASS | PASS | Session, pin and agent stay one word each. |
| C085 | Copy | No invented labels or false ranges ("from solo devs to enterprises"), no weasel sources ("trusted by industry leaders", "join thousands of developers"), no round-number claims. | PASS | PASS | None. |
| C086 | Copy | No generic FAQ filler or an FAQ with no answers. Answer only real questions. | PASS | PASS | No FAQ. |
| C087 | Copy | No narrated UI labels ("Choose a...") where a plain heading works, and no internal or backend terms leaking into user copy. | FAIL | PASS | Before: Backend terms in copy: reader thread condition variable, OSC 133, 0700 and 0600, PTY. After: Removed. Copy talks about cells, keys, sessions and commands. |
| C088 | Copy | Specific, product-true sentences in the owner's voice: British English, short, uneven in length. | FAIL | PASS | Before: "Two days earlier" for 30 October against 2 November was wrong, and "Nobody ran them for this page" was process narration. After: Fixed to "on 30 October" and "in a separate run", narration cut. Short, dry, British. |
| C089 | Copy | Real product evidence: real rush screens (as screenshots or as real text), not mock UI. | FAIL | PASS | Before: The lead claim, ctl, had no captured evidence. First real image came after several screens of text. After: Real `ctl screenshot` PNG, real JSON replies, measured table first, cropped real window. |
| C090 | Copy | Sentence and section rhythm is not uniform: some long, some short, some sections plainly larger than others. | PASS | PASS | Sentence lengths vary. |
| C091 | Craft | Squint test: blur the page. It must not read as "dark hero, glow, one centred button, three cards". | PASS | PASS | Before: a text document. After: table, band, then figures. Neither reads as hero and cards. |
| C092 | Craft | Side-by-side test: a stranger could tell it apart from a page generated by v0, Lovable, Bolt or Claude with a one-line prompt. | PASS | PASS | Marginal conditions and a table as the lead. |
| C093 | Craft | Swap-the-logo test: replace "rush" with another dev tool's name and the page must stop making sense. | PASS | PASS | Page depends on `ctl`, `pin` and dated figures. |
| C094 | Craft | Every colour, font, radius, shadow and easing is a recorded token in `DESIGN.md`, with the reason. Nothing is "whatever the model picked". | PASS | PASS | Recorded in DESIGN.md, including easing and background tokens. |
| C095 | Craft | Design came from a reference or mockup (the bottle, the label, a real terminal capture) first, and was then built. Not prompted from a blank page. | PASS | PASS | Brand spec, then real captures, then Alex's background study. |
| C096 | Craft | Screenshot-and-compare loop done: built page checked against the intended design at mobile, tablet and desktop, differences fixed. | FAIL | PASS | Before: The record shows no compare loop by the author. Captures were made centrally afterwards. After: After captures inspected, three rounds of fixes, final capture byte-identical to source. |
| C097 | Craft | No page drift: font, colour, radius, nav and footer identical across every page unless deliberately changed. | PASS | PASS | Single page, so not applicable. Marked PASS. |
| C098 | Craft | The scanner pass: grep the output for `rounded-2xl`, `rounded-3xl`, `backdrop-blur`, `bg-clip-text`, `indigo-`, `violet-`, `shadow-lg`, `data-aos`, `whileHover`, `from-purple`, `#6366f1`, `lucide`, and explain each hit. | PASS | PASS | Scanner grep had no hits in either version, data URIs excluded. |
| C099 | Craft | Imperfect on purpose in a few places: something hand-set (a custom rule, a drawn mark, an odd crop) that a template would not do. | PASS | PASS | Hand-set margin rule, plus the corners after. |
| C100 | Craft | Finish line done: favicon, social card, 404 page, per-page title and description, working links, real alt text, no placeholder content. | FAIL | FAIL | Still missing: social card and 404. Deferred until a concept is chosen. |
