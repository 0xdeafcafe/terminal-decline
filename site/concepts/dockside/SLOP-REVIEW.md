# Dockside: anti-slop cross-review

Reviewed by Claude Fable 5.1 on 7 November 2026 against `site/BRIEF.md`,
`site/ANTI-SLOP.md` and the 100 checklist items, quoted in full in the
table below so this file stands on its own. The concept was written by Claude Opus 5.5. Verified via
the session list by Alex: different model from the author.

**Before** is the immutable original snapshot, `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/before/snapshot.html`.
**After** is `site/concepts/dockside/index.html` as it stands, whose SHA-256
matches `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/after/snapshot.html` (`f6bff1f16d7f1b7c…`).

Before: 94 pass, 6 fail. After: 99 pass, 1 fail (C100, deferred).

## What changed between before and after

Alex changed the brief twice during the review, so the after state is not
only a fix of the original's fails:

1. Review fixes to the original pin-led page: internals out of the public
   copy, hue captions and repeat disclaimers cut, the sample pin's caption
   made explicit that it is generated and that a program's own icon is
   kept, commands no longer wrap at hyphens, the hero fade removed.
2. Alex: lead with measured speed and footprint. The page now opens on
   "26 MB at idle." with the 2 November table in the first viewport, the
   conditions and the lost 30 October run right after, then the everyday
   terminal, Tab, pins (compressed to two sections), agents, install.
3. Alex: the combined background from `site/concepts/backgrounds/`. The
   opening carries two clipped `pre` blocks of real `ctl --help` output and
   two box-drawing corners under the content, reduced to a strip on
   mobile. Authorised brand exception; provenance in both DESIGN files.

Also recorded: the README's `ctl send sh --text "make test\n"` sends a
literal backslash-n rather than Enter (Opus, on the installed build). Not
on this page; `site/concepts/long-rest/index.html` uses that line and
needs the same correction by its reviewer.

## Screenshots

Taken with headless Chrome by the shared `capture.py`: desktop 1440×900,
tablet 768×1024, mobile as a 390 px iframe in a 500 px window, and a
"full" 1440×5600 window, light and dark.

