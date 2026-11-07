# Screen: anti-slop cross-review

Reviewer: Claude Opus 5.5 (`claude-opus-5-5`). Original author: Claude
Sonnet 5.5 (`claude-sonnet-5-5`). 7 November 2026.

## Evidence

- Before source and captures (immutable, viewed): `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-round-two/screen/before/index.html`,
  `desktop-light.png`, `desktop-dark.png` (1440×900), `full-light.png`,
  `full-dark.png` (1440 wide, page 2430 px tall in a 6000 px capture),
  `phone-light.png` and `phone-dark.png` (390 px iframe in a 500×844 window),
  plus `results.json` (overflow [] and 0 broken images in all six; Chrome
  exited 2 at teardown with valid PNGs).
- `phone-light.png` and `phone-dark.png` are byte-identical. That is
  plausible, because the whole visible phone area is the spruce plate and
  the window, which are the same in both themes. It still means the phone
  capture shows no dark-mode difference.
- After (viewed, eight views): `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-round-two/screen/after/` desktop-light.png, desktop-dark.png,
  phone-light.png, phone-dark.png, full-light.png, full-dark.png,
  tablet-light.png and tablet-dark.png (768×1024), captured by the parent. Its `index.html` snapshot is byte-identical to this
  directory's source. In results.json all eight views report overflow [],
  badAnchors [] and brokenImages 0; page height is 2560 px desktop, 3161 px
  tablet and 4034 px phone. Exit code was 0 for four views and 2 (teardown)
  for phone-dark, full-dark, tablet-light and tablet-dark, all with valid
  PNGs. The after phone PNGs differ between light
  and dark, unlike the before pair.
- Tablet (768, below the 860 px breakpoint) shows header and command on
  one line, title, lead, conditions, the single results pane and then
  "Against Ghostty and kitty" with the table starting, inside 1024 px, in
  both themes. Nothing clips.
- Not viewed: the phone and tablet pages below their first viewport,
  including the narrow-only completion block in Agents (shown below 860 px).
- The inherited tabs (Footprint, Pins, Agents, Install) are page anchors to
  #numbers, #pins, #agents and #install; badAnchors is empty and all links
  resolve.
- I took no screenshots.

## Counts

Before: Pass 79, Fail 16, Exception 4, Deferred 1. After (source plus after captures): Pass 95, Fail 0, Exception 4, Deferred 1.
Exceptions counted explicitly: native brand type and palette (1, 17, 18:
recorded as Pass with the exception noted), terminal chrome (38, 50),
the visible copy command (53) and Alex's settled headline (81). The 13 px
terminal text is required by the brief, and no low-contrast decorative
evidence is used except the aria-hidden window controls. C100 is deferred.
These counts are not a measure of visual merit.

## Items

