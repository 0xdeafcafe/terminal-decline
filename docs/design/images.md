# Inline images: kitty graphics, iTerm2 and Sixel (B.3, B.5)

Status: **draft, revised after an Opus (xhigh) review** (29 Oct 2026; §0).
Builds on the accepted cell design (`docs/design/cells.md`), which already
fixes how images are stored: kitty Unicode placeholders as cells (§7), the
per-screen `Graphics` with its placement table keyed by stable line number
(§8.1, §8.2), Sixel and iTerm2 images as placeholder cells of Terminal
Decline's own images (§8.3), decoding through a trait outside the lock
(§8.4), the snapshot fields (§9.2) and the vte APC/DCS/OSC plumbing (§14,
landed). This document does not repeat those; it designs what they leave
open: the protocols command by command, the process-wide **32 MB image
budget** (Alex's decision) and eviction, how pixels reach the GPU, how
images go into scrollback and come back, Sixel, costs, tests and the PR
plan. Where it changes a cell-design decision, it says so (§4.3).

Modules named here:
- **term-core**: escape-code handling, the screen, scrollback; no platform
  code. Owns the image state and the protocols' meaning.
- **term-image** (new, pure Rust, no platform code): the parsers and
  decoders that need no terminal state: kitty control keys, streaming
  base64, zlib inflate with an output cap, iTerm2 arguments, the Sixel
  decoder, pixel conversion, and the format sniffing and size reading of
  PNG/JPEG/GIF/WebP headers. Small, fuzzed on its own, used by term-core.
- **term-render**: Metal; draws image pieces and owns the Metal pixel
  buffers. One renderer per window draws every pane in it (B.2,
  docs/design/splits.md).
- **term-app**: the AppKit window and the `session` module; implements the
  image host (ImageIO decoding, file and shared-memory access, the decode
  queue).
- **the vendored vte parser**: one small change for Sixel (§8.1).

---

## 0. Revisions after the review

| Review item | Where |
|---|---|
| Hidden tabs and never-placed images could hold the budget forever: "busy" now means only receiving or a job running; new images start as just used; only images drawn in a **shown** window's latest frame are protected; hidden windows' images and images with no placement go first | §5.3 |
| Replacing an image near the budget no longer fails: the old image's bytes are given back first | §5.2 |
| `mpv --vo=kitty` (a new id-less image every frame, never deleted): id-less images are their own kind, freed when their last placement goes; an opaque id-less placement covering an earlier one at the same z replaces it | §3.3, §4.3 |
| Queries are answered before any later input; file and shared-memory transfers are opened and checked at once, read later; no file is read under the lock | §3.3, §3.4 |
| Files on network volumes and iCloud files not yet downloaded are refused; the header read for `a=T` happens with the lock released | §3.4 |
| Delete specifiers' scope fixed (history included for `i/I/n/N/r/R`; virtual placements only by those) | §3.5 |
| Placement key allows several `p=0` placements on one row; visibility is "intersects the view"; placement rows capped | §4.1 |
| Page-rounded buffer sizes and metadata counted or capped; evicted records cleaned up | §5.1, §5.2 |
| Decode jobs: buffers allocated when a job starts, still counted until it ends, producers held back, process-wide job numbers, no busy loop while re-decoding, re-decodes sent from the snapshot path | §4.2, §5.4 |
| Sixel needs a "DCS ended, complete or not" call in vte | §8.1, §11 |
| PR plan dependencies fixed (cells PR 11, F3.3, `MetalPixels`, the vte PR); images stay behind a hidden switch until drawing and the host land | §11 |
| imgcat of a phone photo: iTerm2 images are decoded at the size they are drawn | §7.2 |
| Pool code that panics can't break other windows; decode jobs catch panics | §5.2, §4.2 |
| Chunks continue as kitty continues them; per-chunk base64; a stalled upload is capped and can be stopped | §3.2 |
| Own images are freed (their own virtual placement isn't a reference); kitty images are no longer kept alive by placeholder text (changes cells §8.2) | §4.3 |
| Format sniffing and sizes read in pure Rust; ImageIO only decodes, off the lock | §6.1 |
| Selection and block cursor over images; z order across images; placeholder quads exact | §6.3 |
| File rules tightened; the "no detail in errors" claim corrected | §3.4 |
| iTerm2 cursor at the bottom-right corner; Sixel colour and growth details | §7.2, §8.1 |
| Transient decode memory, eviction cost, snapshot holds the pixel buffer, crate dependency direction, the pool reaching every terminal, large displays, fuzzing with images on | §5, §4, §9, §10, §12 |

---

## 1. Decisions at a glance

| Question | Decision | Why |
|---|---|---|
| Which kitty actions | transmit `t`, transmit+place `T`, place `p`, delete `d`, query `q`; Unicode placeholders (`U=1`); image numbers (`I=`); chunking (`m=`); zlib (`o=z`); formats 24, 32, 100 (PNG) | what `kitten icat`, yazi, image.nvim, timg, chafa, mpv use |
| Not in 1.0 | animation (`a=f`, `a=a`, `a=c`), relative placements (`P=`, `Q=`, `H=`, `V=`): handled as kitty handles an unknown action (§3.6) | no target program needs them; open question 1 |
| Transmission media | direct (`t=d`) always; file (`t=f`), temp file (`t=t`), shared memory (`t=s`), opened and checked at once, read later off the lock (§3.4); on by default | `kitten icat` probes them locally and falls back to direct over SSH; mpv uses shared memory |
| iTerm2 | `OSC 1337;File=` (inline only), `MultipartFile`/`FilePart`/`FileEnd`; PNG, JPEG, GIF, WebP (first frame of animations); decoded at the size drawn | `imgcat`, yazi's iTerm2 mode |
| Sixel | in 1.0: a streaming decoder in term-image, fuzzed; DA1 reports `4`; XTSMGRAPHICS; modes 80, 1070, 8452 | `chafa --format sixel`, `img2sixel`, lsix |
| Where pixels live | one **shared-storage Metal buffer** per image, allocated by the host, written by the decoder, sampled by the GPU through a texture made over that buffer | one copy for CPU and GPU (cells §8.4); no uploads |
| Budget | **32 MB per process** (setting `image_memory`), across every window and tab, counting pixel buffers at their allocated size, encoded bytes kept, and transfers in progress, each once; metadata capped | Alex's decision |
| Who enforces it | a process-wide `ImagePool` in term-core with its own lock (taken after a terminal's, never before; survives a panic) | eviction can free an idle image of another window without that window's terminal lock |
| Eviction | hidden windows' images first, then images with no placement, then least recently drawn; never an image drawn in a shown window's latest frame; pixels go before small encoded copies | an image scrolled into history comes back when scrolled to, if it was compressed |
| Hidden windows | buffers of images that can be decoded again are made **purgeable** while hidden | roadmap B.3 "volatile while hidden" |
| Size before decode | read from the header by term-image (PNG, JPEG with EXIF orientation, GIF, WebP) in microseconds, so cursor movement and placeholder cells happen in stream order; decoding is off the lock | `a=T` and iTerm2 move the cursor by the image's size before the next byte is parsed |
| Drawing | one instanced draw per run of placements of one image in (z, id) order, one quad per placement or per run of placeholder cells; layers as cells §8.1 | a 40×20 placeholder image is 20 quads, not 800 |
| Scrollback | kitty direct placements stay in the placement table by line number until their block is freed; placeholder images (kitty's and our own) are cells and go into history with their text | cells §8.2, §8.3, §11.7 |
| Until it all lands | a hidden switch keeps images off in daily builds (no replies to `APC G`, no `4` in DA1) until drawing and the host are merged | programs fall back to text instead of showing empty boxes |

---

## 2. What it has to serve

The roadmap's "done when": `kitty +kitten icat`, `imgcat`, yazi previews
(B.3), `chafa --format sixel` (B.5). Each uses the protocols differently:

| Program | What it sends | What it needs from us |
|---|---|---|
| `kitten icat` (kitty 0.3x) | a query `a=q` with `t=d`, then with `t=f`/`t=t`/`t=s` to find a medium that works, fenced by DA1; then `a=T,f=100` (PNG) or `f=32`, chunked at 4096 bytes, `q=2` | query replies **before** the DA1 reply, the media, cursor movement, `CSI 14 t` / `16 t` and `TIOCGWINSZ` pixel sizes (already answered) |
| `imgcat` (iTerm2 utilities) | `OSC 1337;File=inline=1;size=N;name=…:<base64> BEL`, newer copies `MultipartFile` | streaming the OSC (it can be megabytes); phone photos (12 MP) |
| yazi | kitty Unicode placeholders (`a=T,U=1` then U+10EEEE cells) when it detects kitty graphics; iTerm2 or Sixel otherwise; deletes with `a=d,d=I` / `d=i` and replaces previews of similar size | placeholders (cells §7), replacing near the budget, a reply to its `a=q` probe |
| `chafa --format sixel`, `img2sixel` | `DCS 0;1;0 q "1;1;W;H #… ST`; checks DA1 for `4` | Sixel decoding, DA1, XTSMGRAPHICS |
| image.nvim, timg, viu | kitty direct placements, delete by id/placement, sometimes `a=p` on scroll | placements by id, `d=p`/`d=i`, `C=1` |
| mpv `--vo=kitty` | every frame `a=T,f=24,s,v,C=1,q=2` **with no `i`**, at the same cursor position, `t=s` (one shm name reused every frame) or `t=d`; `a=d` only on reconfigure and exit | id-less images freed and replaced as kitty does (§3.3, §4.3); shm opened at once (§3.4) |
| tmux (passthrough) | the same sequences wrapped by tmux | nothing extra: tmux unwraps them |

From the Definition of done: every change carries numbers on the standard
set (§9), and idle stays at 0% CPU and 0 wakeups (nothing here runs a
timer; the decode queue is idle unless fed).

---

## 3. kitty graphics

### 3.1 Getting the command

vte calls `apc_start`, `apc_put` (slices split at ESC/CAN/SUB with
`memchr3`) and `apc_end(complete)` (cells §14, landed). term-core's
handler:

1. On the first byte: `G` means kitty graphics; anything else sets the
   state to `Ignore` and the rest of the APC is skipped. With images off
   (the hidden switch, or `image_memory = 0`), `G` is skipped too and
   **nothing is replied**: any reply tells a program graphics work.
2. Control keys up to the first `;`, at most 4 KB (cells §14), parsed by
   `term_image::kitty::Keys::parse` into a `Command` (below). Over 4 KB,
   or a bad value: `EINVAL` (if a reply is due, §3.6) and `Ignore`.
3. The payload streams through `term_image::Base64` into the **sink** the
   command chose: an encoded buffer (PNG, or anything with `o=z`), the
   pixel buffer directly (raw `f=24/32`), or a small path buffer (`t=f/t/s`:
   the payload is a path or shared-memory name, at most 4 KB). The base64
   state starts fresh in each chunk (kitty decodes each chunk on its own,
   so `=` padding may end any chunk).
4. `apc_end(true)` finishes the chunk. `apc_end(false)` (abandoned)
   abandons the whole transmission and frees what it held.

```rust
// term-image::kitty
pub struct Command {
    pub action: Action,          // t T p d q (f a c parsed, not supported)
    pub quiet: u8,               // q: 0, 1 (no OK), 2 (no errors either)
    pub format: Format,          // 24, 32, 100
    pub medium: Medium,          // d f t s
    pub compressed: bool,        // o=z
    pub more: bool,              // m=1
    pub image_id: u32,           // i (0 = none)
    pub image_number: u32,       // I (0 = none)
    pub placement_id: u32,       // p
    pub data_width: u32, pub data_height: u32,   // s, v (raw formats)
    pub file_size: u64, pub file_offset: u64,    // S, O
    pub src: [u32; 4],           // x, y, w, h
    pub cell_offset: (u16, u16), // X, Y
    pub cols: u16, pub rows: u16,// c, r
    pub no_cursor_move: bool,    // C=1
    pub virtual_: bool,          // U=1
    pub z: i32,
    pub delete: Option<Delete>,  // d=… with x, y, z for the specifiers that use them
    pub unsupported: bool,       // P, Q, H, V or an animation action seen
}
```

`Keys::parse` is table-driven: one ASCII letter per key, values unsigned
or signed 32-bit integers or single letters; a repeated key takes the
last value. Unknown keys are ignored. kitty instead logs and drops a
command with an unknown key; being lenient here is deliberate (a newer
client's extra key still works) and is a difference from kitty. `i` and
`I` together are `EINVAL`, as the protocol says.

### 3.2 Chunked transmissions

- The first chunk (`m=1`) carries every key; later chunks carry `m` (and
  possibly `q`). term-core keeps **one transmission in progress per
  terminal** (`Graphics::receiving`), with the first chunk's `Command`.
- As in kitty: while an upload is in progress, a transmit (`a=t`, `T`,
  `q`) with `t=d` is taken as its next chunk (only `m` and `q` are read
  from it), so clients that repeat their keys on every chunk work. A
  delete (`a=d`) abandons the upload. Any other command runs and the
  upload waits for its next chunk.
- Bytes received count against the budget as they arrive (§5.1). One
  terminal's uploads in progress may hold at most **a quarter of the
  budget** (or the declared size, if smaller); passing it abandons the
  upload with the over-budget error (§3.6).
- An upload with no further chunk is never timed out (no timers). It is
  abandoned by a delete, RIS, the terminal ending, or the pool: when
  another terminal needs room and nothing else can be evicted, the pool
  stops the upload that has gone longest without a chunk; its owner
  finds out at its next chunk and replies with the error.

### 3.3 Transmit, place, query

**Queries** (`a=q`) are answered **before any later input is parsed**,
as the protocol requires and as icat's DA1 fence expects: steps 1–3
below run against a throwaway image, with media opened and checked at
once (§3.4) and at most 64 KB of their data looked at; nothing is
stored; the reply is queued before the parser goes on.

On the last chunk of `a=t` / `a=T` (or the only one):

1. **Which image.**
   - `i=N` names it; an existing image with that id is replaced (its
     placements are deleted, as the protocol says), and the pool is told
     it is a replacement, so the old image's bytes are given back before
     the new one is checked against the budget (§5.2).
   - `I=N` (with no `i`) makes a new image with the next free id from a
     counter that skips ids in use, and remembers number → id (the newest
     image with that number wins).
   - Neither (only meaningful for `a=T`): an **id-less** image, a kind of
     its own (`ImageKey::Anonymous`, §4.1) with its own counter, so a
     later explicit `i=N` can never replace it. A placement id given with
     no image id is ignored.
2. **Size first.** Raw `f=24/32`: `s` × `v` are required (`EINVAL`
   without them). PNG: term-image reads the size from the header
   (`IHDR`, the first 33 bytes) in well under a microsecond. The pixel
   caps apply now: width and height ≤ 10,000, and width × height × 4 ≤
   the budget (cells §14). Over: the too-big error (§3.6), nothing
   allocated.
3. **Pixels.** Raw data is converted to premultiplied BGRA as it streams
   into the pixel buffer (§5.1), so the image is `Ready` at once. PNG is
   handed to the decode queue (`Pending`); its reply waits for the decode
   (cells §8.4). With `o=z`, `term_image::Inflate` (miniz_oxide, pure
   Rust) inflates as the payload streams, refusing output past what the
   keys promise (raw: `s × v × bytes per pixel`; PNG: the budget), so a
   zip bomb costs nothing.
4. **Place** (`a=T`, or later `a=p`): a `Placement` (cells §8.1) at the
   cursor. Its size in cells is `c`/`r` if given, else the source
   rectangle's pixels plus the `X`/`Y` offsets, divided by the cell size,
   rounded up. A placement with the same (image, placement id ≠ 0)
   replaces the old one.
   - **Id-less opaque frames** (mpv): when a new placement of an id-less
     image with no alpha (`f=24`, or `f=32`/PNG whose decoder found every
     pixel opaque) covers an earlier id-less placement's whole cell box at
     the same z, the earlier placement is deleted, and with it its image
     (§4.3). This is how kitty's own clean-up behaves for such streams,
     and keeps one frame in memory, not a pile.
   - **Cursor.** Unless `C=1`, the cursor moves right by the placement's
     columns and down by its rows − 1, with linefeeds that scroll as text
     would, so the image's line numbers move with it into history. If
     that passes the last column, it wraps to column 0 of the next row,
     as kitty does.
5. **Virtual** (`U=1`): stored in `virtual_placements` with its cell box;
   no cursor movement, nothing drawn until placeholder cells refer to it
   (cells §7).
6. **Reply** (§3.6).

`a=p` on an image that is still `Pending` makes the placement; it is drawn
when the decode finishes. On an image that doesn't exist: `ENOENT`.

### 3.4 Media: file, temp file, shared memory

The payload is a path (`t=f`, `t=t`) or a POSIX shared-memory name
(`t=s`). The host (term-app: `open`, `shm_open` and `mmap` are platform
code) handles them in two parts.

**At once, in stream order** (microseconds; on the reader thread, under
the lock):
- `t=s`: the name must start with `/`, contain no other `/`, and be ≤ 31
  bytes (macOS's `PSHMNAMLEN`); `shm_open(name, O_RDONLY)`, `fstat`, the
  size checked against what is left of the budget, `mmap`, `shm_unlink`.
  mpv reuses one name for every frame, so the object must be taken before
  the next frame is written.
- `t=f`, `t=t`: `open(path, O_RDONLY | O_NONBLOCK | O_NOCTTY |
  O_CLOEXEC)`, then `fstat`: regular files only (no FIFOs, devices or
  sockets, so nothing can block or have side effects); `fstatfs` must say
  `MNT_LOCAL` (no network or FUSE volumes, which can hang); files whose
  `st_flags` include `SF_DATALESS` (iCloud, not downloaded) are refused,
  since reading them would wait for the network; size (or `S`) ≤ what is
  left of the budget.
- `t=t` also: the parent directory's real path (`realpath`, so `..` and
  symlinks can't escape) must be `/private/tmp` or `$TMPDIR`'s real path
  (`/private/var/folders/…/T`), the name must contain
  `tty-graphics-protocol`, and the file is removed at once with `unlinkat`
  on that opened directory; the open descriptor keeps its contents.

**Later, off the lock**: the data (`S` bytes from offset `O`) is read from
the open descriptor or mapping on the decode queue (§4.2), converted or
decoded, and the descriptor closed or the mapping unmapped.

**The header for `a=T` with PNG from a file**: the cursor movement needs
the size before the next byte is parsed. The handler asks the parser to
stop after this APC (vte's `advance_until_terminated` with `terminated()`
returning true); the reader thread then **unlocks**, reads the first 64 KB
from the descriptor, locks again, finishes the command and parses on from
where it stopped. Stream order is kept, as in kitty, and nothing slow
happens under the lock.

Errors on media are one generic reply (`EBADF`-style) with no path or
detail. That doesn't make local files invisible to a remote program: OK
versus error, and the cursor movement after `a=T` (readable with
`CSI 6n`), still say whether a local image exists and roughly its size.
kitty has the same property. The program never gets the pixels. A setting
turns media off (`image_file_transfer = false`): only `t=d` works and the
others get the error, so icat falls back to direct. Default on (open
question 5).

### 3.5 Delete

`a=d` with `d=` (default `a`). Lower case deletes placements; upper case
also frees each image that is then unreferenced (§4.3). Id-less images are
freed when their last placement goes, whichever case (as kitty does). An
index from image to its placements makes the by-image specifiers cheap.

| `d=` | Deletes placements… | Where | Virtual placements |
|---|---|---|---|
| `a`/`A` | all | on the visible screen | not touched |
| `i`/`I` | of image `i` (only placement `p` if given) | everywhere, history included | yes |
| `n`/`N` | of the newest image with number `I` (`p` as above) | everywhere | yes |
| `r`/`R` | of images with ids `x..=y` | everywhere | yes |
| `c`/`C` | intersecting the cursor's cell | visible screen | not touched |
| `p`/`P` | intersecting cell (`x`, `y`), 1-based | visible screen | not touched |
| `q`/`Q` | as `p`, with z-index `z` | visible screen | not touched |
| `x`/`X` | intersecting column `x` (1-based) | visible screen | not touched |
| `y`/`Y` | intersecting row `y` (1-based) | visible screen | not touched |
| `z`/`Z` | with z-index `z` | visible screen | not touched |
| `f`/`F` | (animation frames) nothing | — | — |

"Not touched" follows the protocol ("a, c, p, q, x, y, z and their capital
variants never affect virtual placements"), so a program clearing the
screen with `d=a` keeps yazi's placeholder images. Visible-screen deletes
are a range of the placement map by top line (§4.1): O(placements on
screen). A deleted virtual placement leaves its placeholder cells as text
that draws nothing. Delete replies only with errors, and only with `i`
given.

### 3.6 Replies

`ESC _ G i=<id>[,I=<number>][,p=<placement>];<message> ESC \`, queued on
the terminal's reply buffer like other replies (written by the reader
thread). Rules:

- no reply if the command had neither `i` nor `I` (the protocol's rule);
- `q=1` drops `OK`, `q=2` drops errors too;
- no reply at all while images are off (§3.1);
- queries reply before later input is parsed (§3.3); PNG transmits reply
  when decoding finishes (cells §8.4); raw data from `t=d` replies at once;
  raw data from files and shared memory when it has been read;
- messages: `OK`, or `<CODE>:<text>` with the codes kitty uses (`EINVAL`,
  `ENOENT`, `EBADF`, `ENODATA`, `EFBIG` and the rest, checked against
  kitty's source in PR I.2 and listed in a table test; icat and yazi only
  compare the `OK` prefix);
- unsupported actions (`a=f/a/c`) get what kitty gives an unknown action:
  no reply. PR I.2 checks icat with an animated GIF: if icat waits for an
  answer, an `EINVAL` reply is sent instead.

### 3.7 Things that change around images

All of these follow cells §8.2 (scroll regions, history blocks freed, the
alternate screen, ED 2/ED 3/RIS, reflow). Added here:

- **Cell size changes** (Cmd+= / Cmd+-, font change): placements keep
  their size in cells, so images scale with the text; kitty images are
  not decoded again. This differs from kitty, which keeps images of
  implicit size at one image pixel per screen pixel and changes their
  size in cells. Sixel images keep their cell size likewise. iTerm2
  images, decoded at the size drawn (§7.2), are decoded again once the
  drawn size grows past 1.25× what was decoded.
- **Cmd+K** (clear scrollback) is ED 3.
- **Resize**: direct placements are remapped like other tracked positions
  (cells §13, risk 5: they register with cells PR 11's tracked
  positions); placeholder cells reflow as cells (cells §8.3, risk 9).
- **RIS** keeps the pool handle and the process-wide job counter, as it
  keeps replies (§4.2).
- **The session replacing its terminal after a caught panic**: the pool
  releases everything the old terminal owned by its `terminal_id`
  (`release_owner`), without walking the old terminal's state (§5.2).

---

## 4. Image state in term-core

### 4.1 Structures

cells §8.1 gives `Graphics`, `Placement` and the layers. Refined:

```rust
/// kitty ids, id-less kitty images and our own (Sixel, iTerm2) are separate
/// namespaces. The placeholder style's `OWN_IMAGE` attribute marks ours (cells §8.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ImageKey { Kitty(u32), Anonymous(u32), Own(u32) }

pub struct Image {
    pub key: ImageKey,
    pub number: u32,              // kitty I=, 0 if none
    pub width: u32, pub height: u32,
    pub opaque: bool,             // no alpha: id-less replacement (§3.3)
    pub state: ImageState,
    pub pool: PoolId,             // its entry in the process-wide pool (§5)
    refs: Refs,
}

pub enum ImageState { Pending, Ready, Lost, Failed }  // Lost: evicted; drawn as nothing

pub struct Graphics {
    images: HashMap<ImageKey, Image>,                 // capped: 4096 per terminal
    numbers: HashMap<u32, u32>,                       // I= → newest id
    placements: BTreeMap<(u64, u64), Placement>,      // (top line, serial); capped: 4096 per screen
    by_id: HashMap<(ImageKey, u32), (u64, u64)>,      // (image, placement id ≠ 0) → key, for replacing
    by_image: HashMap<ImageKey, SmallVec<[(u64, u64); 2]>>, // for i/I/n/N/r/R and freeing
    tallest: u16,                                     // max rows of any placement (rows capped at 1000)
    virtual_placements: HashMap<(ImageKey, u32), VirtualPlacement>,
    receiving: Option<Transmission>,                  // upload in progress (§3.2)
    next_id: u32, next_anonymous: u32, next_own: u32,
    next_serial: u64,
    pool: Arc<ImagePool>,
}

pub struct VirtualPlacement {
    pub image: ImageKey, pub id: u32,
    pub cols: u16, pub rows: u16,                     // the cell box
    pub fit: Fit,                                     // kitty: scale to fit, centred; own images: §7.2/§8.2
}
```

- **Several placements of one image on one row** (`p=0` means "a new
  placement each time", e.g. one icon repeated) each get their own serial,
  so they never collide in the map.
- **Placements that reach into the view.** A placement's top line may be
  above the view. The snapshot asks the map for top lines in
  `[view_top − tallest, view_bottom]` and keeps those that **intersect**
  the view (a placement taller than the view is kept). Rows per placement
  are capped at 1000, so `tallest` (which only grows until the map is
  empty) can't make every snapshot scan 100k lines.
- **Caps**: past 4096 placements on a screen or 4096 images in a
  terminal, the oldest unreferenced image or the oldest placement is
  dropped first (kitty has similar limits). Each record is ~150–250 B, so
  the caps bound metadata at ~2 MB per terminal in the worst case; the
  budget doesn't count it.

### 4.2 The host, the pixel target, and jobs off the lock

The crate dependency runs term-core → term-image, so the pixel target the
Sixel decoder and the converters write into is defined in term-image, and
term-core's traits build on it:

```rust
// term-image
pub trait PixelTarget {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn row_pitch(&self) -> usize;
    fn bytes_mut(&mut self) -> &mut [u8];   // premultiplied BGRA8
}
/// How a decoder asks for (more) room; the caller reserves budget first.
pub type Alloc<'a> = &'a mut dyn FnMut(u32, u32) -> Option<Box<dyn PixelTarget + Send>>;

// term-core
pub trait PixelBuffer: term_image::PixelTarget + Send + Sync {
    fn allocated_size(&self) -> usize;      // what macOS really holds (page-rounded)
    fn set_volatile(&self, volatile: bool) -> bool;  // false: macOS already took it
    fn as_any(&self) -> &dyn std::any::Any; // term-render gets its Metal buffer back
}

/// Implemented by term-app (ImageIO, files, shm) and by a pure-Rust test host.
pub trait ImageHost: Send + Sync {
    fn alloc(&self, width: u32, height: u32) -> Option<Box<dyn PixelBuffer>>;
    fn open_medium(&self, req: &MediumRequest) -> Result<OpenMedium, ImageError>; // §3.4, at once
    fn read_head(&self, m: &OpenMedium, max: usize) -> Result<Vec<u8>, ImageError>; // lock released
    fn decode(&self, bytes: &[u8], into: &mut dyn PixelBuffer, max_size: (u32, u32)) -> Result<(), ImageError>;
}
```

As built (I.1): the decoders are generic over the target type, with
`AllocWith<'a, T> = &'a mut dyn FnMut(u32, u32) -> Option<T>` and `Alloc`
its boxed form, so term-core gets its own `Box<dyn PixelBuffer>` back
from the Sixel decoder without a downcast. `term_image::VecPixels` is the
`Vec`-backed target for tests and the pure-Rust test host.

term-core has no threads of its own. Work that must leave the lock is
queued on the terminal:

```rust
pub enum ImageJob {
    Decode { job: JobId, image: ImageKey, generation: u32, bytes: Encoded, size: (u32, u32) },
    Read   { job: JobId, image: ImageKey, generation: u32, medium: OpenMedium, then: AfterRead },
}
impl Terminal {
    pub fn take_image_jobs(&mut self, out: &mut Vec<ImageJob>);   // after each parse slice, and after each snapshot
    pub fn image_job_done(&mut self, done: JobDone);              // locks briefly
}
```

- **Job numbers** come from a process-wide counter, and completion checks
  (`terminal_id`, image key, generation), so a job finishing after RIS or
  after the session replaced its terminal can't land on a new image.
- **When a job runs**, the session reserves its pixel bytes and allocates
  the buffer then, not when the job was queued; until then the job holds
  only the encoded bytes. The entry stays counted until the job ends,
  even if its image was deleted meanwhile, so the budget never undercounts
  memory still held.
- **Holding the producer back**: when a terminal's queued encoded bytes
  pass a quarter of the budget, its reader thread waits (without the
  lock) for the queue to drain before reading more from the PTY, so a
  `for f in *.jpg; do imgcat "$f"; done` is slowed by the kernel's PTY
  buffer instead of filling memory.
- **The queue**: one serial dispatch queue per process at utility QoS
  (decoding is memory-bound; two at once would only raise the peak). Each
  job runs inside `catch_unwind`, so one bad image fails that job and the
  queue keeps going.
- **Completion** locks the terminal briefly for `image_job_done`: marks
  the image `Ready` (or `Failed`), queues the reply, bumps the counter;
  the session wakes the view.
- **Re-decodes** (§5.4) are queued by the snapshot and taken right after
  it on the main thread, so an idle terminal (no parse slices) still
  runs them.

### 4.3 Who needs an image, and when it is freed

This changes cells §8.2's "needs it" rule for kitty images: placeholder
text no longer keeps them alive.

- **kitty images with an id** (`Kitty`) are needed by their placements
  and virtual placements only. Placeholder cells are just text pointing at
  an id, as in kitty: they draw the image while it exists and nothing
  after. Such an image stays until deleted by an upper-case delete (when
  it has no placement left), replaced by a new image with the same id,
  dropped by the caps (§4.1), or evicted (§5). An unplaced kitty image is
  kept, because programs transmit first and place later.
- **Id-less images** (`Anonymous`) are freed when their last placement
  goes, by any delete, by replacement (§3.3), or by scrolling out of
  history.
- **Our own images** (`Own`, Sixel and iTerm2) are needed by
  `screen_styles` and `history_blocks`; their own virtual placement isn't
  counted (it exists as long as the image does):
  - `screen_styles`: placeholder cells of one image share one style
    (cells §7), so interning a placeholder style adds one and the style
    sweep freeing it (cells §5.2) subtracts one. A sweep runs only when
    an intern misses with at least max(1024, 1.5 × cells) entries, so on
    a screen with few styles an overwritten image can stay counted for a
    long time. That only costs metadata: it isn't drawn, so its pixels are
    evicted first (§5.3);
  - `history_blocks`: a block counts an image once, however many of its
    lines name it; the count is dropped when the block is freed
    (`Scrollback::push` returns freed blocks' image lists; ED 3 and RIS
    return all of them).
  - An own image is freed when both are zero.
- **A kitty id reused** after a delete: placeholder cells in history that
  named the old image show the new one, as in kitty.

### 4.4 Snapshot additions

The snapshot (cells §9.2) already lists `placements`, `virtual_placements`
and `new_images`. Concretely:

```rust
pub struct SnapImage {
    pub pool: PoolId, pub generation: u32,
    pub pixels: Arc<dyn PixelBuffer>,      // the renderer needs no pool lock to reach it
    pub width: u32, pub height: u32,
}
pub struct SnapPlacement {
    pub image: SnapImage,
    pub view_row: i32, pub col: u16,       // top-left cell relative to the view (may be negative)
    pub cols: u16, pub rows: u16, pub offset: (u16, u16), pub src: [u32; 4],
    pub layer: Layer, pub z: i32, pub serial: u64,
}
pub struct SnapVirtual { pub key: (ImageKey, u32), pub image: SnapImage, pub cols: u16, pub rows: u16, pub fit: Fit }
```

`new_images` goes away: nothing is uploaded (§6). `generation` changes
when the pixels change (replaced, decoded again). Only `Ready` images are
listed; `Lost` images with an encoded copy that are in view queue one
re-decode (§5.4) and draw nothing this frame. Taking the snapshot tells
the pool which images this window drew (§5.2).

---

## 5. The budget: 32 MB per process

### 5.1 What counts

Every byte Terminal Decline holds for images, once (cells §8.4):
- pixel buffers at their **allocated size** (`MTLBuffer.allocatedSize`:
  whole 16 KB pages, so 10,000 tiny images count as the 160 MB they really
  take, not 160 KB);
- encoded bytes: PNG/JPEG/GIF waiting to be decoded, and the copy kept
  after decoding (§5.4), stored in 64 KB pieces so growing them never
  copies;
- uploads in progress: bytes of a chunked transmission or an OSC 1337
  stream so far;
- headroom for the decode running now: ImageIO may make a full-size
  intermediate (16-bit or interlaced PNGs, colour conversion), so a
  decode reserves its output size again until it finishes.

Not counted, but capped: placements and image records (§4.1). Not
counted: texture objects made over buffers (no storage of their own,
§6.2), Sixel's palette (4 KB per decoder).

`image_memory = "32MB"` (the setting, in the preferences schema; read by
config, passed to the pool; live: lowering it evicts at once).
`image_memory = 0` turns images off as the hidden switch does (§3.1): no
replies, no `4` in DA1, iTerm2 files dropped.

**Large displays**: a full-screen image at 5K is 59 MB, on a 16" MacBook
Pro about 31 MB. With 32 MB, a kitty image larger than the budget is
refused; iTerm2 images are decoded no larger than they are drawn (§7.2),
so they fit; mpv at full screen on a large display gets refusals. Open
question 1.

### 5.2 The pool

```rust
/// One per process, shared by every terminal. Its own lock; taken after a
/// terminal's lock, never the other way round; poisoning is ignored (as
/// session::lock does) and `used` is only changed once an update is complete.
pub struct ImagePool { inner: Mutex<PoolInner> }
struct PoolInner {
    limit: usize,
    used: usize,
    entries: HashMap<PoolId, Entry>,
    order: BTreeMap<(u8, u64, PoolId), ()>,   // (eviction class, last_used, id): §5.3
    shown: HashMap<u64, bool>,                // per owner: is its window shown
    latest_frame: HashMap<u64, u64>,          // per owner: clock value of its latest frame
    evicted: HashMap<u64, Vec<PoolId>>,       // per owner, cleared at its next command or snapshot
    clock: u64,                               // ticks once per snapshot of any window
}
struct Entry {
    owner: u64,                               // the terminal's terminal_id
    pixels: Option<Arc<dyn PixelBuffer>>,
    encoded: Option<Encoded>,                 // kept only if ≤ ¼ of the pixel bytes (§5.4)
    bytes: usize,
    last_used: u64,                           // clock when made, or when last drawn
    placed: bool,                             // has a placement or a placeholder in use
    busy: bool,                               // receiving, or a job running
    volatile: bool,
    generation: u32,
}
```

- `reserve(owner, bytes, replacing: Option<PoolId>)`: gives back the
  replaced entry's bytes first, then evicts (§5.3) until the request
  fits; if it still doesn't, refuses.
- `drawn(owner, ids)`: called once per frame by the snapshot with the
  images about to be drawn (a few hundred ns): `last_used = clock`,
  `latest_frame[owner] = clock`.
- `set_shown(owner, bool)`: from the session when a window hides or shows
  (§5.5).
- `release_owner(owner)`: frees everything a terminal owned; used when a
  terminal is dropped, including after a caught panic.
- Eviction never needs another terminal's lock: the owner learns from
  `evicted[owner]` at its next graphics command or snapshot and marks
  those images `Lost`.

### 5.3 Eviction

An entry is **protected** if it is `busy`, or it was drawn in its owner's
latest frame **and** that owner is shown. Everything else may be evicted,
cheapest loss first, oldest first within a class (the `order` index, so
choosing is O(log n), not a scan under the lock the main thread waits on
in `drawn`):

1. images of hidden windows;
2. images with no placement (`placed` false: transmitted and never
   placed, or whose cells were overwritten), as kitty frees unreferenced
   images first;
3. the rest, least recently used first.

For each victim, the **pixels** go first; if it has an encoded copy, it
becomes `Lost`-with-source and can be decoded again when next drawn
(§5.4). Otherwise it is `Lost`: placements and placeholders draw nothing,
kitty commands naming it get `ENOENT`, and its owner deletes it. When no
candidate has pixels left, **encoded copies** go, in the same order. Still
over: the new image is refused (the too-big error). An image bigger than
the whole budget is refused before anything is evicted.

A new image starts with `last_used` = the current clock, so one
transmitted and never placed ages normally instead of being treated as
"just drawn".

Memory goes back to macOS when the last reference to the Metal buffer
goes: the pool's, the snapshot's (§4.4), the textures the renderer made
over it, and command buffers in flight (Metal retains what a command
buffer uses). So eviction is safe while a frame is on the GPU, and the
memory returns within a frame.

### 5.4 The encoded copy, and decoding again

After a PNG/JPEG/GIF decode, the encoded bytes are kept only if they are
at most a quarter of the pixel bytes (a 1624×1018 screenshot is 6.6 MB of
pixels and 0.3–1.5 MB as PNG). Otherwise they are dropped.

A `Lost`-with-source image in view gets **one** re-decode job at a time.
If its reservation fails (the budget is full of protected images), it
waits until the pool next frees bytes; a failure never wakes the view,
so a lost image in view can't make an idle window redraw in a loop. It
draws nothing until decoded (typically one to a few frames; a 1 MB PNG
decodes in ~5–15 ms, measured in I.5).

### 5.5 Hidden windows

When a window hides (background tab, covered, minimised; DESIGN "Windows
and tabs"), the order is: the renderer waits for its frames in flight and
drops its textures (it already does this to give back drawables), then the
session calls `set_shown(owner, false)`. Entries of that terminal with an
encoded copy are then made purgeable (`setPurgeableState(.volatile)`);
macOS may take them under memory pressure without asking. Entries without
a source stay non-volatile. Hidden windows' images are also first in line
for eviction (§5.3). On show, `set_shown(true)` makes them non-volatile
again; an entry that macOS emptied becomes `Lost`-with-source, and `used`
drops by its bytes.

Whether volatile memory still counts in `footprint`'s physical footprint
is measured in PR I.5.

---

## 6. Decoding and the GPU

### 6.1 Sniffing and decoding

- **Sniffing and sizes** are pure Rust in term-image (Linux-testable,
  fuzzed): PNG (`IHDR`), GIF (logical screen descriptor), JPEG (the frame
  header, plus the EXIF orientation tag, which swaps width and height for
  orientations 5–8), WebP (the first chunk: `VP8 `, `VP8L`, or `VP8X`'s
  canvas; its EXIF orientation is not applied, as browsers don't).
  Anything else is refused before a decoder sees a byte (cells §8.4's
  list: PNG, JPEG, GIF, plus WebP since Alex's decision; HEIC is open
  question 4). Sub-µs, under the lock.
- **Decoding** (term-app, on the queue, off the lock): ImageIO's
  `CGImageSourceCreateImageAtIndex` (index 0: GIFs show their first
  frame) with `kCGImageSourceShouldCache = false`, drawn into a
  `CGBitmapContext` over the pixel buffer's memory, sRGB,
  `kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little`, with the
  orientation transform. When a smaller size is wanted (§7.2), ImageIO's
  thumbnail path (`kCGImageSourceThumbnailMaxPixelSize`) decodes at that
  size directly.
- **As decided (§12, question 3) and built in I.1**: decoding is pure
  Rust instead, in `term_image::decode` (the `png`, `jpeg-decoder` and
  `gif` crates), fuzzed with the other targets; term-app's host only runs
  it on the queue. `decode(bytes, into, limits)` writes premultiplied BGRA
  at `into`'s size (`fit` picks it from the header), averaging down and
  applying the EXIF orientation as rows stream; JPEG uses the decoder's
  DCT scaling (up to 1/8 each way) first, so a phone photo drawn small is
  never decoded at full size. Each decoder's own allocations are capped
  (`Limits::transient`). Colour profiles are ignored (taken as sRGB).
  Measured (`cargo bench -p term-image --bench decode`, M-series Mac):
  PNG 1624 × 1018 screenshot 4.3 ms at its size, 7.6 ms halved; JPEG
  1600 × 1200 11.8 ms, 3.1 ms at 400 × 300; binary size +321 KB
  stripped for the three decoders (on top of +50 KB for the parsers and
  Sixel), more than the 100–150 KB estimated. `jpeg-decoder` starts one
  short-lived thread per colour component for images over 128 × 128 (its
  built-in heuristic; rayon is off).
- **WebP** (Alex's decision, after I.1): `image-webp` (pure Rust, the
  `image` crate's decoder; no features, no threads), lossy, lossless and
  alpha; an animated WebP shows its first frame, as a GIF does. It has no
  scaled decoding, so the whole image is decoded at full size as RGB or
  RGBA and then streamed through the same scaler and colour conversion
  (ICC profile from `ICCP`, else sRGB). Its own memory limit covers only
  metadata chunks, so `decode` first checks an estimate against
  `transient`: the output (3 or 4 bytes a pixel) plus 4 bytes a pixel of
  working copy (YUV and alpha planes, or the lossless RGBA frame), plus 8
  more for an animation's canvas and frame; a counting allocator test
  holds the real peak under it (a 1600 × 1200 lossy photo peaks at
  9.0 MB of an estimated 13.4 MB). Measured (`cargo bench -p term-image
  --bench decode`, M-series Mac, colour conversion on, same run as PNG
  8.4 ms and JPEG 15.9–19 ms): the 1624 × 1018 screenshot as lossless
  WebP 9.6 ms at its size, 12.5 ms halved; the 1600 × 1200 photo as lossy
  WebP (`cwebp -q 80`, 15 KB) 20.4 ms at its size, 22.3 ms at 400 × 300
  (no DCT scaling to help, unlike JPEG's 3.2 ms). Binary size +196 KB
  stripped (200,768 bytes; thin LTO, measured as for the other decoders:
  a release binary calling `decode`, before and after).
- ImageIO parses untrusted bytes from programs (possibly remote), and a
  crash in it takes every window. Sniffing in Rust narrows what reaches
  it, and decoding off the lock keeps a slow image from stalling frames,
  but a decoder bug can still crash the app. Open question 3 (pure-Rust
  decoders, or a separate decoding process).
- Raw kitty data and Sixel never reach ImageIO: term-image converts RGB /
  RGBA / palette pixels into BGRA premultiplied as they arrive.
- **Colour** (decided, question 9): the window's layer stays untagged,
  and each image is converted once, when decoded, from its own colour
  space (its ICC profile; sRGB if it has none) to Display P3, in
  `term_image::color::ToP3`: the source curve through a table to 16-bit
  linear, a 3 × 3 matrix in fixed point, the sRGB curve (P3's) through a
  table. Matrix/TRC ICC profiles are read (`rXYZ`/`gXYZ`/`bXYZ`, `rTRC`
  as `curv` or `para`); others are taken as sRGB; a Display P3 profile
  (iPhone photos) is left as is; grey images are not touched. On by
  default in `decode::Options`; kitty's raw pixels
  (`Converter::display_p3`) and Sixel colours (`Decoder::display_p3`,
  registers keep sRGB) take it as sRGB when term-core asks. Cost about
  3 ns a pixel (M-series Mac, machine not idle): kitty's 8 MB full-HD
  RGBA 10.2 → 17.3 ms; the 1624 × 1018 PNG 4.3 → 9.5 ms (§9 still
  expects 5–15 ms).

### 6.2 Pixel buffers and textures (term-render)

- `MetalPixels` (term-render) wraps an `MTLBuffer` made with
  `newBufferWithLength:options:` (shared storage). Its row pitch is the
  width × 4 rounded up to
  `device.minimumLinearTextureAlignmentForPixelFormat(BGRA8Unorm)`, so a
  texture can be made over it with
  `buffer.newTextureWithDescriptor:offset:bytesPerRow:` (usage
  `ShaderRead`) — no copy, no upload, no `replaceRegion` wait (the buffer
  is never written after it is `Ready`).
- Pixel format `BGRA8Unorm`, as the drawable and the colour glyph texture,
  so images blend in the same space as text.
- Linear textures can't have mipmaps. Images drawn at less than half size
  would shimmer while scrolling, so the fragment shader takes four
  bilinear samples (a 2×2 box) when the scale is below ½; below ¼ it still
  aliases a little (open question 6). iTerm2 images avoid this by being
  decoded at the drawn size (§7.2).
- Textures are made per renderer (one per window, B.2) from the
  snapshot's `SnapImage`, kept in a map rebuilt each frame with only the
  images drawn in it (making a texture over a buffer is ~5–10 µs and
  allocates nothing big), and dropped entirely on hide.

### 6.3 Drawing

The frame (cells §8.1's order, with highlights placed) is drawn in the one
render encoder:

1. `BelowBackground` placements;
2. cell backgrounds (default-background runs push no rectangle in
   frame.rs, so the image shows through them as kitty requires);
3. `BelowText` placements and placeholder images;
4. **selection, search highlights and the block cursor over image
   cells**: today these are cell backgrounds (frame.rs sets the cell's
   background to the selection or cursor colour), which step 3 would
   cover; over image cells they are drawn here instead, as a translucent
   tint (selection, search) or the cursor rectangle. The rectangle buffer
   is split into backgrounds and decorations so this ordering is
   possible;
5. glyphs and decorations;
6. `AboveText` placements;
7. cursor overlays.

```rust
#[repr(C)]
pub struct ImageInstance {      // 32 bytes
    pub dst: [i16; 4],          // x, y, w, h in drawable pixels, clipped to the view before packing
    pub uv: [f32; 4],           // u0, v0, u1, v1, inset by half a texel inside the source rectangle
    pub min_scale: f32,         // for the 2×2 box when shrinking
    pub _pad: [u32; 1],
}
```

- Within a layer, placements are sorted by (z, image id, serial), the
  protocol's order, and consecutive ones of the same image are one
  instanced draw. A screen showing 1–20 images costs 1–20 draws (~1–2 µs
  each to encode).
- `dst` is clipped to the view in f32 before it is packed into i16, so
  tall placements far off screen can't overflow it. `uv` is inset by half
  a texel so bilinear filtering doesn't bleed in pixels from outside the
  `x,y,w,h` source rectangle.
- Placeholder cells: the renderer walks rows flagged `PLACEHOLDERS` (cells
  §7) and merges cells into one quad only if they are next to each other
  on screen **and** have consecutive placeholder columns in the same
  placeholder row of the same (image, placement). A 40×20 image is 20
  quads; a partly overwritten one splits into more.
- kitty's fit for virtual placements: the image is scaled to fit the cell
  box keeping its aspect ratio, centred; each quad's `uv` is computed from
  that exact fit (not from rounded pixels), so pieces line up.
- With the R.2 row cache, image quads are overlays rebuilt each frame
  (cells §9.3).

---

## 7. iTerm2 inline images (OSC 1337)

### 7.1 Getting the string

`osc_large` answers `Stream` for `1337;File=` and `1337;MultipartFile=`
(replacing today's `Drop`, cells §14), so the payload streams through
`osc_put`. A small `File=` (under 4 KB) arrives whole through
`osc_dispatch`; term-core joins `params[1..]` with `;` and feeds the same
reader. When vte reports exactly 16 parts the string may have been
truncated and is ignored (cells §14).

### 7.2 The reader

`term_image::iterm::Args` parses `key=value` pairs up to the `:`:

| Key | Meaning | We do |
|---|---|---|
| `inline` | 1 = show; 0 = download | only 1; `inline=0` (a file download) is dropped — we save no files |
| `name` | base64 file name | ignored |
| `size` | bytes | if given, the stream is refused early when it is over a quarter of the budget (§3.2's upload cap) |
| `width`, `height` | `N` (cells), `Npx`, `N%` (of the screen), `auto` | the cell box |
| `preserveAspectRatio` | default 1 | 1: fit inside the box, top-left aligned (iTerm2's alignment); 0: stretch |
| `doNotMoveCursor` | 1 = leave the cursor (iTerm2 3.5) | honoured |

The base64 after `:` streams into an encoded buffer. At the end
(`osc_end(true)` or dispatch):

1. sniff and read the size (§6.1; PNG/JPEG/GIF/WebP only; anything else is
   dropped);
2. the cell box: `auto` is the image's size in cells, scaled down to fit
   the screen's width (iTerm2 does this); percentages are of the screen;
   one of width/height `auto` keeps the aspect ratio. `Npx` and `auto`
   count image pixels as device pixels, like kitty and Sixel; iTerm2
   counts them as points, so images look half the size they do in iTerm2
   on a Retina display (open question 7);
3. **the decoded size** is the smaller of the image's size and the box in
   device pixels, and the pixel caps apply to that, not to the source: a
   12-megapixel photo (48 MB of pixels) shown at window size takes a few
   MB instead of being refused;
4. an own image (`ImageKey::Own`), a virtual placement of the box, and
   placeholder cells written at the cursor row by row (cells §8.3),
   scrolling as text does;
5. the cursor ends after the bottom-right corner of the image (on its last
   row, one column past its right edge), as iTerm2 and WezTerm leave it;
   imgcat prints its own newline. Unless `doNotMoveCursor=1`. Pinned by a
   test from a recording of iTerm2 3.5;
6. a decode job; the cells show the image when it is `Ready`.

`MultipartFile=<args>` starts the same reader; each `FilePart=<base64>`
(buffered by vte up to 1 MB, already the policy) appends; `FileEnd`
finishes. Another `MultipartFile` before `FileEnd` abandons the first; one
that never ends is capped and stoppable as §3.2's uploads are.

iTerm2 has no replies, so a refused image is silent; nothing is written
into the window for it (a notice per image would bury the screen under a
flood).

---

## 8. Sixel (B.5)

### 8.1 The decoder (term-image)

`term_image::sixel::Decoder` is a byte-at-a-time state machine fed with
slices (`put_slice` from vte's DCS passthrough, cells §14):

- **Introducer** `DCS P1 ; P2 ; P3 q`: `P1` is the pixel aspect
  (omitted, 0, 1, 5, 6 → 2:1; 2 → 5:1; 3, 4 → 3:1; 7–9 → 1:1, the VT340
  table); `P2 = 1` keeps unset pixels transparent, otherwise they take
  colour 0 (the background); `P3` ignored.
- **Raster attributes** `" Pan ; Pad ; Ph ; Pv`: the aspect `Pan/Pad`
  (overrides P1; rounded to a whole vertical scale 1–5; `Pad = 0` is
  taken as 1) and the size, which lets the decoder allocate once. chafa
  and img2sixel send `"1;1;W;H`.
- **Colours** `# Pc ; Pu ; Px ; Py ; Pz`: select register `Pc`, or define
  it in HLS (`Pu = 1`; DEC's HLS, where hue 0° is **blue**, not red as in
  the usual HLS) or RGB percentages (`Pu = 2`). At most 1024 registers
  (cells §14); higher numbers wrap modulo 1024, as xterm does. The initial
  palette is the VT340's 16 colours, then black. Redefining a register
  doesn't recolour pixels already painted (pixels are written as colours,
  not register numbers).
- **Data** `?`…`~`: six vertical pixels in the current colour; `! Pn c`
  repeats `c` `Pn` times, clamped to the width limit (cells §14); `$` back
  to column 0 of the band; `-` the next band of six rows.
- **Growth**: without raster attributes, the buffer grows by doubling in
  each direction, up to 10,000 × 10,000 and the budget. Growing reserves
  the **whole new size** from the pool first (old and new exist together
  while pixels are copied); refused: the decoder stops writing, the rest
  of the string is skipped, and the image is dropped.
- At the end, the image is cropped to the area painted (or the raster
  size if larger, as xterm does).
- As built (I.1): with `P2 ≠ 1`, unset pixels take register 0 as it is
  at the end of the string (programs define `#0` after the raster
  attributes); painted pixels are always opaque, so alpha 0 marks unset
  ones and no mask is kept. Decoding chafa-style data (a 1000 × 1000
  photo-like image, 216 colours) runs at ~215 MB/s on an M-series Mac
  (`cargo bench -p term-image --bench sixel`), through a fast path for
  runs of single sixels at 1:1.

The decoder writes BGRA premultiplied straight into the `PixelTarget`
(§4.2; the Metal buffer the GPU reads). Decoding runs on the reader
thread, under the lock, in the 64 KB slices the reader already uses: at
~150–300 MB/s (to be measured, §9) a slice costs well under 1 ms, the same
order as parsing text.

**A vte change is needed** for "an abandoned string drops the image":
today vte calls `unhook()` with no completion flag, on ST and on CAN/SUB
alike, and calls it at the ESC before knowing whether `\` follows
(`advance_dcs_passthrough`). PR V.1 adds `dcs_end(complete: bool)` with an
escape state as the APC one has (cells §14's `apc_end`), with the default
calling `unhook()` so nothing else changes.

### 8.2 Placing it

At `ST` (`dcs_end(true)`): an own image with the decoded pixels (`Ready`
at once, no decode job), a virtual placement of `ceil(w / cell_w) ×
ceil(h / cell_h)` cells, drawn at its natural pixel size from the top-left
(not scaled to fill the last partial cell; after a cell size change, it
scales with the cells as §3.7 says), and placeholder cells written at the
cursor (cells §8.3). Then:

- **Sixel scrolling** (DECSDM, mode 80, reset = scrolling, the default):
  the image starts at the cursor and scrolls the screen as needed; the
  cursor ends on the row after the image at its left column, or, with
  mode 8452 set, right of the image on its last row (xterm's rules,
  checked against xterm in PR I.3);
- mode 80 set: the image starts at the top-left of the screen, doesn't
  scroll, and is clipped at the bottom; the cursor doesn't move.
- **Private colour registers** (mode 1070, set by default as in xterm):
  each image starts from the initial palette; reset, registers carry over
  between images (kept per terminal, 4 KB).

A string abandoned (`dcs_end(false)`, CAN/SUB) drops the image.

### 8.3 What programs ask

- **DA1** answers `ESC [ ? 62 ; 4 ; 22 c` (adds `4`, Sixel) once B.5
  lands, and only while images are on.
- **XTSMGRAPHICS** `CSI ? Pi ; Pa ; Pv S`: `Pi = 1` (colour registers):
  read → 1024; `Pi = 2` (Sixel geometry): read → the screen's size in
  pixels, read max → 10000 × 10000; set/reset answer success without
  changing anything. `Pi = 3` (ReGIS): error.
- `CSI 14 t`, `CSI 16 t` and `TIOCGWINSZ` pixel sizes are already
  answered (term.rs; term-pty).

---

## 9. Costs, and how to measure them

| What | Expected | How measured |
|---|---|---|
| kitty direct transmit, base64 + copy into pixels (`f=32`, 8 MB in 4 KB chunks) | ≥ 500 MB/s of payload, ~16 ms for a full-HD RGBA image, spread over 64 KB slices (≤ 0.2 ms each under the lock) | new criterion bench `cargo bench -p term-core --bench images` (`kitty_rgba_8mb`, `kitty_png_1mb`, `kitty_zlib`, `mpv_frames`) |
| Header sniff under the lock | < 1 µs | `cargo bench -p term-image --bench probe` |
| shm / file open and checks under the lock | 10–50 µs | `images` bench with a test host using real `shm_open` (term-app bench) |
| PNG decode, 1624×1018 screenshot | 5–15 ms, on the queue | term-app bench on fixtures |
| WebP decode, 1600×1200 lossy photo | measured 20 ms at any drawn size (decoded whole, then scaled), on the queue | `cargo bench -p term-image --bench decode` (`webp_*`) |
| Sixel decode | 150–300 MB/s | `cargo bench -p term-image --bench sixel` on chafa output of a 1000×1000 photo (checked-in fixture) |
| iTerm2 stream, 2 MB PNG | dominated by base64, ≥ 500 MB/s | `images` bench (`iterm_png_2mb`) |
| Frame build, 10 images on screen | + ≤ 10 µs | `cargo bench -p term-render --bench frame` gains `with_images` |
| GPU time, one window-sized image | + ≤ 0.1 ms | Metal counters, as the render bench does |
| Footprint: images at the budget | peak − idle ≤ 32 MB + 4 MB (the 4 MB for metadata, ImageIO's own buffers beyond the reserved headroom, and allocator slack; measured and the margin set from the numbers in I.5) | `scripts/pressure/run.py` gains `images_flood` (mpv-like: 500 id-less 1600×1000 frames) and `images_tiny` (20,000 1×1 images); `/usr/bin/footprint -p <pid>` before, at peak, after |
| Footprint with no images | unchanged (the pool and an empty `Graphics` are < 1 KB) | standard set before/after each PR |
| Binary size | + ~60 KB (miniz_oxide) + ~40 KB (decoder, sniffing, protocols); measured in I.1: +50 KB for those, +321 KB for the PNG/JPEG/GIF decoders (stripped, thin LTO); +196 KB for WebP (`image-webp`, same way); JPEG XL (`jxl-oxide`) measured at +1.54 MB and left out (§12) | `ls -l` of the release library, recorded per PR |
| Idle | 0% CPU, 0 wakeups, also with a lost image in view | `top -l 2 -s 5 -stats cpu,idlew` with an image on screen, and with the budget full of protected images and a lost one in view |
| Parse speed of text | unchanged (an APC/DCS branch only) | `cargo bench -p term-core --bench parse` within noise; the parse-speed gate (cells §17.4) |

Pressure scenarios added to `scripts/pressure/`: `images_flood`,
`images_tiny` (above), `images_chunks_abandoned` (10k uploads stopped
mid-way: budget back to zero after RIS), `sixel_huge` (a 10,000 × 10,000
raster attribute with little data: refused, no allocation),
`images_scrollback` (an image every 20 lines through 100k lines: pool
bytes stay ≤ budget, blocks free their images), `images_hidden_tab` (a
background tab transmitting 100 MB of images: the front tab's images keep
working).

---

## 10. Tests

**term-image** (Linux CI too):
- kitty keys: one row per key and value form, unknown keys, repeats,
  overflow, `i`+`I`, the 4 KB limit; the protocol document's examples as
  rows.
- base64: every split point of a payload (decoding in any slices equals
  one go), padding at a chunk's end, bad characters.
- inflate: a 1 KB stream that inflates to 1 GB is refused after the cap
  with at most the cap allocated; truncated streams.
- sniffing: PNG/GIF/JPEG/WebP (lossy, lossless, extended) sizes, EXIF
  orientations 1–8, truncated and garbage headers, TIFF/PDF/HEIC/SVG/WAV
  refused.
- WebP decoding: lossy, lossless, alpha (lossy `ALPH` and lossless),
  animated (first frame) fixtures checked by hand and against golden
  images (`tests/webp_golden.rs`), ICC tagging, limits, every truncation;
  the decoder's allocations counted against its estimate
  (`tests/decode_memory.rs`).
- iTerm2 args: every key and size form; base64 names; garbage.
- Sixel: golden images (one colour, repeats, `$` and `-`, HLS with a known
  colour checking hue 0 is blue, RGB colours, transparent vs background,
  raster attributes, `Pad = 0`, P1 omitted with no raster attributes (2:1),
  growth without raster attributes, a register redefined after painting)
  compared pixel by pixel; chafa and img2sixel outputs of three fixtures
  with expected PNGs; caps.
- **Fuzz targets** (cargo-fuzz, in `fuzz/`): `sixel` (1 MB budget: never
  panics, never allocates past the cap), `kitty_keys`, `iterm_args`,
  `inflate`, `sniff`. The nightly fuzz workflow runs them in turn (one
  per night, 20 minutes; `parse` every night).
- The `parse` target gets images on: the pure-Rust test host, a 1 MB
  budget and `set_sweep_at(8)` (so sweeps happen), and its dictionary
  gains `\x1b_G`, `a=T`, `U=1`, `d=a`, `\x1bPq`, `"1;1;`, `1337;File=`.

**term-core** (Linux CI too), with a pure-Rust test host (PNG via the
`png` crate as a dev-dependency, `Vec`-backed pixel buffers, real files in
temp dirs, a fake shared memory):
- per command: transmit/place/query replies with each `q`; **a query's
  reply comes before the DA1 reply sent right after it**; `I=` numbering;
  replace by id; every delete specifier with its scope (`d=a` leaves
  virtual placements; `d=i` removes placements in history); two `p=0`
  placements on one row; cursor movement with and without `C=1`, and
  wrapping past the last column; placements scrolling with the screen,
  inside and across scroll regions; ED 2, ED 3, RIS; the alternate
  screen; chunk continuation and abandonment; media rules through the
  host (the lock is released around `read_head`).
- an mpv-like stream (1000 id-less opaque frames at one position, no
  deletes): one image alive at the end; yazi replacing a 20 MB image by id
  with another 20 MB image under a 32 MB budget: succeeds.
- iTerm2 and Sixel: placeholder cells written, cursor after, modes 80/
  8452/1070, DA1 with and without images, XTSMGRAPHICS, nothing replied
  while images are off.
- budget, with two terminals sharing one pool: eviction classes and order
  (hidden first, unplaced next, never protected, never busy), pixels
  before encoded copies, replacement, refusal, `image_memory` lowered
  live, a hidden tab not holding the budget, `release_owner`, job numbers
  after RIS and after the session replaces its terminal, a failed
  re-decode not waking the view, a panic inside pool code (test hook)
  leaving other terminals' images working.
- property test (extends `props.rs`): random interleavings of graphics
  commands, text, scrolling, resizes, ED/RIS and snapshots keep the
  invariants: pool `used` = sum of live entries ≤ limit; every placement's
  image exists; `Refs` equal a recount; `by_id`/`by_image` match the map;
  history block image lists match their placeholder cells; no own image
  outlives its last reference after a forced sweep; caps hold.

**term-app** (macOS CI): ImageIO decoding of PNG/JPEG/GIF fixtures to
expected pixels, orientation 6, thumbnails at a requested size; the media
rules (FIFO, device, symlink escaping the temp dir, `..`, missing marker,
`t=t` removed at open, `t=s` unlinked at open, a non-local volume refused
— a disk image mounted with `nobrowse` stands in for a network volume
where a real one isn't available).

**term-render** (macOS CI, offscreen as the existing test does): a 2×2
image of four known colours placed at a cell, read back; a placeholder
image partly overwritten draws only the remaining cells; layer order; the
selection tint over an image cell; z order across two images; no bleed
from outside a source rectangle; shrinking below ½ uses the box filter.

**Pipeline and smoke** (term-app): scripts emit recorded sequences
(icat's in stream, file and memory modes, imgcat's, chafa's, yazi's,
mpv's, one through tmux passthrough — recorded once, checked in under
`tests/fixtures/images/`) and the dump (the `dump` control gains an
`images` section: images, placements, pool bytes) is checked. If `kitten`
is on `PATH`, a pipeline test runs `kitten icat
--transfer-mode=stream`, `=file` and `=memory` for real.

**Manual checklist** (docs/test-plan.md): `kitten icat` (each mode),
`imgcat` (a phone photo; an animated GIF shows its first frame), yazi
previews (200 images; footprint ≤ idle + 36 MB), `chafa --format sixel`,
`img2sixel`, lsix, image.nvim, timg, `mpv --vo=kitty` (CPU and footprint
for 60 s, window and full screen), inside tmux.

---

## 11. PR plan

The roadmap's rule: parallel agents never work on the same module. Cells
PR 14 ("placement table and decline's own images") is replaced by I.0–I.3.
Everything here comes after cells PRs 7, 8 and 10 (scrollback v2, the
snapshot, placeholders); I.2 also after cells PR 11 (tracked positions,
which placements register with); I.4 after F3.3 (frames from the
snapshot).

Images stay behind a hidden switch (`TD_IMAGES=1` in the environment,
read once at start) until I.4 and I.5a have landed, so daily builds don't
answer kitty queries or report Sixel while nothing can draw. I.7 removes
the switch.

| # | Module | PR | Size | After |
|---|---|---|---|---|
| V.1 | vte | `dcs_end(complete)` with an escape state, default calls `unhook` (§8.1) | S | — (any time) |
| I.1 | image (new) | **term-image**: `PixelTarget`/`Alloc`, kitty keys, base64, inflate (miniz_oxide), sniffing and sizes, iTerm2 args, Sixel decoder, pixel conversion; unit tests, golden images, benches, the five fuzz targets | M | — (any time) |
| I.0 | core (+ app, render: constructors only) | **interfaces**: `ImageKey`, `Image`, `Graphics` skeleton, `PixelBuffer`/`ImageHost`, `ImagePool` with reserve/evict/drawn/`release_owner` and its tests, jobs and the process-wide job counter, snapshot fields (§4.4), block image lists as `ImageKey`; the pool passed to every `Terminal::new` (session, `replace_term`, tests) and kept across `reset()`; the hidden switch. No protocol yet. Main session, since it touches three modules | M | cells 7, 8, 10; I.1's `PixelTarget` |
| I.2 | core | **kitty graphics**: APC routing, chunking, transmit/place/query/delete with scopes, replies, media through the host with the lock released, id-less images, refs and freeing, eviction wired in, caps, cursor rules, placements through scroll regions/ED/RIS/alt screen and as tracked positions; conformance and property tests | L | I.0, I.1, cells 11 |
| I.3 | core | **iTerm2 and Sixel** as own images: `osc_large` → `Stream`, the readers, decoded size for iTerm2, placeholder cells, modes 80/1070/8452, DA1 `4`, XTSMGRAPHICS | M | I.2, V.1 |
| I.4 | render | **drawing**: `MetalPixels`, textures over buffers, the image pipeline and shader (box filter, half-texel inset), layers, highlights over images (rect buffer split), z order, placeholder run merging, per-frame texture map, a test-only way to put images in a snapshot, offscreen tests, `with_images` bench | M | I.0, F3.3 (parallel with I.2) |
| I.5a | app | **host and plumbing**: the ImageIO decoder, media open/read rules, the decode queue with `catch_unwind`, `take_image_jobs` after slices and snapshots, producer hold-back, hide/show order and volatility, `image_memory` and `image_file_transfer` settings (preferences schema), the dump `images` section | M | I.0, I.4 (`MetalPixels`) (parallel with I.2) |
| I.5b | app | pipeline fixtures and tests, footprint numbers, pressure scenarios run | S | I.3, I.4, I.5a |
| I.6 | ci + fuzz | fuzz targets in the nightly rotation, the `parse` target with images on, dictionary entries, the pressure scenarios added | S | I.1, I.2 |
| I.7 | docs | DESIGN.md (Rendering, memory, idle), the manual checklist, numbers on the standard set, roadmap B.3/B.5 marked; the hidden switch removed | S | I.3, I.5b |

**Settings (after I.5a).** `images` (default on: Alex's decision, 30 Oct 2026),
`image_memory` (MB, 0 = the larger of 32 MB and two window-sized images)
and `image_file_transfer` (default on) are in the preferences schema and
all apply live (the test dump has an `images` section: enabled, count,
ready, bytes, placements_in_view, budget): `images::apply_config` runs at start and on each reload.
`TD_IMAGES=1` still turns images on whatever `images` says. Sessions
always install the app's host, so `images` can be turned on in a running
window; turning it off stops new images and queries, and what is held
leaves as the budget needs it. Installing the host costs nothing: its
Metal device is made on the first image (made with the host, it took
70 ms to seconds at every session start and made quick failures look
slow, closing their windows).

**Landed (I.6).** Fuzzing and pressure, ahead of the image code being
complete (the seeds and dictionary entries simply find nothing to do until
I.2 and I.3 land).

- **Nightly rotation**: `.github/workflows/fuzz.yml` is a matrix, one job
  per target, all of them every night (not one per night as planned:
  they run in parallel, so it costs no more wall time). `parse` gets 20
  minutes, the term-image targets (`sixel`, `kitty_keys`, `iterm_args`,
  `inflate`, `sniff`) and `decode` 5 each. Each target has its own corpus
  cache entry (`fuzz-corpus-<target>-<run>`), its own crash artifact
  (`fuzz-crash-<target>`) and its issues carry the target in the title.
  Manual runs can name targets and minutes. CI builds all targets.
- **`parse` with images on**: `fuzz/steps.rs` makes the terminal under
  test with a 4 MB pool with images enabled (not the process pool, which
  the hidden switch leaves off); `TD_FUZZ_IMAGES=0` goes back to the
  process pool. The corpus replay test shares the file, so it replays
  with images on too. Image jobs are dropped after each slice (no host).
- **Seeds**: `scripts/fuzz-seed.py` writes `parse` seeds for kitty
  graphics (all actions, chunks, id-less frames, placeholders, refused
  file and shm media), iTerm2 `File=` and Sixel, mixed with resizes,
  scrolling, selections and the alternate screen, and seeds for the five
  term-image targets. Dictionaries: `parse.dict` gains the image
  sequences; `sixel.dict`, `kitty_keys.dict` and `iterm_args.dict` are new.
- **Pressure** (`scripts/pressure/gen.py` generates them on the fly, `run.py`
  runs them with `TD_IMAGES=1`): `images_flood` (500 id-less 1600 x 1000
  PNG frames), `images_ids`, `images_tiny`, `images_chunks_abandoned`,
  `images_scrollback`, `sixel_flood`, `sixel_huge`. Not run yet:
  footprint numbers come with I.5b. `images_hidden_tab` waits for a way
  to drive tabs.

**Landed (I.0).** What was built (`term-core/src/image.rs`,
`image/pool.rs` and its tests), and where it differs from §4 and §5:
- **Graphics** is one per screen (cells §8.1), boxed in the terminal,
  each holding the pool and the terminal's id as owner; it frees its
  pool entries when dropped, so `replace_term` and RIS (which keep the
  pool) give the bytes back. Placements are keyed (top line, serial);
  `placements_in` uses `tallest` as §4.1 says; `remap_placements` is
  what reflow uses (cells PR 10). The caps are constants, not enforced
  yet (I.2).
- **ImagePool**: `reserve` (the replaced entry's bytes back first),
  `grow` for uploads, `set_pixels`/`set_encoded` (a reservation turns
  into what it holds; encoded copies count whole 64 KB pieces),
  `set_busy` (giving back what a finished job reserved and didn't use),
  `set_placed`, `drawn`, `set_shown` (purgeable buffers with an encoded
  copy while hidden; on show, buffers macOS took become lost-with-source
  and are reported as evicted), `release`, `release_owner`,
  `take_evicted`. The order index holds each idle entry's key in the
  entry, so every update is O(log n). An entry is protected when busy
  or drawn in its owner's latest frame while shown. `budget_for` gives
  §12's budget; `ImagePool::process()` is the process pool, made with
  32 MB and the hidden switch.
- **The hidden switch** is `image::images_switch()` (`TD_IMAGES=1`, read
  once) behind `ImagePool::process()`: its terminals report
  `images_enabled() == false`. Tests make their own pools, with images
  on.
- **Every `Terminal::new` and `Term::new` takes the pool**: the session
  passes the process pool, `replace_term` the old terminal's, tests and
  benches the process pool (or their own).
- **Jobs**: `ImageJob`, `JobDone`, `JobId::next()` (process-wide),
  `take_image_jobs` and `image_job_done`, which drops a job for another
  terminal or another generation; generations come from a process-wide
  counter too.
- **The snapshot** has `placements` and `virtual_placements` (empty
  until I.2), and marks evicted images lost; `drawn` is not called yet
  (nothing is drawn until I.4).
- **History blocks** list the own images their dictionary names
  (`BlockData::images`); `Scrollback::take_freed_images` hands the lists
  of freed blocks back (rather than `push` returning them). The terminal
  starts taking them with I.3, when own images exist.

**Landed (I.2).** What was built (`term-core/src/term/graphics.rs`, the
`Graphics` deletes and scrolling in `image.rs`, `image/host.rs`, tests
in `tests/kitty.rs` and a graphics property test in `tests/props.rs`),
and where it differs from §3 and §4:
- **Routing, chunks, commands, replies** as §3.1–§3.3, §3.5 and §3.6.
  Reply codes: `EINVAL`, `ENOENT`, `EBADF` (any medium error),
  `ENODATA`, `EFBIG`, `EBADPNG` (kitty's for data that doesn't decode).
  A new transmit with another medium abandons an upload in progress,
  as a delete does. Commands with `P`/`Q`/`H`/`V` or an animation
  action are dropped with no reply; icat with an animated GIF is not
  checked yet.
- **The host**: the terminal holds an `Arc<dyn ImageHost>`
  (`set_image_host`, kept across RIS); until the app sets one it is
  `RustHost::new()` (Vec buffers, term-image decoding, media refused).
  `RustHost::with_media()` is the test host (`std::fs` files, the temp
  file rules simplified to the temp directory and the name, a map for
  shared memory). `run_job(host, terminal_id, job)` runs a job;
  `ImageJob::Read` gained `size` and `compressed`, and `JobDone` carries
  the pixels and opacity.
- **Headers off the lock**: PNG from a file or shared memory is sized
  from its first 64 KB at once for `a=t` too, so every image has its
  size when stored. With `set_reads_off_lock(true)` the parser stops
  after such a command (`Term::advance_until_head`); the session takes
  `take_head_read()`, reads it unlocked and calls `head_read_done`.
  Off (the default, and tests), `Term::advance` reads it inline;
  normal parsing doesn't go through the stopping parser.
- **Budget**: reservations are made when the size is known (raw: at the
  first chunk; PNG: encoded bytes as they arrive, the pixels at the
  last chunk; media: at open), not when a job starts as §4.2 says.
  The encoded copy is not kept after decoding yet (§5.4), and the
  `ImageHost::decode` signature doesn't report opacity, so PNG images
  are never "opaque" for the id-less replacement (mpv sends `f=24`).
- **Placements**: scroll regions move those wholly inside, drop those
  scrolled out, and leave those crossing a margin (cells §8.2); the
  alternate screen drops them as they scroll off, and its images are
  cleared on entering and leaving it; ED 2 deletes those on screen, ED
  3 those in history; lines leaving history drop theirs one scroll
  late. The image cap drops the oldest unplaced image first, the
  placement cap the topmost placement.
- **Snapshot**: the shape of `SnapPlacement` and `SnapVirtual` is
  unchanged; placements reaching into the view are listed in (z, pool
  id, serial) order with their view row (negative above the view, one
  row per line), and when any row in view has placeholders, every
  virtual placement of a ready image is listed (not only those the
  cells name). The snapshot doesn't call `drawn`: the frame does (I.4).
- **Not built**: the producer hold-back and the stalled-upload stop by
  the pool (§3.2, §4.2), `image_file_transfer` (the app's host decides),
  re-decoding lost images (§5.4), images in the `dump`, the
  `images` bench (§9).

**Landed (I.4).** Drawing, in term-render (`images.rs` without Metal,
`pixels.rs`, `metal.rs`, `shaders.metal`); where it differs from §6:
- **`MetalPixels`** is a shared-storage `MTLBuffer` with rows padded to
  the device's linear texture alignment; `PixelAlloc` makes them from any
  thread (`Renderer::pixel_alloc`, or `PixelAlloc::system_default` for
  the app's decode queue). Pixels of any other type (the tests'
  `VecBuffer`) are listed but not drawn.
- **`ImageInstance` is 48 bytes**, not 32: it carries the clamp rectangle
  (the source rectangle inset by half a texel) besides the texels at the
  quad's edges, so the pieces of one image sample across their seams and
  never outside the source. Instances are clipped to the grid in f32 and
  rounded to whole pixels, the texels following the rounded edges.
- **Layers**: the frame's `rects` split at the end of the backgrounds
  (`Frame::split`); per pane: images under the backgrounds, backgrounds,
  images under the text (and placeholder pieces), highlights, block
  cursor and decorations, glyphs, images above the text, then the
  bar/underline/hollow cursor again over them. Without images the draws
  are the same as before. Highlights over cells covered by images under
  the text are drawn at half their alpha.
- **Placeholders**: a cached row keeps its runs of placeholder cells
  (merged as §6.3 says) in its row cache entry; the pieces are rebuilt
  each frame from the snapshot's `virtual_placements` (a run naming
  placement 0 takes any of the image's). A placeholder's content bits are
  no longer read as a character.
- **The texture map** is per window, keyed by (pool entry, generation),
  rebuilt each frame from the panes' images and emptied on hide.
- **`pool.drawn`** is called by the frame after building from a snapshot,
  with the images it drew (and once more when it draws none after some).
- **Test injection**: `Renderer::build_pane_with` (and
  `Frame::snapshot_mut` between `snapshot` and `build_snapshot`) put
  placements in a snapshot; no term-core change.
- **Measured** (`cargo bench -p term-render --bench frame -- with_images`,
  80×24, rows cached, on a loaded machine): no images 2.9 µs, ten
  placements 4.7 µs, a 40×20 placeholder image (20 pieces) 3.3 µs.

**Landed (I.5a, part).** The host and its plumbing, in term-app
(`images.rs`, `images/host.rs`, `images/queue.rs`); where it differs
from §3.4, §4.2 and §5.5:
- **`AppHost`** allocates `MetalPixels` through `PixelAlloc::system_default`
  and decodes with `term_image::decode` (Display P3 on), not ImageIO;
  term-core's `run_job` runs each job. The media rules are §3.4's, with
  a temporary file opened by `openat` with `O_NOFOLLOW` in its checked
  directory and removed there with `unlinkat`; relative paths are
  refused; what is read must fit the budget's limit (term-core reserves
  the bytes). No Metal device: every image is refused.
- **The queue** is one thread per process at utility QoS (a thread, not
  a dispatch queue); jobs and their completions run inside
  `catch_unwind`. Each session has its side (`SessionImages`), keyed by
  its own number rather than the terminal's id, which changes when a
  terminal is replaced after a panic; closing the session drops its
  waiting jobs, and a completion for a gone session does nothing.
- **Off-lock header reads**: with images on, the session sets the host
  and `set_reads_off_lock`, and its reader parses with
  `advance_until_head`, letting the lock go around each `read_head`.
- **Jobs** are taken after each parse slice (reader thread) and after
  each snapshot (main thread). **Hold-back**: before each PTY read, the
  reader waits while its queued encoded bytes pass a quarter of the
  budget, at most 2 s at a time, so a stuck decode can't stop output for
  good.
- **Hide and show**: `set_shown(false)` right after the renderer's `hide`
  (frames waited for, textures dropped), `set_shown(true)` right after
  its `show`, before the frame is built; once per change.
- **The budget** follows the largest window's drawable (`budget_for`),
  set on each layout and when a window closes.
- Not yet: the `image_memory` and `image_file_transfer` settings (media
  is on, the budget follows windows only), the dump's `images` section,
  and the `MNT_LOCAL` refusal tested against a real non-local volume.

**Landed (I.3).** iTerm2 and Sixel as own images
(`term-core/src/term/own_images.rs`, tests in `tests/own_images.rs`);
where it differs from §7 and §8:
- **iTerm2**: `osc_large` answers `Stream` for `1337;File=` and also for
  `1337;FilePart=` over 4 KB (streamed into the multipart reader rather
  than buffered); a part with no `MultipartFile` before it is dropped.
  Image pixels count as points (§12, 7): `Terminal::set_image_scale`
  (device pixels per point, default 1) is what the app must set from the
  window's backing scale; until it does, images are drawn one image
  pixel per device pixel. Box widths are capped at the screen's width.
  `preserveAspectRatio=1` uses a new `Fit::ContainTopLeft` (term-render
  draws it as `Contain` with no centring offset). A JPEG whose size isn't
  in the first 64 KB is sniffed from a copy of all of it.
- **Sixel**: colour registers carried over (1070 reset) are kept per
  screen. The decoder's width limit is the screen's width in pixels (and
  its height too in display mode, mode 80), so data past the right edge is
  clipped rather than allocated. One pool entry per string: each growth
  is `grow`n first, and `set_pixels` at ST settles it.
- **Freeing** (§4.3): instead of counting `screen_styles` and
  `history_blocks`, `collect_own_images` looks for live styles naming own
  images and the blocks' lists, after a style sweep or when history
  blocks are freed (run from `take_image_jobs` and before each new own
  image); `Refs` stays zero for own images.
- **XTSMGRAPHICS** replies `Ps = 2` for an unknown action and `Ps = 1`
  for `Pi = 3` and others; nothing is answered while images are off.
- **Not done**: the cursor rules weren't checked against a real xterm
  or a recording of iTerm2 3.5 (they follow §7.2 and §8.2 as written); no
  fixture from a real img2sixel or chafa run (the round trip uses
  term-image's img2sixel-style encoder); placeholder rows and columns are
  capped at 512 (a cell's 9 bits); reflow of image rows (open question
  11); the app doesn't call `set_image_scale` yet.
- **Measured** (`cargo bench -p term-core --bench parse`, before and
  after binaries alternated on a machine at load 10–19): the quietest
  paired round was plain 7.93 → 8.01 ms, coloured 25.80 → 25.63 ms, tui
  15.80 → 15.45 ms; no change to the text path itself.

V.1 and I.1 can start now. After I.0: I.2 (core), I.4 (render) at once,
then I.5a (app) once I.4's `MetalPixels` is in; I.3 follows I.2. B.3 is
done with I.2 + I.4 + I.5 (+ the iTerm2 half of I.3); B.5 with the Sixel
half of I.3.

---

## 12. Open questions (need Alex)

Decided 29 Oct 2026: 1 the budget is the larger of 32 MB and two images
the size of the largest window; 2 animation after 1.0; 3 pure-Rust
decoders, fuzzed; 4 no HEIC in 1.0; 5–8, 10, 11 as proposed; 7 iTerm2
images sized per point, as iTerm2; 9 the window stays untagged (text keeps today's colours) and each image is
converted from its own colour space (sRGB if none) to Display P3 once, when
it is decoded (built in I.1, §6.1).

Also decided 29 Oct 2026: WebP is decoded too (`image-webp`, +196 KB,
§6.1); JPEG XL only if it adds under about 300 KB. **Measured, and left
out**: `jxl-oxide` 0.12.6 (pure Rust, rayon off, no colour management
features) adds 1,614,368 bytes, **+1.54 MB** stripped (thin LTO, the
same before/after release binary as the other decoders; it brings eleven
`jxl-*` crates and `brotli-decompressor`), five times the limit. Decoding
a 1600 × 1200 photo (the JPEG fixture's image as lossy VarDCT at
distance 1.0, 31 KB) to 8-bit RGB took 70 ms on one thread (M-series
Mac, median of 15), against 20 ms for the same photo as WebP and 16 ms
as JPEG, with no scaled decoding to help when drawn small. A JPEG XL
image from a program is refused at sniffing, as before. Worth measuring
again if a smaller pure-Rust decoder appears (libjxl's `jxl-rs` is the
one to watch).

1. **Budget on large screens.** One full-screen image is about 59 MB on a
   5K display and 31 MB on a 16" MacBook Pro, and the kitty protocol asks
   for room for "a few full screen images". Keep 32 MB (kitty images
   larger than it are refused; iTerm2 images are decoded at the drawn size
   and fit), or let the budget grow with the largest window, e.g. at least
   two window-sized images?
2. **Animation.** kitty animation (`a=f/a/c`) and animated GIFs: 1.0 shows
   the first frame. Adding animation later needs a frame timer while an
   animation is visible (stopped when hidden), which the idle goal allows
   only while something visibly moves. In 1.0 or after?
3. **Who decodes images from programs?** ImageIO inside the app (a bad
   image could crash every window; sniffing in Rust narrows it), pure-Rust
   PNG/JPEG/GIF decoders (about +100–150 KB, fuzzed), or a separate
   sandboxed decoding process (safest, more work and memory)?
4. **HEIC.** iPhone photos are HEIC; with PNG/JPEG/GIF only, `imgcat` of
   one shows nothing. Add HEIC (ImageIO decodes it), or keep the cell
   design's list?
5. **File and shared-memory transfers on by default?** On is what icat
   and mpv expect locally; a remote program can still learn whether a
   local image file exists and roughly its size (as in kitty).
6. **Large kitty images drawn small** (image.nvim may send full-size
   images into small boxes): is the 2×2 box filter enough, or keep a
   half-size copy (counted against the budget) for images drawn below ¼?
7. **iTerm2 image size on Retina.** One image pixel per screen pixel (as
   kitty and Sixel here; images look half the size they do in iTerm2), or
   per point, as iTerm2?
8. **Budget fairness.** Hidden windows' images go first; a shown window
   can still evict another shown window's off-screen images. Acceptable,
   or guarantee each window a share (e.g. 4 MB)?
9. **Colour space.** Tag the window's layer as sRGB so photos (and text
   colours) aren't oversaturated on P3 displays? It changes how every
   colour looks, which the current "no P3 tagging" choice in DESIGN.md
   was about.
10. **yazi's detection.** With `TERM_PROGRAM=terminal-decline`, yazi should
    find kitty graphics through `a=q`; if it doesn't, it needs a small
    upstream change (checked in I.5b's manual run). Fine to send one?
11. **Reflowing image rows** (cells risk 9): wrap in pieces as WezTerm, or
    clip rows that are all placeholder cells? Decide after trying yazi and
    imgcat with window resizes.