| | Before | After |
| --- | --- | --- |
| Desktop light | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/before/desktop-light.png` | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/after/desktop-light.png` |
| Desktop dark | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/before/desktop-dark.png` | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/after/desktop-dark.png` |
| Tablet light | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/before/tablet-light.png` | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/after/tablet-light.png` |
| Tablet dark | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/before/tablet-dark.png` | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/after/tablet-dark.png` |
| Mobile light | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/before/mobile-light.png` | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/after/mobile-light.png` |
| Mobile dark | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/before/mobile-dark.png` | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/after/mobile-dark.png` |
| Full light | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/before/full-light.png` | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/after/full-light.png` |
| Full dark | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/before/full-dark.png` | `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-site-review/dockside/after/full-dark.png` |

Validation limits:

- Chrome exited 2 on six of the eight after captures with a teardown
  watchdog message in the log (`*.log` beside each PNG) and
  `CVDisplayLinkCreateWithCGDisplay` errors on every run. Every PNG was
  written with the expected dimensions and was opened and inspected.
- The "full" capture is capped at 5600 px by the script; the page is
  about 8300 px tall at 1440, so the full shots end in the agents
  section. The lower sections were inspected in 1440×8600, 768×8000 and
  500×7000 captures in the reviewer's `TMPDIR` (`dockside-inspect/`),
  which are not durable.
- Headless Chrome, not Safari: SF Pro rendering and `text-wrap: balance`
  were checked in Chrome only.
- Hover, focus and reduced-motion states were checked by reading the CSS,
  not by driving the page.
- No production checks: this is one concept file, not a deployed site.

## The 100 items

| ID | Section | Check | Before | After | Evidence |
| --- | --- | --- | --- | --- | --- |
| C001 | Type | No unchosen default fonts: Inter, Geist, Roboto, Arial, Open Sans, Lato or the system stack used only because they were the default. | PASS | PASS | SF Pro via `-apple-system`, the brand's own choice (docs/brand/README.md); Helvetica fallback documented. Not a default. |
| C002 | Type | No "tasteful free" fonts used by reflex either: Space Grotesk (the model's favourite once Inter is banned), Instrument Serif, Fraunces, DM Sans, Cabinet Grotesk, Rajdhani, Share Tech Mono. | PASS | PASS | No webfonts at all. Menlo only for captured text and commands. |
| C003 | Type | No Instrument Serif (or any serif) headline paired with a mono caption, the 2026 "tasteful AI" default. | PASS | PASS | No serif anywhere; captions are SF Pro at 14 px, not mono. |
| C004 | Type | No serif headline with one italic, accent-coloured word ("Software that *feels* alive"). | PASS | PASS | No italic or accent-coloured word in any heading. |
| C005 | Type | No small uppercase, tracked eyebrow or mono caption above every heading ("01 / GET INTO IT", "AN INDEPENDENT APP", "FEATURES"). | PASS | PASS | No eyebrows. The masthead "terminal for macOS" is the brand's required category pairing beside the wordmark, once. |
| C006 | Type | No all-caps, wide-tracked, mono section labels used as the page's chrome (about 42% of builder-made pages). | PASS | PASS | No uppercase or tracked labels; headings are sentence case. |
| C007 | Type | No monospace face at H1 size (JetBrains Mono, Geist Mono, IBM Plex Mono) as the only way to say "developer". | PASS | PASS | H1 is SF Pro Display; mono only for commands. |
| C008 | Type | Not lowercase-everything as an affectation: headings and buttons in plain sentence case unless the brand voice truly is lowercase. | PASS | PASS | Only the wordmark is lowercase, per the brand. Headings and links sentence case. |
| C009 | Type | No very light weights (200 to 300) on grey text; weights limited to a few (for example 400, 500, 600). | PASS | PASS | Weights 400, 500, 600 only; no light weights (grep for 200/300 weights: none). |
| C010 | Type | One deliberate type scale with real contrast: 5 or 6 sizes at most, body at least 16 px, line height about 1.6 for body, tight tracking only on big display sizes. | PASS | PASS | Six sizes (note 14, body 17/1.6, lede 21, sub, head, big). After: the H1 and install heading share `--s-big`; an interim revision had a seventh size, removed before the final capture. |
| C011 | Type | Prose measure no wider than about 65ch, and no full-container-width paragraphs. | PASS | PASS | `p { max-width: 62ch }`; lede 30em; the pin dl and table are not prose. |
| C012 | Type | A font declared in CSS must actually load (no silent fallback to system sans). | PASS | PASS | System stack; nothing declared that could fail to load. |
| C013 | Type | No gradient text (`bg-clip-text text-transparent`) on "the one word that matters", and no one-word accent colour in the headline. | PASS | PASS | No gradient text, no accent word. |
| C014 | Type | No Title Case On Every Heading, button and bullet. | PASS | PASS | All headings sentence case ("How it was measured, and the run it lost."). |
| C015 | Type | No em-dash-heavy copy, and no decorative middle dots or `//` as separators in headers ("Home // Work", "A · B · C" meta strings). | PASS | PASS | Zero em dashes (grep). The only middle dots are inside the app screenshot. |
| C016 | Colour | No purple, violet, indigo or blue gradients, and no Tailwind `indigo-*`, `violet-*`, `#6366f1`, or `--primary` hue 255 to 280. | PASS | PASS | No purple or blue gradients on the page. Pin gradients are term-icon artwork (brand palette), documented exception. |
| C017 | Colour | No "tasteful default" cluster: warm cream or parchment ground (`#faf8f5`, `#f4f1ea`, `bg-stone-50`) plus serif plus terracotta (`#d97757` and neighbours), sage or forest green. Any two together is a strong signal. | PASS | PASS | White ground, no serif, no terracotta. Spruce and mint are the sourced brand colours. |
| C018 | Colour | No near-black ground with a single acid-green, vermilion or neon accent as the "dev tool" look (Linear-style `#08090a` canvas with `#23252a` hairlines). | PASS | PASS | Dark mode is brand spruce `#092D2B`, not near-black; no neon accent; rules are tinted `#2C5550`. |
| C019 | Colour | No glowing blob, aurora, mesh gradient or radial bloom behind the hero or behind the product frame, and no coloured `shadow-[0_0_*]` glows. | PASS | PASS | No blobs, blooms or glow shadows (scanner: none). |
| C020 | Colour | No untouched shadcn palette: `components.json` not left on `baseColor: "slate"` with unedited CSS variables, no `rounded-lg border bg-card shadow-sm` on everything, no slate or zinc greys by default. | PASS | PASS | No shadcn or Tailwind; hand-written CSS with named tokens. |
| C021 | Colour | One dominant colour, one accent (the label red or yellow) and one neutral. No palette-cycling cards (red, blue, green, orange), and no emerald or teal fallback. | PASS | PASS | Spruce dominant, mint accent, white neutral. Pin hues are artwork. Exception recorded in DESIGN.md. |
| C022 | Colour | No permanent dark mode by reflex. Follow the system, with a real light design. | PASS | PASS | `prefers-color-scheme` swaps tokens; light is the primary design (before/desktop-light.png, after/desktop-light.png). |
| C023 | Colour | Grey tints chosen to match the palette, not default grey (also tint shadows, not pure black or grey). | PASS | PASS | `--quiet #47625E`, `--rule #C9D9D1`, dark `#B4CFC3` / `#2C5550`: all spruce-tinted. |
| C024 | Colour | Contrast of at least 4.5:1 everywhere, especially muted text on dark grounds. | PASS | PASS | Quiet on white 6.61:1, dark quiet on spruce 8.90:1, swap-quiet on mint 6.05:1. After: background texture at 9% is decorative, aria-hidden, under no copy; not body text. |
| C025 | Colour | Not a flat, empty ground: if a texture is used it has a reason (see Layout). | PASS | PASS | Before: plain white ground. After: captured `ctl --help` text and two corners in the opening margins, authorised by Alex, provenance in backgrounds/DESIGN.md. |
| C026 | Layout and components | No pill or badge above the hero headline ("✨ New", "AI-powered", "v1.0 is here", "Not on the App Store"), and no pulsing or blinking status dot in a corner. | PASS | PASS | No badge, pill or status dot above the H1. |
| C027 | Layout and components | Not a centred hero with a subhead and two buttons, plus a logo strip (about half of builder-made pages centre everything). | PASS | PASS | Left-aligned 7/5 (before) and 6/6 (after) spread; links are plain underlined text; no logo strip. |
| C028 | Layout and components | Not the stock section order: hero, logos, three features, stats, testimonial, pricing, FAQ, CTA band, four-column footer. Order follows the story, not the template. | PASS | PASS | Before: pins, generator, bundle, completion, agents, terminal, measurements, install. After: result, conditions, terminal, completion, pins, agents, install. Neither is the template order. |
| C029 | Layout and components | No row of three (or four, or six) identical icon-topped feature cards, and no bento grid of uneven icon boxes under a "FEATURES" label. | PASS | PASS | No feature cards. The 4×3 pin specimen is twelve real artwork samples with names, labelled as samples. |
| C030 | Layout and components | No icon inside a rounded square tile, no Lucide-only icon set, and no sparkle, lightning bolt or brain icon to signal "AI". | PASS | PASS | The only rounded-square icons are the pins themselves, which are the product. No icon set. |
| C031 | Layout and components | No emoji as icons, bullets or sidebar items (rocket, bolt, lock, check, sparkles). | PASS | PASS | No emoji. |
| C032 | Layout and components | No check-mark list under the hero buttons ("No credit card, free forever, cancel any time") and no check-mark feature lists in cards. | PASS | PASS | No check-mark lists. |
| C033 | Layout and components | No "1 · 2 · 3" numbered steps, no decorative "01 / 02 / 03" section numbering, no stats row, and no A/B/C lettered lists. | PASS | PASS | No numbered steps or stats row. The measurement table carries date, versions, units and conditions. |
| C034 | Layout and components | No stat banner with unsourced round numbers ("10M+", "99.9% uptime", "500x faster", "24/7") and no count-up animation. | PASS | PASS | Every figure is a dated median with a linked script; the run Decline lost is on the page. |
| C035 | Layout and components | No card around every block of content, no nested cards, and no coloured left or top border strip (3 to 4 px) on cards. Group with whitespace, proximity and type instead. | PASS | PASS | No cards or border strips; grouping by whitespace and the grid. Table rows use hairlines only. |
| C036 | Layout and components | No glassmorphism or backdrop-blur sticky nav by reflex, and not the same `rounded-2xl`, `shadow-lg`, `p-6` on everything. | PASS | PASS | No sticky or blurred nav (scanner: no backdrop-blur). One radius, used once. |
| C037 | Layout and components | No "glowing" or gradient hairline borders (1 px light border with a gradient or a top radial glow on cards). | PASS | PASS | No gradient hairlines. |
| C038 | Layout and components | No fake window chrome: traffic-light dots (red, yellow, green), a fake title bar, a fake URL bar or a vertical rotated label. | PASS | PASS | The traffic lights are in the real settings screenshot; nothing is drawn around either screenshot. Documented exception. |
| C039 | Layout and components | No grid or dot-pattern background, noise or grain overlay, or "blueprint" crosshair marks used as a default finish. If texture exists it is part of the identity (the label, the paper), not an overlay. | PASS | PASS | Before: no texture. After: no grid or noise. The captured text and two box-drawing glyphs are real product output, placed once, authorised as a brand exception. |
| C040 | Layout and components | No marquee or infinite-scroll strip of logos, tags or quotes, and no "Trusted by" row of logos that are not real. | PASS | PASS | No marquee or logo row. |
| C041 | Layout and components | No testimonial carousel, and no testimonials with placeholder avatars, "Jane Doe" names or round-number praise. | PASS | PASS | No testimonials. |
| C042 | Layout and components | No three-tier pricing with a ringed "Popular" middle card, unless there really is pricing (rush may have none). | PASS | PASS | No pricing; the product is MIT. |
| C043 | Layout and components | No giant footer link grid duplicating the nav, with the same links in every column. | PASS | PASS | Footer is one licence line and one repository link. |
| C044 | Layout and components | No decorative captions ("A GOOD CONNECTION.", "Design preview"), badges, page counts or extra stats nobody asked for. | FAIL | PASS | Before: hue names ("Violet", "Indigo") under all twelve pins and "so you can check the work" were extra labels nobody asked for. After: pin names only; captions state what is shown. |
| C045 | Layout and components | Consistency: one corner radius (2 or 3 radii in total, and not `rounded-2xl` or `rounded-3xl` or `9999px` pills everywhere), one shadow rule, one spacing scale (8 pt), and no uniform section padding that makes every section the same weight. | PASS | PASS | One radius (crop, 12 px) plus 2 px on focus; no shadows; 8 pt spacing; section padding varies from 1 rem to 17 rem on purpose. |
| C046 | Layout and components | Section rhythm varies: not every section the same heading, subhead, then grid structure. | PASS | PASS | No two neighbouring sections share a structure (DESIGN.md table). |
| C047 | Layout and components | No full-width content stretched to a wide container (centred `max-w-7xl` 12-column grid with everything the same width); use a narrower measure and let some things break the grid. | PASS | PASS | No container; the crop runs off the right edge, the settings window off the left, the background text off both. |
| C048 | Layout and components | Focus states are visible, alt text exists, favicon, title and meta description are real and differ per page. | PASS | PASS | `:focus-visible` 2 px outline in `--mark`; every `img` has alt (0 without, checked); favicon is the brand icon; title and description are specific to this page. |
| C049 | Layout and components | Nothing overflows or overlaps: text fits its buttons and containers at every width, edges align, icons and arrows are optically centred. | FAIL | PASS | Before: at 1440 the Homebrew line wrapped as "terminal-" / "decline" and the cargo line likewise (before/full-light.png, install section). After: flags and the repository name are `white-space: nowrap` spans; wrapping only at spaces; checked at 1440, 768 and 390 (after/desktop-light.png, after/tablet-light.png, after/mobile-light.png). |
| C050 | Dev-tool pages | No terminal-window mockup with traffic-light dots wrapped around invented output. Real rush screens, real captured text, or no mock at all. | PASS | PASS | Two real screenshots from docs/img, one cropped by CSS and captioned as such. No mockup. |
| C051 | Dev-tool pages | No fake or "perfect" demo output: round numbers, tidy timings, a `$` prompt in front of a command the tool does not take, or commands that were not run. | PASS | PASS | Commands are labelled examples with no prompt and no output. Correction recorded: the README's `send sh --text "make test\n"` sends a literal backslash-n (verified by Opus on the installed build); it is not on this page, and any shell example must use `--text "make test" --key enter`. |
| C052 | Dev-tool pages | No typewriter animation typing the terminal output or the headline letter by letter. | PASS | PASS | No typewriter effect. |
| C053 | Dev-tool pages | No hero install line styled as a pill with a copy button and a `npx` or `curl \| sh` that is not the real install route. | PASS | PASS | The install line is plain large mono text, no pill or copy button; it is the real cask route. |
| C054 | Dev-tool pages | No GitHub stars badge, "Open source" badge, or version pill used as decoration, and nothing that implies stats we do not have. | PASS | PASS | No badges or version pills. |
| C055 | Dev-tool pages | No row of `kbd` keycap chips as decoration ("⌘K", "Ctrl+Enter") unless each is a real rush key shown in context. | PASS | PASS | No kbd chips. "Cmd+Tab" appears once in prose. |
| C056 | Dev-tool pages | No "Built for developers", "Built for teams", "Built for speed" or "The AI-powered X for modern Y" mad-lib headline. Say what rush does for whom, in plain words. | PASS | PASS | Before H1: "Give any command its own Mac app." After H1: "26 MB at idle." with the lede naming the product and platform. Neither is a mad-lib. |
| C057 | Dev-tool pages | No "Linear clone" look: near-black canvas, hairline borders, radial glow over a dashboard frame, gradient headline word, glass cards. | PASS | PASS | White ground, no hairline cards, no glow, no gradient word. |
| C058 | Dev-tool pages | No abstract "dashboard" or 3D blob as the hero image in place of the actual product. | PASS | PASS | Hero object is the measurement table (after) or real pin artwork (before), not a dashboard. |
| C059 | Dev-tool pages | No product name in lowercase mono with a blinking cursor as the logo as the only identity. | PASS | PASS | Identity is the outlined wordmark with the Slope; no cursor. |
| C060 | Dev-tool pages | No generic "Get started" or "Learn more" CTA, and no "→" arrow glued to every link and button; labels say the exact action ("Install rush", "Read the config docs"). | PASS | PASS | Links say the action: "Install with Homebrew", "Read the source on GitHub", "How it was measured", "See the whole window", "How pins are built", "Read the agent API". No arrows. |
| C061 | Dev-tool pages | No Aceternity or Magic UI stock effects (spotlight, beams, meteors, shimmer buttons, animated borders) on a page that is not about them. | PASS | PASS | No stock effects. |
| C062 | Dev-tool pages | No privacy or "runs locally" reassurance that was not asked for or cannot be backed by a link to the code. | PASS | PASS | The socket and `agent_api = false` are stated as facts of the API with a link to docs/design/agent-api.md, not as reassurance. |
| C063 | Dev-tool pages | No leftover artefacts: "Vite + React" titles, default platform subdomains, builder badges, stale model names, an old footer year, section-label HTML comments, `Hero`/`Testimonials` class names, old product names in copy. | PASS | PASS | No leftover titles, comments or template class names. The favicon data URI carries the icon SVG's own title text, unchanged brand artwork. |
| C064 | Dev-tool pages | The page names its actual agents, keys, files and commands (real names from the Rust port) rather than category words ("workflow", "agents", "automation"). | PASS | PASS | Real names throughout: `decline pin`, `decline ctl spawn`, `wait-for`, `completion_as_you_type`, `config.toml`, `scripts/compare/run.py`. |
| C065 | Motion | No fade-and-slide-up on every section (`data-aos="fade-up"`, identical staggered reveals), and no motion library reveals as the only motion. | PASS | PASS | No section reveals. Before had a single 300 ms fade-and-settle on the hero pin; removed in review so the only motion is the link underline. |
| C066 | Motion | No `whileHover={{ scale: 1.05 }}` or lift-with-shadow on every card, especially cards that are not clickable. | PASS | PASS | No hover scale or lift. |
| C067 | Motion | No bounce or spring easing for its own sake, no count-up numbers, no auto-typing headlines, no glowing blob that follows the cursor. | PASS | PASS | No springs, count-ups or cursor-following elements. The background corners are still. |
| C068 | Motion | Motion must mean something: a row floating up, a chirp landing, the bottle. Two or three animation types at most, used consistently. | PASS | PASS | One motion type after review (underline thickens on hover, 160 ms). Before: two. |
| C069 | Motion | No scroll-jacking, parallax or carousels, and nothing that alters native scroll speed. | PASS | PASS | Native scroll; no parallax; `overflow-x: clip` only guards the bleeding images. |
| C070 | Motion | No animation that restarts when the cursor moves, and `prefers-reduced-motion` is honoured. | PASS | PASS | `prefers-reduced-motion` disables all animation and transitions. |
| C071 | Motion | Most interactions 150 to 300 ms; none over about 500 ms that feel like lag. | PASS | PASS | 160 ms. |
| C072 | Motion | No pointless hover effects, and no buttons that look clickable but go nowhere. | PASS | PASS | Only links change on hover; every link has a real href. |
| C073 | Motion | No loading text that narrates the build ("Initialising workspace...") in place of a real state. | PASS | PASS | No loading text. |
| C074 | Copy | No adjective-stacked subtitles, "Elevate your...", "Supercharge", "Unleash", "Transform your", "Unlock the power of", "X, reimagined", "Say goodbye to X", or vague SaaS wording ("seamless", "powerful", "robust", "streamline", "cutting-edge", "effortless", "in just a few clicks"). | PASS | PASS | Grep for the banned vocabulary: none. No adjective stacks; "fast" is not used anywhere. |
| C075 | Copy | No "No X. No Y. Just Z." negation list, and no "Less X, more Y" or "Forget X. Focus on Y.". | FAIL | PASS | Before: "There's no account to make and no AI sidebar to close" was a No X / No Y pair. After: "It never asks you to sign in, and nothing in it talks back." |
| C076 | Copy | No "It's not X, it's Y" or "Not because X. Because Y." contrast reveal. | PASS | PASS | No contrast reveal. |
| C077 | Copy | No fragment drumbeat or tricolon headline ("Fast. Simple. Done.", "Build. Ship. Scale.", "Fast, secure and scalable"), and not three of anything by reflex. | PASS | PASS | No tricolons. The H1 is one measured fact. |
| C078 | Copy | No rhetorical question then answer ("The result? Calm."), colon reveal ("The best part: ...") or "Here's the thing" setup. | PASS | PASS | No rhetorical question or colon reveal. "Not zero." is a qualification, not a reveal. |
| C079 | Copy | No wisdom-shaped aphorisms ("Speed is a feature.") or performed cleverness ("Spreadsheets with a pulse."). | PASS | PASS | No aphorisms. The asides are dry ("which is to say an ordinary afternoon"). |
| C080 | Copy | No slogan on every section (one wink per section at most) and no tidy closer ("The future of X starts now."). | PASS | PASS | One wink per section at most; no closer. "Less overhead. More terminal." does not appear. |
| C081 | Copy | Headline says the result or the thing, not the technology, and could not sit unchanged on a competitor's site. | PASS | PASS | "26 MB at idle." is a measured result that could not sit on a competitor's page; the lede says what and for whom. |
| C082 | Copy | No header, eyebrow, H1, description and footer all saying the same thing. | PASS | PASS | Title, H1, lede and footer each say different things; the footer is licence and author. |
| C083 | Copy | No bold label on every bullet ("**Seamless integration:** ...") and no parallel bullet openers ("Ensures... Provides... Enables..."). | PASS | PASS | No bold-label bullets; the dl descriptions are plain phrases with varied openers. |
| C084 | Copy | No synonym cycling: one name for one thing (agent, session, card) everywhere. | PASS | PASS | One name each: pin, session, Decline, rush. |
| C085 | Copy | No invented labels or false ranges ("from solo devs to enterprises"), no weasel sources ("trusted by industry leaders", "join thousands of developers"), no round-number claims. | PASS | PASS | No invented ranges or weasel sources; every number is from the README or docs/comparison-2026-10-30.md. |
| C086 | Copy | No generic FAQ filler or an FAQ with no answers. Answer only real questions. | PASS | PASS | No FAQ. |
| C087 | Copy | No narrated UI labels ("Choose a...") where a plain heading works, and no internal or backend terms leaking into user copy. | FAIL | PASS | Before: public copy carried internals (`term-icon`, "hash of the name", a bundle tree with `Info.plist`, `MacOS/rush`, `rush.pin.toml`, "the stub, copied from the app, then signed"). After: those are gone; the page says what a pin is and that it keeps its permissions across upgrades, with a link to pins.md. Provenance moved to DESIGN.md. |
| C088 | Copy | Specific, product-true sentences in the owner's voice: British English, short, uneven in length. | PASS | PASS | British spelling (colour, licence, notarised), short and uneven sentences, one "quite some". |
| C089 | Copy | Real product evidence: real rush screens (as screenshots or as real text), not mock UI. | PASS | PASS | Two real screenshots, twelve real term-icon renders, real `ctl --help` text in the background. |
| C090 | Copy | Sentence and section rhythm is not uniform: some long, some short, some sections plainly larger than others. | PASS | PASS | Sections range from a three-paragraph conditions block to a full-height pin spread; sentence length varies. |
| C091 | Craft | Squint test: blur the page. It must not read as "dark hero, glow, one centred button, three cards". | PASS | PASS | Blurred, the page reads as a left-heavy spread with one big number and a table, then alternating white and spruce bands. after/full-light.png. |
| C092 | Craft | Side-by-side test: a stranger could tell it apart from a page generated by v0, Lovable, Bolt or Claude with a one-line prompt. | PASS | PASS | Captured help text bleeding off the page, a crop off the right edge, a pixel loupe and a measurement table in the hero are not one-prompt output. |
| C093 | Craft | Swap-the-logo test: replace "rush" with another dev tool's name and the page must stop making sense. | PASS | PASS | Replace "Decline" and the page stops making sense: the figures, the conditions, `decline pin`, the Slope pins, `ctl` are all specific. |
| C094 | Craft | Every colour, font, radius, shadow and easing is a recorded token in `DESIGN.md`, with the reason. Nothing is "whatever the model picked". | PASS | PASS | Every colour, size, radius, opacity and easing is in DESIGN.md with a reason, including the texture opacities and the eyeballed crop backing. |
| C095 | Craft | Design came from a reference or mockup (the bottle, the label, a real terminal capture) first, and was then built. Not prompted from a blank page. | PASS | PASS | Built from the brand specification, the README table and the real assets; the background from the isolated studies Alex chose. |
| C096 | Craft | Screenshot-and-compare loop done: built page checked against the intended design at mobile, tablet and desktop, differences fixed. | FAIL | PASS | Before: DESIGN.md says no screenshots were taken and composition was not checked by eye. After: checked at 1440×900, 768×1024 and a 390 px iframe in light and dark, plus 1440×8600 and 500×7000 full-page captures; four layout fixes came from those checks (command wrapping, mobile table fit, corner glyph beside a link, pin dl column). |
| C097 | Craft | No page drift: font, colour, radius, nav and footer identical across every page unless deliberately changed. | PASS | PASS | One page; nav and footer appear once. |
| C098 | Craft | The scanner pass: grep the output for `rounded-2xl`, `rounded-3xl`, `backdrop-blur`, `bg-clip-text`, `indigo-`, `violet-`, `shadow-lg`, `data-aos`, `whileHover`, `from-purple`, `#6366f1`, `lucide`, and explain each hit. | PASS | PASS | Scanner grep on the final file for `rounded-2xl`, `rounded-3xl`, `backdrop-blur`, `bg-clip-text`, `indigo-`, `violet-`, `shadow-lg`, `data-aos`, `whileHover`, `from-purple`, `#6366f1`, `lucide`: 0 hits. |
| C099 | Craft | Imperfect on purpose in a few places: something hand-set (a custom rule, a drawn mark, an odd crop) that a template would not do. | PASS | PASS | The 16 px pin enlarged with `image-rendering: pixelated`, the crop off the right edge, and after review the help text bleeding off both edges. |
| C100 | Craft | Finish line done: favicon, social card, 404 page, per-page title and description, working links, real alt text, no placeholder content. | FAIL | FAIL | Favicon, title, description, alt text and working links are done. No social card and no 404 page: deferred until a concept is chosen, so this stays FAIL. |

## Reviewer notes

- Documented exceptions, per the brief: SF Pro as the brand face; spruce
  and mint as the brand palette; pin gradients and rounded icons as
  term-icon artwork; the screenshots' own macOS chrome; and, after Alex's
  decision, the captured-output background texture.
- C100 stays FAIL on purpose. A social card and 404 page are production
  items for the Astro build after a concept is chosen.
- The concept's personality survives: left-heavy spread, native type,
  objects bleeding off the page edges, the pixel loupe, no cards. What
  moved is the order and the first object, which is now the measurement.
