# Space cross-review

Reviewer: Claude Sonnet 5.5 (`claude-sonnet-5-5`). Original author: Claude Fable 5.1 (`claude-fable-5-1`). 7 November 2026. Protocol: `../REVIEW.md`.

Before evidence (immutable, viewed): `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-round-two/space/before/`: `desktop-light.png`, `desktop-dark.png`, `phone-light.png`, `phone-dark.png`, `full-light.png`, `results.json`. I did not view `full-dark.png`. The phone PNGs show only the first 844px, so lower content was audited from source.

After evidence (viewed, all eight PNGs): `/Users/afr/.config/rush/tmp/terminal-decline-96189d59/ee086a9a/decline-round-two/space/after/` holds `desktop-light.png`, `desktop-dark.png`, `tablet-light.png`, `tablet-dark.png`, `phone-light.png`, `phone-dark.png`, `full-light.png`, `full-dark.png`, `results.json` and the source snapshot, which is byte-identical to `index.html` as audited. Parent also ran a clipboard check (`copy/results.json`: exact Homebrew command, selectable pre text); I did not see that file. tablet-light exited 2 at teardown with PNG and metrics present. Limits: the phone PNGs show only the first 844px of about 5380px; full-*.png are 1440 wide, downscaled when viewed, so small text there is legible only at the opening. No tablet before capture exists.

Verdicts: PASS, FAIL, EXCEPTION (documented, counted separately, not a pass), DEFERRED (C100, not a pass).

## Tally

| | Before | After (captured) |
| --- | --- | --- |
| PASS | 91 | 95 |
| FAIL | 4 | 0 |
| EXCEPTION | 4 | 4 |
| DEFERRED | 1 | 1 |

Exceptions: C001 native brand type, C038 and C050 terminal chrome, C081 plain product-description headline (Alex settled it). Counts after captures. Terminal text at 13px/1.3 is carved out inside C010. The tally counts items; it is not visual merit.

## Changes made

- Phone overflow (555px at 390px): fixed at the cause. Single-column grids used `1fr`, whose minimum is the content, so `pre` lines widened the track. They are now `minmax(0,1fr)` with `min-width:0` items; code blocks scroll in their own box. No `overflow:hidden` on body.
- Key line under the title ("0.58 s to cat 100 MB, 26 MB idle. M1 Max, 2 November 2026.") so the cat time is in the first phone view, before the drawing.
- GitHub link shares the wordmark row below 860px.
- Removed "written in Rust and drawn with Metal" from the lead and "written in Rust" from the meta description. The solid-square explanation moved to the caption.
- Removed CSS section-label comments; provenance stays in DESIGN.md. Method, conditions and the earlier Ghostty win stay in "How it was measured".
- Spacing moved onto an 8px scale (C045). Terminal content, including completion descriptions on phones and the command blocks, is 13px/1.3 (they were 12px and 14px/1.65).
- Type sizes collapsed to 12, 13, 14, 16, 20, 28 and the title. Two caption phrases tightened.
- Not changed: the area drawing, real data, Copy, terminal text and system light/dark.

## The 100 items