| # | Checklist item (verbatim) | Before | After | Evidence |
| ---: | --- | --- | --- | --- |
| 1 | No unchosen default fonts: Inter, Geist, Roboto, Arial, Open Sans, Lato or the system stack used only because they were the default. | Pass | Pass | System stack -apple-system / SF Pro, the brand's deliberate native face (docs/brand/README.md). Counted exception: native brand type. |
| 2 | No "tasteful free" fonts used by reflex either: Space Grotesk (the model's favourite once Inter is banned), Instrument Serif, Fraunces, DM Sans, Cabinet Grotesk, Rajdhani, Share Tech Mono. | Pass | Pass | None of the listed fonts appears in the CSS. |
| 3 | No Instrument Serif (or any serif) headline paired with a mono caption, the 2026 "tasteful AI" default. | Pass | Pass | No serif anywhere; captions are SF Pro, not mono. |
| 4 | No serif headline with one italic, accent-coloured word ("Software that *feels* alive"). | Pass | Pass | The H1 is one weight and one colour (mint 800). |
| 5 | No small uppercase, tracked eyebrow or mono caption above every heading ("01 / GET INTO IT", "AN INDEPENDENT APP", "FEATURES"). | Pass | Pass | No eyebrows. The before page had small pane labels in the window but none above the headings; they were removed. |
| 6 | No all-caps, wide-tracked, mono section labels used as the page's chrome (about 42% of builder-made pages). | Pass | Pass | No uppercase or tracked labels. |
| 7 | No monospace face at H1 size (JetBrains Mono, Geist Mono, IBM Plex Mono) as the only way to say "developer". | Pass | Pass | H1 is SF Pro 800; mono appears only at 13 px. |
| 8 | Not lowercase-everything as an affectation: headings and buttons in plain sentence case unless the brand voice truly is lowercase. | Pass | Pass | Sentence case. Lowercase only in the outlined wordmark, which is the brand. |
| 9 | No very light weights (200 to 300) on grey text; weights limited to a few (for example 400, 500, 600). | Fail | Pass | Before: seven weights (400, 500, 600, 650, 700, 750, 800). After: 400, 600 and 800, plus 700 only for bold cells from the capture. |
| 10 | One deliberate type scale with real contrast: 5 or 6 sizes at most, body at least 16 px, line height about 1.6 for body, tight tracking only on big display sizes. | Fail | Pass | Before: about eleven sizes (112/48/32 clamps, 17, 16, 15, 14, 13, 12, 11, plus phone overrides). After: six (H1, h2, lead, 17 body at 1.6, 15 small, 13 terminal/chrome). Body is at least 16 px. |
| 11 | Prose measure no wider than about 65ch, and no full-container-width paragraphs. | Fail | Pass | Before: notes at 72ch. After: notes 65ch, prose 60ch, header note 44ch. |
| 12 | A font declared in CSS must actually load (no silent fallback to system sans). | Pass | Pass | Only system fonts are declared, and nothing is downloaded. On a Mac they render as SF Pro and Menlo; elsewhere they fall back. Documented native choice. |
| 13 | No gradient text (`bg-clip-text text-transparent`) on "the one word that matters", and no one-word accent colour in the headline. | Pass | Pass | No gradient text, and no one-word accent. |
| 14 | No Title Case On Every Heading, button and bullet. | Pass | Pass | Headings and links are in sentence case. |
| 15 | No em-dash-heavy copy, and no decorative middle dots or `//` as separators in headers ("Home // Work", "A · B · C" meta strings). | Pass | Pass | The scanner finds 0 em dashes; there are no middle-dot or // separators in copy. |
| 16 | No purple, violet, indigo or blue gradients, and no Tailwind `indigo-*`, `violet-*`, `#6366f1`, or `--primary` hue 255 to 280. | Pass | Pass | No purple or indigo. Flag blue #81a2be is only on terminal text. |
| 17 | No "tasteful default" cluster: warm cream or parchment ground (`#faf8f5`, `#f4f1ea`, `bg-stone-50`) plus serif plus terracotta (`#d97757` and neighbours), sage or forest green. Any two together is a strong signal. | Pass | Pass | The ground is white (light) or deep spruce (dark). There is no cream and no serif; the greens are sourced brand colours (counted exception: brand palette). |
| 18 | No near-black ground with a single acid-green, vermilion or neon accent as the "dev tool" look (Linear-style `#08090a` canvas with `#23252a` hairlines). | Pass | Pass | Dark mode is deep spruce #051A17 with pastel mint, both brand colours, not an acid accent on #08090a. A real light design exists. Borderline, noted. |
| 19 | No glowing blob, aurora, mesh gradient or radial bloom behind the hero or behind the product frame, and no coloured `shadow-[0_0_*]` glows. | Pass | Pass | The plate is a hard two-tone step (linear-gradient with one stop), not a bloom. The window shadow is tinted and offset downward, not a glow. |
| 20 | No untouched shadcn palette: `components.json` not left on `baseColor: "slate"` with unedited CSS variables, no `rounded-lg border bg-card shadow-sm` on everything, no slate or zinc greys by default. | Pass | Pass | No shadcn; all tokens are custom. |
| 21 | One dominant colour, one accent (the label red or yellow) and one neutral. No palette-cycling cards (red, blue, green, orange), and no emerald or teal fallback. | Pass | Pass | Spruce is dominant, mint the accent, and white or deep spruce the neutral. Terminal colours stay inside terminal surfaces. |
| 22 | No permanent dark mode by reflex. Follow the system, with a real light design. | Pass | Pass | prefers-color-scheme. Both themes were captured before: desktop-light.png and desktop-dark.png differ below the plate. |
| 23 | Grey tints chosen to match the palette, not default grey (also tint shadows, not pure black or grey). | Pass | Pass | Page greys are spruce-tinted (#4B6762, #CFE0D8, #9DBBB0). The window greys are neutral to match the captured #16181c. The shadow is spruce-tinted. |
| 24 | Contrast of at least 4.5:1 everywhere, especially muted text on dark grounds. | Pass | Pass | Computed: muted/white 6.15, link/white 7.57, plate muted 7.71, t-dim 7.10, flag blue 6.64, tab text 7.28, dark muted 8.72, muted on wash 5.59, selected row (white override) above 8. Only the decorative window controls (2.18) and aria-hidden bars are lower. |
| 25 | Not a flat, empty ground: if a texture is used it has a reason (see Layout). | Pass | Pass | The flat ground is deliberate: structure comes from the spruce step. No texture. (Alex's selected terminal-text background is not used; not required by this item.) |
| 26 | No pill or badge above the hero headline ("✨ New", "AI-powered", "v1.0 is here", "Not on the App Store"), and no pulsing or blinking status dot in a corner. | Pass | Pass | Nothing above the H1. |
| 27 | Not a centred hero with a subhead and two buttons, plus a logo strip (about half of builder-made pages centre everything). | Pass | Pass | Left-aligned H1, no button pair, no logo strip. |
| 28 | Not the stock section order: hero, logos, three features, stats, testimonial, pricing, FAQ, CTA band, four-column footer. Order follows the story, not the template. | Pass | Pass | Order: title and window, numbers, pins, agents, install. |
| 29 | No row of three (or four, or six) identical icon-topped feature cards, and no bento grid of uneven icon boxes under a "FEATURES" label. | Pass | Pass | No feature cards or bento. |
| 30 | No icon inside a rounded square tile, no Lucide-only icon set, and no sparkle, lightning bolt or brain icon to signal "AI". | Pass | Pass | The rounded squares are real generated pin artwork, the product's output, not icon tiles. |
| 31 | No emoji as icons, bullets or sidebar items (rocket, bolt, lock, check, sparkles). | Pass | Pass | No emoji. |
| 32 | No check-mark list under the hero buttons ("No credit card, free forever, cancel any time") and no check-mark feature lists in cards. | Pass | Pass | None. |
| 33 | No "1 · 2 · 3" numbered steps, no decorative "01 / 02 / 03" section numbering, no stats row, and no A/B/C lettered lists. | Pass | Pass | No numbered steps. The two lead numbers sit in a sentence with their conditions, not a stats row. |
| 34 | No stat banner with unsourced round numbers ("10M+", "99.9% uptime", "500x faster", "24/7") and no count-up animation. | Pass | Pass | All numbers are dated and sourced (README, 2 November 2026). |
| 35 | No card around every block of content, no nested cards, and no coloured left or top border strip (3 to 4 px) on cards. Group with whitespace, proximity and type instead. | Pass | Pass | Only the window, two command chips and one command block have backgrounds. No nesting, no stripes. |
| 36 | No glassmorphism or backdrop-blur sticky nav by reflex, and not the same `rounded-2xl`, `shadow-lg`, `p-6` on everything. | Pass | Pass | No sticky or blurred nav. |
| 37 | No "glowing" or gradient hairline borders (1 px light border with a gradient or a top radial glow on cards). | Pass | Pass | The window has a 1 px mint border at 28% opacity: flat, not a gradient or glow. |
| 38 | No fake window chrome: traffic-light dots (red, yellow, green), a fake title bar, a fake URL bar or a vertical rotated label. | Exception | Exception | The title bar, three grey window controls and tab strip are fake chrome, explicitly authorised in round-two BRIEF.md. The controls are grey, not red/yellow/green, and aria-hidden. Counted exception: terminal chrome. |
| 39 | No grid or dot-pattern background, noise or grain overlay, or "blueprint" crosshair marks used as a default finish. If texture exists it is part of the identity (the label, the paper), not an overlay. | Pass | Pass | None. |
| 40 | No marquee or infinite-scroll strip of logos, tags or quotes, and no "Trusted by" row of logos that are not real. | Pass | Pass | None. |
| 41 | No testimonial carousel, and no testimonials with placeholder avatars, "Jane Doe" names or round-number praise. | Pass | Pass | None. |
| 42 | No three-tier pricing with a ringed "Popular" middle card, unless there really is pricing (rush may have none). | Pass | Pass | None. |
| 43 | No giant footer link grid duplicating the nav, with the same links in every column. | Pass | Pass | Before: one footer line. After: one line, MIT licence plus a GitHub link. |
| 44 | No decorative captions ("A GOOD CONNECTION.", "Design preview"), badges, page counts or extra stats nobody asked for. | Fail | Pass | Before: the decorative pane labels (Recorded results, Commands, Tab completion). After: removed. The table caption explains the bars and is kept. |
| 45 | Consistency: one corner radius (2 or 3 radii in total, and not `rounded-2xl` or `rounded-3xl` or `9999px` pills everywhere), one shadow rule, one spacing scale (8 pt), and no uniform section padding that makes every section the same weight. | Fail | Pass | Before: radii 3, 6, 8 and 10, and uniform 48 px section padding. After: radii 3, 6 and 10; spacing above the four sections is 48, 88, 72 and 104 px. One shadow. |
| 46 | Section rhythm varies: not every section the same heading, subhead, then grid structure. | Fail | Pass | Before: Pins and Agents were both heading-and-prose left, artefact right. After: Agents is mirrored on desktop and gains the completion block on phones. The numbers section is a table; Install pairs the Slope with text. |
| 47 | No full-width content stretched to a wide container (centred `max-w-7xl` 12-column grid with everything the same width); use a narrower measure and let some things break the grid. | Fail | Pass | Before: the table stretched across the 1232 px container. After: the table is capped at 960 px and the window is the one full-width element; prose has a measure. |
| 48 | Focus states are visible, alt text exists, favicon, title and meta description are real and differ per page. | Fail | Pass | Before: no favicon, and the Copy focus ring was spruce on a spruce plate. After: icon.svg favicon and a mint focus ring. Alt text is on every pin; title and description are real. 404 is under item 100. |
| 49 | Nothing overflows or overlaps: text fits its buttons and containers at every width, edges align, icons and arrows are optically centred. | Pass | Pass | Before: results.json overflow [] in all six before views, but the phone opening ran through all three panes (before/phone-light.png). After: results.json overflow [], badAnchors [] and brokenImages 0 in all eight views. after/tablet-light.png and tablet-dark.png show the same compact opening at 768, with the table starting inside 1024. after/phone-light.png and after/phone-dark.png show header, title, lead, conditions and the single results pane, with "Against Ghostty and kitty" at about y760 of 844. The header command wraps beside Copy without clipping. |
| 50 | No terminal-window mockup with traffic-light dots wrapped around invented output. Real rush screens, real captured text, or no mock at all. | Exception | Exception | The window holds the real terminal-preview.json capture and README commands, with no invented output. The chrome is authorised (item 38). |
| 51 | No fake or "perfect" demo output: round numbers, tidy timings, a `$` prompt in front of a command the tool does not take, or commands that were not run. | Pass | Pass | No $ prompt. The commands come from the README in the verified --key enter form; the timings are real. |
| 52 | No typewriter animation typing the terminal output or the headline letter by letter. | Pass | Pass | No animation. |
| 53 | No hero install line styled as a pill with a copy button and a `npx` or `curl \| sh` that is not the real install route. | Exception | Exception | The header command sits in a 6 px bordered box with a mint Copy button: close to the banned look, but it is the real brew route, and Alex required a visible, copyable command. Counted exception: visible copy command. |
| 54 | No GitHub stars badge, "Open source" badge, or version pill used as decoration, and nothing that implies stats we do not have. | Pass | Pass | None. |
| 55 | No row of `kbd` keycap chips as decoration ("⌘K", "Ctrl+Enter") unless each is a real rush key shown in context. | Pass | Pass | The only key shown is Copy's fallback text, ⌘C, which is a real instruction. |
| 56 | No "Built for developers", "Built for teams", "Built for speed" or "The AI-powered X for modern Y" mad-lib headline. Say what rush does for whom, in plain words. | Pass | Pass | None. |
| 57 | No "Linear clone" look: near-black canvas, hairline borders, radial glow over a dashboard frame, gradient headline word, glass cards. | Pass | Pass | No radial glow, gradient word or glass. Dark mode uses hairlines, but on spruce. |
| 58 | No abstract "dashboard" or 3D blob as the hero image in place of the actual product. | Pass | Pass | The hero image is the product's real output. |
| 59 | No product name in lowercase mono with a blinking cursor as the logo as the only identity. | Pass | Pass | Outlined wordmark with the Slope. |
| 60 | No generic "Get started" or "Learn more" CTA, and no "→" arrow glued to every link and button; labels say the exact action ("Install rush", "Read the config docs"). | Pass | Pass | Labels name the action ("Copy", "Builds, script and full results", "Build from source", "Pin options"). No arrows. |
| 61 | No Aceternity or Magic UI stock effects (spotlight, beams, meteors, shimmer buttons, animated borders) on a page that is not about them. | Pass | Pass | None. |
| 62 | No privacy or "runs locally" reassurance that was not asked for or cannot be backed by a link to the code. | Pass | Pass | None. |
| 63 | No leftover artefacts: "Vite + React" titles, default platform subdomains, builder badges, stale model names, an old footer year, section-label HTML comments, `Hero`/`Testimonials` class names, old product names in copy. | Pass | Pass | No artefacts. The title, meta and link targets exist (CHANGELOG.md and DESIGN.md were checked). |
| 64 | The page names its actual agents, keys, files and commands (real names from the Rust port) rather than category words ("workflow", "agents", "automation"). | Pass | Pass | Names decline pin rush --dock, ctl spawn/send/wait-for/screen/kill, git fetch flags, Ghostty 1.4.0, kitty 0.49.2. |
| 65 | No fade-and-slide-up on every section (`data-aos="fade-up"`, identical staggered reveals), and no motion library reveals as the only motion. | Pass | Pass | No reveals. |
| 66 | No `whileHover={{ scale: 1.05 }}` or lift-with-shadow on every card, especially cards that are not clickable. | Pass | Pass | No lift. Hover changes background only, on links, tabs and Copy. |
| 67 | No bounce or spring easing for its own sake, no count-up numbers, no auto-typing headlines, no glowing blob that follows the cursor. | Pass | Pass | None. |
| 68 | Motion must mean something: a row floating up, a chirp landing, the bottle. Two or three animation types at most, used consistently. | Pass | Pass | The only motion is a 150 ms background change on Copy and tabs. |
| 69 | No scroll-jacking, parallax or carousels, and nothing that alters native scroll speed. | Pass | Pass | Native scroll, no scroll-behavior override. |
| 70 | No animation that restarts when the cursor moves, and `prefers-reduced-motion` is honoured. | Pass | Pass | Transitions only under prefers-reduced-motion: no-preference. |
| 71 | Most interactions 150 to 300 ms; none over about 500 ms that feel like lag. | Pass | Pass | 150 ms. |
| 72 | No pointless hover effects, and no buttons that look clickable but go nowhere. | Pass | Pass | The tabs link to their sections and Copy copies. The before fallback label "Select text" read like an instruction to do something the page had already done; it now says "Press ⌘C". |
| 73 | No loading text that narrates the build ("Initialising workspace...") in place of a real state. | Pass | Pass | None. |
| 74 | No adjective-stacked subtitles, "Elevate your...", "Supercharge", "Unleash", "Transform your", "Unlock the power of", "X, reimagined", "Say goodbye to X", or vague SaaS wording ("seamless", "powerful", "robust", "streamline", "cutting-edge", "effortless", "in just a few clicks"). | Pass | Pass | No SaaS adjectives. |
| 75 | No "No X. No Y. Just Z." negation list, and no "Less X, more Y" or "Forget X. Focus on Y.". | Pass | Pass | None. |
| 76 | No "It's not X, it's Y" or "Not because X. Because Y." contrast reveal. | Pass | Pass | None. |
| 77 | No fragment drumbeat or tricolon headline ("Fast. Simple. Done.", "Build. Ship. Scale.", "Fast, secure and scalable"), and not three of anything by reflex. | Fail | Pass | Before: "Small, and quick to cat."; "Apple Silicon. macOS 14 Sonoma or later. MIT licence."; "Its own icon, windows and settings." After: plain sentences and the heading "Against Ghostty and kitty". |
| 78 | No rhetorical question then answer ("The result? Calm."), colon reveal ("The best part: ...") or "Here's the thing" setup. | Pass | Pass | None. |
| 79 | No wisdom-shaped aphorisms ("Speed is a feature.") or performed cleverness ("Spreadsheets with a pulse."). | Fail | Pass | Before: "Small, and quick to cat." is performed cleverness (parent: forced copy). After: removed. |
| 80 | No slogan on every section (one wink per section at most) and no tidy closer ("The future of X starts now."). | Pass | Pass | No slogans, no closer. |
| 81 | Headline says the result or the thing, not the technology, and could not sit unchanged on a competitor's site. | Exception | Exception | "A terminal for macOS." on its own could sit on a competitor's page. Alex settled it, and the measured line under it makes the opening specific. |
| 82 | No header, eyebrow, H1, description and footer all saying the same thing. | Fail | Pass | Before: the footer "Terminal Decline. A terminal for macOS." repeated the title, H1 and meta. After: "MIT licence. Source on GitHub". |
| 83 | No bold label on every bullet ("**Seamless integration:** ...") and no parallel bullet openers ("Ensures... Provides... Enables..."). | Pass | Pass | No bullets. |
| 84 | No synonym cycling: one name for one thing (agent, session, card) everywhere. | Pass | Pass | "pin", "ctl", "Decline" and "completion" are used consistently. The tab name Footprint leads to the measurements section. |
| 85 | No invented labels or false ranges ("from solo devs to enterprises"), no weasel sources ("trusted by industry leaders", "join thousands of developers"), no round-number claims. | Fail | Pass | Before: "Completion, output colouring and copy on select are opt-in" was false (README line 25: completion as you type is opt-in, Tab is not), and "Idle CPU is a percentage of one core" had no source. After: corrected and removed respectively. |
| 86 | No generic FAQ filler or an FAQ with no answers. Answer only real questions. | Pass | Pass | No FAQ. |
| 87 | No narrated UI labels ("Choose a...") where a plain heading works, and no internal or backend terms leaking into user copy. | Pass | Pass | No narrated labels. |
| 88 | Specific, product-true sentences in the owner's voice: British English, short, uneven in length. | Fail | Pass | Before: "Ghostty and kitty beside it", plus fragments. After: British, short, sourced sentences. Judgement call. |
| 89 | Real product evidence: real rush screens (as screenshots or as real text), not mock UI. | Pass | Pass | The capture, the screenshot transcription and real pin artwork. |
| 90 | Sentence and section rhythm is not uniform: some long, some short, some sections plainly larger than others. | Fail | Pass | Before: Pins and Agents had identical structure and every section had the same padding. After: varied (see items 45 and 46). |
| 91 | Squint test: blur the page. It must not read as "dark hero, glow, one centred button, three cards". | Pass | Pass | Squint on after/desktop-light.png: a large mint title on spruce, then a wide dark window across the colour step into white, then an even white page. No glow or centred button. after/desktop-dark.png has a weaker step (spruce into deep spruce), so it reads closer to "dark hero plus dark product frame". Pass, but marginal in dark. |
| 92 | Side-by-side test: a stranger could tell it apart from a page generated by v0, Lovable, Bolt or Claude with a one-line prompt. | Pass | Pass | The straddling window with tabs as page navigation and the heavy mint title are not one-prompt defaults. Moderate distinctness; see the judgement below. |
| 93 | Swap-the-logo test: replace "rush" with another dev tool's name and the page must stop making sense. | Pass | Pass | Pins, ctl, the measurements against named terminals and the Slope make no sense for another product. |
| 94 | Every colour, font, radius, shadow and easing is a recorded token in `DESIGN.md`, with the reason. Nothing is "whatever the model picked". | Fail | Pass | Before: chrome greys, #EAF7EF, #9CC4B3, #0B5F52, hover colour and easing were not recorded. After: recorded in DESIGN.md "Tokens". |
| 95 | Design came from a reference or mockup (the bottle, the label, a real terminal capture) first, and was then built. Not prompted from a blank page. | Pass | Pass | Built from the real capture and the window metaphor. |
| 96 | Screenshot-and-compare loop done: built page checked against the intended design at mobile, tablet and desktop, differences fixed. | Fail | Pass | After captures from the current source (after/index.html is byte-identical to the working copy) were viewed: desktop, tablet, phone and full, in light and dark (eight views). The phone and tablet captures show only the first viewport, so the narrow-only completion block in Agents has not been seen rendered. |
| 97 | No page drift: font, colour, radius, nav and footer identical across every page unless deliberately changed. | Pass | Pass | Single page. |
| 98 | The scanner pass: grep the output for `rounded-2xl`, `rounded-3xl`, `backdrop-blur`, `bg-clip-text`, `indigo-`, `violet-`, `shadow-lg`, `data-aos`, `whileHover`, `from-purple`, `#6366f1`, `lucide`, and explain each hit. | Pass | Pass | Scanner: rounded-2xl, rounded-3xl, backdrop-blur, bg-clip-text, indigo-, violet-, shadow-lg, data-aos, whileHover, from-purple, #6366f1 and lucide all 0. The // hits are GitHub URLs and the SVG xmlns only. |
| 99 | Imperfect on purpose in a few places: something hand-set (a custom rule, a drawn mark, an odd crop) that a template would not do. | Pass | Pass | Hand-set: the window straddles the plate step, and its tabs are the page navigation. |
| 100 | Finish line done: favicon, social card, 404 page, per-page title and description, working links, real alt text, no placeholder content. | Deferred | Deferred | Favicon added after review. No social card or 404: deferred for a concept, not a pass. |

## Source changes made in review

1. Phone opening: below 860 px the window keeps only the recorded results
   pane. The commands pane is hidden, since the same commands are in Pins
   and Agents. Completion moves to a standalone block in Agents, shown only
   below 860 px, so the phone's first view is title, numbers and evidence.
   Estimated from the CSS (header ~110, title ~115, lead and conditions
   ~130, window ~385 px) at about 740 px, inside 844. Confirmed in
   after/phone-light.png: the next heading starts at about y760.
2. Copy: as itemised in items 77, 79, 82, 85 and 88. The heading is now
   "Against Ghostty and kitty".
3. Type: 3 weights (+ capture bold), 6 sizes. Pane labels removed.
4. Layout: table capped at 960 px; Agents mirrored on desktop; section
   spacing varied; radii reduced to 3, 6 and 10.
5. Access: favicon, mint focus ring on Copy, "Press ⌘C" fallback.
6. DESIGN.md: tokens, phone behaviour and the review changes recorded.

The header install command is unchanged: always visible, copyable, and
wraps beside Copy on phones.

## Visual judgement (after captures)

Screen has a recognisable idea, and the after captures keep it. One wide,
native-looking window straddles the edge of a spruce plate, and its tabs
are the page navigation. At 1440 the opening is the strongest part of this
round's work I have seen: the heavy mint title fills the width, the
measured line and its conditions sit directly under it, and the window holds
real evidence (the capture, real commands, the transcribed completion list).
It says what the product is and why to try it in one screen, and it is
clearly not the production layout. On phones the opening is now compact
and useful: title, numbers, conditions and the full results pane, with the
next heading at about y760. At 768 the same single-pane window leaves
the right half of a 720 px window empty and dark (after/tablet-light.png):
not clipped, but it looks under-filled.

The limits are below the window. The full captures show an orderly but
ordinary product page: a table, a pins row, a mirrored commands block, then
a large plain Slope beside the install text. The fixes stopped it repeating
itself, but the lower page has no idea of its own; the window metaphor is
not carried down. In dark mode the spruce-to-deep step is faint, so the
straddle that defines the concept almost disappears (after/desktop-dark.png).
In light mode the mint Decline bars are paler than the competitors' spruce
bars, so the winning row looks weakest at a glance.

**Weakest point:** everything distinctive happens in the first screen. Below
the window it is a tidy, generic page, and on phones the window is reduced
to one pane under four tabs. Two of those tabs, Pins and Agents, point to
content the phone window no longer shows. They work as anchors, but the
metaphor thins exactly on the device Alex is likely to check.