| ID | Item (verbatim) | Before | After | Evidence |
| --- | --- | --- | --- | --- |
| C001 | No unchosen default fonts: Inter, Geist, Roboto, Arial, Open Sans, Lato or the system stack used only because they were the default. | EXCEPTION | EXCEPTION | Mac system stack (SF Pro) and Menlo/SF Mono. Brand choice per brief and DESIGN.md Tokens. |
| C002 | No "tasteful free" fonts used by reflex either: Space Grotesk (the model's favourite once Inter is banned), Instrument Serif, Fraunces, DM Sans, Cabinet Grotesk, Rajdhani, Share Tech Mono. | PASS | PASS | No such fonts; only system stacks. |
| C003 | No Instrument Serif (or any serif) headline paired with a mono caption, the 2026 "tasteful AI" default. | PASS | PASS | No serif anywhere. |
| C004 | No serif headline with one italic, accent-coloured word ("Software that *feels* alive"). | PASS | PASS | No serif or italic; title is one colour. |
| C005 | No small uppercase, tracked eyebrow or mono caption above every heading ("01 / GET INTO IT", "AN INDEPENDENT APP", "FEATURES"). | PASS | PASS | No eyebrow above any heading (before/full-light.png). |
| C006 | No all-caps, wide-tracked, mono section labels used as the page's chrome (about 42% of builder-made pages). | PASS | PASS | Only chrome label is the sentence-case title bar text. |
| C007 | No monospace face at H1 size (JetBrains Mono, Geist Mono, IBM Plex Mono) as the only way to say "developer". | PASS | PASS | H1 is SF Pro 800; mono only for commands. |
| C008 | Not lowercase-everything as an affectation: headings and buttons in plain sentence case unless the brand voice truly is lowercase. | PASS | PASS | Sentence case. Lowercase "decline" is the outlined wordmark. |
| C009 | No very light weights (200 to 300) on grey text; weights limited to a few (for example 400, 500, 600). | PASS | PASS | Weights 400, 500, 600, 800 (title only); muted text is 400. |
| C010 | One deliberate type scale with real contrast: 5 or 6 sizes at most, body at least 16 px, line height about 1.6 for body, tight tracking only on big display sizes. | FAIL | PASS | Before: about ten distinct sizes (11 to 18px plus 20, 28, title). After: 12 (chrome), 13 (terminal), 14, 16, 20, 28, title. Body 16px/1.5. Captions and Copy are 14px, a judgement call against "body at least 16"; terminal 13px/1.3 is a documented exception. |
| C011 | Prose measure no wider than about 65ch, and no full-container-width paragraphs. | PASS | PASS | Prose max 58ch, key line 40ch, captions 42ch. |
| C012 | A font declared in CSS must actually load (no silent fallback to system sans). | PASS | PASS | No webfont declared; system faces resolve on macOS. |
| C013 | No gradient text (`bg-clip-text text-transparent`) on "the one word that matters", and no one-word accent colour in the headline. | PASS | PASS | No gradient text. |
| C014 | No Title Case On Every Heading, button and bullet. | PASS | PASS | Headings sentence case; "Terminal Decline" is the product name. |
| C015 | No em-dash-heavy copy, and no decorative middle dots or `//` as separators in headers ("Home // Work", "A · B · C" meta strings). | PASS | PASS | grep finds no em dash, middle dot or `//` in the page. |
| C016 | No purple, violet, indigo or blue gradients, and no Tailwind `indigo-*`, `violet-*`, `#6366f1`, or `--primary` hue 255 to 280. | PASS | PASS | Only gradient is the brand teal pin artwork. |
| C017 | No "tasteful default" cluster: warm cream or parchment ground (`#faf8f5`, `#f4f1ea`, `bg-stone-50`) plus serif plus terracotta (`#d97757` and neighbours), sage or forest green. Any two together is a strong signal. | PASS | PASS | White or spruce ground, no cream, no serif, no terracotta. Spruce and mint are brand colours. |
| C018 | No near-black ground with a single acid-green, vermilion or neon accent as the "dev tool" look (Linear-style `#08090a` canvas with `#23252a` hairlines). | PASS | PASS | Dark mode ground is spruce #092D2B, not near-black. The #16181c terminal is the real captured theme. |
| C019 | No glowing blob, aurora, mesh gradient or radial bloom behind the hero or behind the product frame, and no coloured `shadow-[0_0_*]` glows. | PASS | PASS | No glow or blob; one tinted shadow on terminal frames. |
| C020 | No untouched shadcn palette: `components.json` not left on `baseColor: "slate"` with unedited CSS variables, no `rounded-lg border bg-card shadow-sm` on everything, no slate or zinc greys by default. | PASS | PASS | No shadcn, no components.json. |
| C021 | One dominant colour, one accent (the label red or yellow) and one neutral. No palette-cycling cards (red, blue, green, orange), and no emerald or teal fallback. | PASS | PASS | Spruce dominant, mint accent, white/grey-green neutral. |
| C022 | No permanent dark mode by reflex. Follow the system, with a real light design. | PASS | PASS | `prefers-color-scheme` with a real light design. |
| C023 | Grey tints chosen to match the palette, not default grey (also tint shadows, not pure black or grey). | PASS | PASS | Muted greys are spruce-tinted (#47625E, #B4CFC3); shadow tinted. Window-control greys are neutral chrome. |
| C024 | Contrast of at least 4.5:1 everywhere, especially muted text on dark grounds. | PASS | PASS | Muted #47625E on white and #B4CFC3 on spruce are both above 4.5:1; terminal title grey on #26282d is about 5.6:1. Window controls are decorative. |
| C025 | Not a flat, empty ground: if a texture is used it has a reason (see Layout). | PASS | PASS | No texture used; the scale drawing is the identity. |
| C026 | No pill or badge above the hero headline ("✨ New", "AI-powered", "v1.0 is here", "Not on the App Store"), and no pulsing or blinking status dot in a corner. | PASS | PASS | No pill or badge above the title. |
| C027 | Not a centred hero with a subhead and two buttons, plus a logo strip (about half of builder-made pages centre everything). | PASS | PASS | Left-aligned title, drawing and spec list; no logo strip. |
| C028 | Not the stock section order: hero, logos, three features, stats, testimonial, pricing, FAQ, CTA band, four-column footer. Order follows the story, not the template. | PASS | PASS | Order follows the story: figures, method, pins, ctl, completion, install. |
| C029 | No row of three (or four, or six) identical icon-topped feature cards, and no bento grid of uneven icon boxes under a "FEATURES" label. | PASS | PASS | No icon cards; each section has a different split. |
| C030 | No icon inside a rounded square tile, no Lucide-only icon set, and no sparkle, lightning bolt or brain icon to signal "AI". | PASS | PASS | No icon tiles. The rounded pin is real product artwork. |
| C031 | No emoji as icons, bullets or sidebar items (rocket, bolt, lock, check, sparkles). | PASS | PASS | No emoji. |
| C032 | No check-mark list under the hero buttons ("No credit card, free forever, cancel any time") and no check-mark feature lists in cards. | PASS | PASS | No check-mark list. |
| C033 | No "1 · 2 · 3" numbered steps, no decorative "01 / 02 / 03" section numbering, no stats row, and no A/B/C lettered lists. | PASS | PASS | No numbering or lettering. The spec list is labelled, dated rows, not a counter row; DESIGN.md names it as a weakness. |
| C034 | No stat banner with unsourced round numbers ("10M+", "99.9% uptime", "500x faster", "24/7") and no count-up animation. | PASS | PASS | Every figure is dated and sourced to the 2 November README table; no count-up. |
| C035 | No card around every block of content, no nested cards, and no coloured left or top border strip (3 to 4 px) on cards. Group with whitespace, proximity and type instead. | PASS | PASS | No cards. Terminal frames are evidence, not content cards. |
| C036 | No glassmorphism or backdrop-blur sticky nav by reflex, and not the same `rounded-2xl`, `shadow-lg`, `p-6` on everything. | PASS | PASS | Header is not sticky or blurred. |
| C037 | No "glowing" or gradient hairline borders (1 px light border with a gradient or a top radial glow on cards). | PASS | PASS | Plain 1px borders. |
| C038 | No fake window chrome: traffic-light dots (red, yellow, green), a fake title bar, a fake URL bar or a vertical rotated label. | EXCEPTION | EXCEPTION | Mac-style title bar and grey (unfocused) window controls on the two terminal views. Authorised by Alex (ANTI-SLOP, Requested terminal chrome); aria-hidden, not traffic-light colours. |
| C039 | No grid or dot-pattern background, noise or grain overlay, or "blueprint" crosshair marks used as a default finish. If texture exists it is part of the identity (the label, the paper), not an overlay. | PASS | PASS | No grid, dot, noise or crosshair texture. |
| C040 | No marquee or infinite-scroll strip of logos, tags or quotes, and no "Trusted by" row of logos that are not real. | PASS | PASS | None. |
| C041 | No testimonial carousel, and no testimonials with placeholder avatars, "Jane Doe" names or round-number praise. | PASS | PASS | None. |
| C042 | No three-tier pricing with a ringed "Popular" middle card, unless there really is pricing (rush may have none). | PASS | PASS | No pricing; the app is MIT. |
| C043 | No giant footer link grid duplicating the nav, with the same links in every column. | PASS | PASS | One-line footer with the wordmark and licence. |
| C044 | No decorative captions ("A GOOD CONNECTION.", "Design preview"), badges, page counts or extra stats nobody asked for. | PASS | PASS | Captions state method and provenance; no decorative captions or page counts. |
| C045 | Consistency: one corner radius (2 or 3 radii in total, and not `rounded-2xl` or `rounded-3xl` or `9999px` pills everywhere), one shadow rule, one spacing scale (8 pt), and no uniform section padding that makes every section the same weight. | FAIL | PASS | Radii 12, 6 and circle dots; one shadow rule. Before: spacing values of 9, 10, 12, 14, 18, 20, 28, 44px. After: layout gaps, margins and padding are multiples of 8px; 4px only for tight terminal rows and the Decline value, 12px only for Copy padding (documented in DESIGN.md). Square ratios unchanged. After captures: gaps between header, title, key line, drawing, sections and terminal frames look even and on one rhythm (desktop-light.png, tablet-light.png, phone-light.png). |
| C046 | Section rhythm varies: not every section the same heading, subhead, then grid structure. | PASS | PASS | Sections use four different splits and vary in size. |
| C047 | No full-width content stretched to a wide container (centred `max-w-7xl` 12-column grid with everything the same width); use a narrower measure and let some things break the grid. | PASS | PASS | 1200px wrap but content uses 440px, 58ch and varied columns; the drawing breaks the grid. |
| C048 | Focus states are visible, alt text exists, favicon, title and meta description are real and differ per page. | PASS | PASS | `:focus-visible` outline, alt on all dock images, pin SVG labelled, favicon, real title and description. Single page, so "differ per page" is not testable. |
| C049 | Nothing overflows or overlaps: text fits its buttons and containers at every width, edges align, icons and arrows are optically centred. | FAIL | PASS | Before: results.json phone pageWidth 555 at 390px, overflow in pins, agents and pre blocks. After: results.json pageWidth equals viewport at 1440, 768 and 390 in light and dark, empty `overflow` lists, 0 broken images, 0 bad anchors. phone-*.png and tablet-*.png show no cropped text or horizontal scroll in the visible part. The grey strip right of the phone PNGs is the 500px capture canvas, not page overflow (pageWidth 390). Lower phone content is covered by the metrics only, not by an image. |
| C050 | No terminal-window mockup with traffic-light dots wrapped around invented output. Real rush screens, real captured text, or no mock at all. | EXCEPTION | EXCEPTION | Dots wrap real captured text (terminal-preview.json, completion-preview.json), not invented output. Same chrome exception as C038. |
| C051 | No fake or "perfect" demo output: round numbers, tidy timings, a `$` prompt in front of a command the tool does not take, or commands that were not run. | PASS | PASS | Captured values and dates preserved; ctl commands are labelled documentation examples with no output and no `$`. |
| C052 | No typewriter animation typing the terminal output or the headline letter by letter. | PASS | PASS | No animation of output or headline. |
| C053 | No hero install line styled as a pill with a copy button and a `npx` or `curl \| sh` that is not the real install route. | PASS | PASS | Real brew cask command as plain text with a Copy button, not a pill, not npx or curl. Visible and copyable is Alex's requirement. |
| C054 | No GitHub stars badge, "Open source" badge, or version pill used as decoration, and nothing that implies stats we do not have. | PASS | PASS | No badges. |
| C055 | No row of `kbd` keycap chips as decoration ("⌘K", "Ctrl+Enter") unless each is a real rush key shown in context. | PASS | PASS | No keycap chips. |
| C056 | No "Built for developers", "Built for teams", "Built for speed" or "The AI-powered X for modern Y" mad-lib headline. Say what rush does for whom, in plain words. | PASS | PASS | "A terminal for macOS." names the thing. Not a mad-lib. |
| C057 | No "Linear clone" look: near-black canvas, hairline borders, radial glow over a dashboard frame, gradient headline word, glass cards. | PASS | PASS | Not a Linear look: no glow, glass or near-black canvas. |
| C058 | No abstract "dashboard" or 3D blob as the hero image in place of the actual product. | PASS | PASS | Hero is the data drawing plus real terminal text; no blob. |
| C059 | No product name in lowercase mono with a blinking cursor as the logo as the only identity. | PASS | PASS | Outlined wordmark with the exact Slope. |
| C060 | No generic "Get started" or "Learn more" CTA, and no "→" arrow glued to every link and button; labels say the exact action ("Install rush", "Read the config docs"). | PASS | PASS | Labels are Copy, GitHub, How it was measured, Pin a command to the Dock. No arrows. |
| C061 | No Aceternity or Magic UI stock effects (spotlight, beams, meteors, shimmer buttons, animated borders) on a page that is not about them. | PASS | PASS | None. |
| C062 | No privacy or "runs locally" reassurance that was not asked for or cannot be backed by a link to the code. | PASS | PASS | No privacy claim. |
| C063 | No leftover artefacts: "Vite + React" titles, default platform subdomains, builder badges, stale model names, an old footer year, section-label HTML comments, `Hero`/`Testimonials` class names, old product names in copy. | PASS | PASS | No leftovers; before had CSS section comments, now removed. No HTML comments. |
| C064 | The page names its actual agents, keys, files and commands (real names from the Rust port) rather than category words ("workflow", "agents", "automation"). | PASS | PASS | Names `decline pin`, `decline ctl`, `footprint`, scripts/compare/run.py, real flags. |
| C065 | No fade-and-slide-up on every section (`data-aos="fade-up"`, identical staggered reveals), and no motion library reveals as the only motion. | PASS | PASS | No reveals. |
| C066 | No `whileHover={{ scale: 1.05 }}` or lift-with-shadow on every card, especially cards that are not clickable. | PASS | PASS | No lift or scale effects. |
| C067 | No bounce or spring easing for its own sake, no count-up numbers, no auto-typing headlines, no glowing blob that follows the cursor. | PASS | PASS | No bounce, count-up or typing. |
| C068 | Motion must mean something: a row floating up, a chirp landing, the bottle. Two or three animation types at most, used consistently. | PASS | PASS | Only motion is the Copy button colour change, which confirms the action. |
| C069 | No scroll-jacking, parallax or carousels, and nothing that alters native scroll speed. | PASS | PASS | Native scroll only. |
| C070 | No animation that restarts when the cursor moves, and `prefers-reduced-motion` is honoured. | PASS | PASS | Transition is inside `prefers-reduced-motion: no-preference`; nothing restarts. |
| C071 | Most interactions 150 to 300 ms; none over about 500 ms that feel like lag. | PASS | PASS | 150ms. |
| C072 | No pointless hover effects, and no buttons that look clickable but go nowhere. | PASS | PASS | Hover only on links and Copy; Copy writes to the clipboard. |
| C073 | No loading text that narrates the build ("Initialising workspace...") in place of a real state. | PASS | PASS | No loading text. |
| C074 | No adjective-stacked subtitles, "Elevate your...", "Supercharge", "Unleash", "Transform your", "Unlock the power of", "X, reimagined", "Say goodbye to X", or vague SaaS wording ("seamless", "powerful", "robust", "streamline", "cutting-edge", "effortless", "in just a few clicks"). | PASS | PASS | Before: lead said "written in Rust and drawn with Metal" (implementation narration, not adjective stacking, so still a pass here). Removed after. No SaaS wording either way. |
| C075 | No "No X. No Y. Just Z." negation list, and no "Less X, more Y" or "Forget X. Focus on Y.". | PASS | PASS | No negation list. |
| C076 | No "It's not X, it's Y" or "Not because X. Because Y." contrast reveal. | PASS | PASS | No contrast reveal. "nothing runs live" is a plain limit. |
| C077 | No fragment drumbeat or tricolon headline ("Fast. Simple. Done.", "Build. Ship. Scale.", "Fast, secure and scalable"), and not three of anything by reflex. | PASS | PASS | No drumbeat. The key line is two facts. The Also row lists three real sections; judged not reflexive. |
| C078 | No rhetorical question then answer ("The result? Calm."), colon reveal ("The best part: ...") or "Here's the thing" setup. | PASS | PASS | No rhetorical devices. |
| C079 | No wisdom-shaped aphorisms ("Speed is a feature.") or performed cleverness ("Spreadsheets with a pulse."). | PASS | PASS | No aphorisms. |
| C080 | No slogan on every section (one wink per section at most) and no tidy closer ("The future of X starts now."). | PASS | PASS | No slogans or closers. |
| C081 | Headline says the result or the thing, not the technology, and could not sit unchanged on a competitor's site. | EXCEPTION | EXCEPTION | "A terminal for macOS." is a plain product description that could sit on a competitor's page. Alex explicitly settled it ("say what it is immediately"), so counted as an exception, not a fail. The distinguishing idea is the drawing and numbers under it. |
| C082 | No header, eyebrow, H1, description and footer all saying the same thing. | PASS | PASS | Header, title, key line, meta and footer say different things (before: lead repeated title's product claim with implementation). |
| C083 | No bold label on every bullet ("**Seamless integration:** ...") and no parallel bullet openers ("Ensures... Provides... Enables..."). | PASS | PASS | No bold-label bullets. The spec list is a definition list of measurements. |
| C084 | No synonym cycling: one name for one thing (agent, session, card) everywhere. | PASS | PASS | "terminal", "agents", "pin" used consistently; "session" appears only in the ctl example comment. |
| C085 | No invented labels or false ranges ("from solo devs to enterprises"), no weasel sources ("trusted by industry leaders", "join thousands of developers"), no round-number claims. | PASS | PASS | No ranges or weasel sources; method named. |
| C086 | No generic FAQ filler or an FAQ with no answers. Answer only real questions. | PASS | PASS | No FAQ. |
| C087 | No narrated UI labels ("Choose a...") where a plain heading works, and no internal or backend terms leaking into user copy. | PASS | PASS | Plain headings. |
| C088 | Specific, product-true sentences in the owner's voice: British English, short, uneven in length. | PASS | PASS | British, short, uneven; "licence", "colour". |
| C089 | Real product evidence: real rush screens (as screenshots or as real text), not mock UI. | PASS | PASS | Real captured terminal text and a real screenshot transcription; no screenshots of terminals. |
| C090 | Sentence and section rhythm is not uniform: some long, some short, some sections plainly larger than others. | PASS | PASS | Sections differ in length and shape. |
| C091 | Squint test: blur the page. It must not read as "dark hero, glow, one centred button, three cards". | PASS | PASS | after/desktop-light.png and full-light.png: white page with one large spruce block and a left-hand drawing; not a dark hero, glow, centred button or three cards. Dark variants read the same with a mint block. |
| C092 | Side-by-side test: a stranger could tell it apart from a page generated by v0, Lovable, Bolt or Claude with a one-line prompt. | PASS | PASS | The nested squares with areas 26/60/99 MB, drawn bottom-left with the solid one, are not a layout a one-line prompt typically produces. Judgement, from after/desktop-light.png and desktop-dark.png. |
| C093 | Swap-the-logo test: replace "rush" with another dev tool's name and the page must stop making sense. | PASS | PASS | Swap Decline for another terminal and the figures, versions, brew cask, pins and ctl all stop making sense. |
| C094 | Every colour, font, radius, shadow and easing is a recorded token in `DESIGN.md`, with the reason. Nothing is "whatever the model picked". | PASS | PASS | Tokens and reasons in DESIGN.md; updated for sizes and key line. |
| C095 | Design came from a reference or mockup (the bottle, the label, a real terminal capture) first, and was then built. Not prompted from a blank page. | PASS | PASS | Started from real data (comparison.json areas) and real captures. |
| C096 | Screenshot-and-compare loop done: built page checked against the intended design at mobile, tablet and desktop, differences fixed. | FAIL | PASS | Before captures (desktop, phone) caught the overflow and the missing cat time; fixes made; after captures taken at 1440, 768 and 390 in light and dark plus tall views and viewed by me. No tablet before capture exists, so there is no tablet before/after comparison. tablet-light exited 2 at teardown, with PNG and metrics present. |
| C097 | No page drift: font, colour, radius, nav and footer identical across every page unless deliberately changed. | PASS | PASS | Single page; header and footer use the same wordmark and tokens. |
| C098 | The scanner pass: grep the output for `rounded-2xl`, `rounded-3xl`, `backdrop-blur`, `bg-clip-text`, `indigo-`, `violet-`, `shadow-lg`, `data-aos`, `whileHover`, `from-purple`, `#6366f1`, `lucide`, and explain each hit. | PASS | PASS | Scanner terms grepped in the final file: 0 hits. No `backdrop-filter`, gradients only inside pin art. |
| C099 | Imperfect on purpose in a few places: something hand-set (a custom rule, a drawn mark, an odd crop) that a template would not do. | PASS | PASS | The area-scaled squares are hand-set from computed square roots (77.85%, 51.25%), not a template. |
| C100 | Finish line done: favicon, social card, 404 page, per-page title and description, working links, real alt text, no placeholder content. | DEFERRED | DEFERRED | Favicon (data URI), title and description exist. No social card or 404 page; concept only, deferred, not a pass. |

## Visual merit (not scored)

The nested squares are a specific idea that Alex can recognise: the page is mostly one drawing of room taken, filled for Decline, and it says what the product is and shows 26 MB at a glance. That is a real gain over two columns and a green band. The phone view before was weak: broken width and the cat time absent. The weakest point is that the rest of the page, below the drawing, is conventional: a left/right text and terminal rhythm with plain lists, and only idle memory is drawn, so speed (the main claim) is text. The headline is the plain description Alex settled, so the drawing carries all the character. Seen in the after captures: the drawing, spec list and key line fit the first desktop view with the method heading and terminal starting below, and the phone opens with title, cat time and the drawing. One small flaw: the key line breaks inside the date ("2 / November 2026") at desktop and tablet widths because of its 40ch limit. The spec list leaves a blank area under it on desktop. Neither is material enough to change the frozen source.
