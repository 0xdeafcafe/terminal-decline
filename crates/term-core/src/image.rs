//! Image state (images.md §4): image keys, a screen's [`Graphics`], the
//! host traits the app implements, the jobs that run off the lock, and
//! the snapshot's image fields.
//!
//! I.0 builds the interfaces only: no protocol reaches them yet (kitty
//! graphics is I.2, iTerm2 and Sixel I.3), and images stay behind the
//! hidden switch ([`images_switch`]) until drawing and the host land.

mod host;
pub mod placeholder;
mod pool;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use term_image::PixelTarget;

pub use host::{RustHost, run_job};
pub use pool::{Encoded, ImagePool, PoolId, Refused};

/// kitty ids, id-less kitty images and decline's own (Sixel, iTerm2) are
/// separate namespaces. The placeholder style's `OWN_IMAGE` attribute marks
/// ours (cells §8.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum ImageKey {
    Kitty(u32),
    Anonymous(u32),
    Own(u32),
}

/// The image a placeholder cell's style names (cells §7): the foreground
/// colour (24 bits of RGB, or a palette index) and bits 24–31 from
/// `image_id_high`. Decline's own images (`OWN_IMAGE`) are named the same way.
pub fn placeholder_image_id(style: &crate::Style) -> u32 {
    let low = match style.fg.kind() {
        crate::ColorKind::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
        crate::ColorKind::Indexed(i) => i as u32,
        crate::ColorKind::Default => 0,
    };
    low | (style.image_id_high as u32) << 24
}

/// The image a placeholder style names, as a key.
pub fn placeholder_key(style: &crate::Style) -> ImageKey {
    let id = placeholder_image_id(style);
    if style.attrs.contains(crate::Attrs::OWN_IMAGE) {
        ImageKey::Own(id)
    } else {
        ImageKey::Kitty(id)
    }
}

/// Where an image is in its life.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageState {
    /// Being received, read or decoded.
    Pending,
    Ready,
    /// Evicted: drawn as nothing (decoded again when drawn, if the pool
    /// kept its encoded copy, §5.4).
    Lost,
    Failed,
}

/// Who needs an image (§4.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Refs {
    /// Direct and virtual placements (kitty images only).
    pub placements: u32,
    /// Interned placeholder styles naming it (own images only).
    pub screen_styles: u32,
    /// History blocks listing it (own images only).
    pub history_blocks: u32,
}

/// One image of a terminal.
#[derive(Clone, Debug)]
pub struct Image {
    pub key: ImageKey,
    /// kitty `I=`, 0 if none.
    pub number: u32,
    pub width: u32,
    pub height: u32,
    /// No alpha: an id-less placement of it may replace one under it (§3.3).
    pub opaque: bool,
    pub state: ImageState,
    /// Its entry in the process-wide pool (§5), once it has one.
    pub pool: Option<PoolId>,
    /// Unique in the process, so a job finishing after a reset or a
    /// replacement can't land on a new image of the same key.
    pub generation: u32,
    pub refs: Refs,
}

static GENERATIONS: AtomicU32 = AtomicU32::new(1);

/// A fresh image generation, unique in the process.
pub fn next_generation() -> u32 {
    GENERATIONS.fetch_add(1, Ordering::Relaxed)
}

impl Image {
    pub fn new(key: ImageKey, width: u32, height: u32) -> Image {
        Image {
            key,
            number: 0,
            width,
            height,
            opaque: false,
            state: ImageState::Pending,
            pool: None,
            generation: next_generation(),
            refs: Refs::default(),
        }
    }
}

/// Which layer a placement is drawn in (cells §8.1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum Layer {
    /// Under non-default cell backgrounds.
    BelowBackground,
    BelowText,
    #[default]
    AboveText,
}

impl Layer {
    /// kitty's rule: z below −2³⁰ is under the backgrounds, other
    /// negative z under the text.
    pub fn of_z(z: i32) -> Layer {
        match z {
            z if z < -(1 << 30) => Layer::BelowBackground,
            z if z < 0 => Layer::BelowText,
            _ => Layer::AboveText,
        }
    }
}

/// How an image fills a virtual placement's cell box.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Fit {
    /// kitty: scaled to fit, keeping its aspect, centred.
    #[default]
    Contain,
    /// Stretched to the box (iTerm2 `preserveAspectRatio=0`).
    Stretch,
    /// One image pixel per device pixel from the top left (Sixel).
    Actual,
}

/// A kitty direct placement, anchored at a stable line number (cells §8.2).
#[derive(Clone, Debug)]
pub struct Placement {
    pub image: ImageKey,
    /// kitty placement id (0 = none).
    pub id: u32,
    /// Top-left cell, as a stable line and a column.
    pub line: u64,
    pub col: u16,
    /// Size in cells.
    pub cols: u16,
    pub rows: u16,
    /// Pixel offset inside the top-left cell (kitty `X`, `Y`).
    pub offset: (u16, u16),
    /// Source rectangle in image pixels: x, y, width, height.
    pub src: [u32; 4],
    pub z: i32,
}

/// A kitty `U=1` placement, or decline's own for a Sixel or iTerm2 image:
/// shown through placeholder cells only.
#[derive(Clone, Debug)]
pub struct VirtualPlacement {
    pub image: ImageKey,
    pub id: u32,
    /// The cell box.
    pub cols: u16,
    pub rows: u16,
    pub fit: Fit,
}

/// Caps on metadata (§4.1): past them the oldest unreferenced image or the
/// oldest placement goes first.
pub const MAX_IMAGES: usize = 4096;
pub const MAX_PLACEMENTS: usize = 4096;
/// Rows of one placement, so `tallest` can't make a snapshot scan history.
pub const MAX_PLACEMENT_ROWS: u16 = 1000;

/// One screen's images and where they are shown. The primary and the
/// alternate screen each have one, as kitty does. A skeleton in I.0: it
/// holds and frees images and placements; the protocols fill it from I.2.
pub struct Graphics {
    images: HashMap<ImageKey, Image>,
    /// kitty `I=` → the newest id given to it.
    numbers: HashMap<u32, u32>,
    /// By (top line, serial), so several placements may start on one row.
    placements: BTreeMap<(u64, u64), Placement>,
    /// (image, placement id ≠ 0) → its key in `placements`, for replacing.
    by_id: HashMap<(ImageKey, u32), (u64, u64)>,
    /// Every placement of an image, for deleting by image and freeing.
    by_image: HashMap<ImageKey, Vec<(u64, u64)>>,
    /// The most rows of any placement since the map was last empty.
    tallest: u16,
    virtual_placements: HashMap<(ImageKey, u32), VirtualPlacement>,
    next_id: u32,
    next_anonymous: u32,
    next_own: u32,
    next_serial: u64,
    /// The terminal's `terminal_id`: whose entries these are in the pool.
    owner: u64,
    pool: Arc<ImagePool>,
}

impl fmt::Debug for Graphics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Graphics")
            .field("images", &self.images.len())
            .field("placements", &self.placements.len())
            .field("virtual_placements", &self.virtual_placements.len())
            .finish()
    }
}

impl Graphics {
    pub fn new(owner: u64, pool: Arc<ImagePool>) -> Graphics {
        Graphics {
            images: HashMap::new(),
            numbers: HashMap::new(),
            placements: BTreeMap::new(),
            by_id: HashMap::new(),
            by_image: HashMap::new(),
            tallest: 0,
            virtual_placements: HashMap::new(),
            next_id: 1,
            next_anonymous: 1,
            next_own: 1,
            next_serial: 1,
            owner,
            pool,
        }
    }

    pub fn pool(&self) -> &Arc<ImagePool> {
        &self.pool
    }

    pub fn owner(&self) -> u64 {
        self.owner
    }

    pub fn image(&self, key: ImageKey) -> Option<&Image> {
        self.images.get(&key)
    }

    pub fn image_mut(&mut self, key: ImageKey) -> Option<&mut Image> {
        self.images.get_mut(&key)
    }

    pub fn images(&self) -> impl Iterator<Item = &Image> {
        self.images.values()
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.placements.is_empty() && self.virtual_placements.is_empty()
    }

    /// A key for a new image: kitty's own ids come from the program; these
    /// are for kitty images sent with only `I=`, id-less ones and ours.
    pub fn new_key(&mut self, kind: fn(u32) -> ImageKey) -> ImageKey {
        let counter = match kind(0) {
            ImageKey::Kitty(_) => &mut self.next_id,
            ImageKey::Anonymous(_) => &mut self.next_anonymous,
            ImageKey::Own(_) => &mut self.next_own,
        };
        let n = *counter;
        *counter = counter.wrapping_add(1).max(1);
        kind(n)
    }

    /// The newest kitty id given to image number `number`.
    pub fn by_number(&self, number: u32) -> Option<u32> {
        self.numbers.get(&number).copied()
    }

    /// Add `image`, replacing (and freeing) one with the same key.
    pub fn insert(&mut self, image: Image) {
        if self.images.contains_key(&image.key) {
            self.remove(image.key);
        }
        if let (ImageKey::Kitty(id), n) = (image.key, image.number)
            && n != 0
        {
            self.numbers.insert(n, id);
        }
        self.images.insert(image.key, image);
    }

    /// Free an image, its placements and its pool entry.
    pub fn remove(&mut self, key: ImageKey) -> Option<Image> {
        for k in self.by_image.remove(&key).unwrap_or_default() {
            if let Some(p) = self.placements.remove(&k)
                && p.id != 0
            {
                self.by_id.remove(&(key, p.id));
            }
        }
        self.virtual_placements.retain(|k, _| k.0 != key);
        let image = self.images.remove(&key)?;
        if let Some(id) = image.pool {
            self.pool.release(id);
        }
        if image.number != 0 && self.numbers.get(&image.number) == Some(&key_id(key)) {
            self.numbers.remove(&image.number);
        }
        if self.placements.is_empty() {
            self.tallest = 0;
        }
        Some(image)
    }

    /// Add a direct placement; one with the same nonzero id on the same
    /// image is replaced. Returns its key.
    pub fn place(&mut self, mut p: Placement) -> (u64, u64) {
        if p.id != 0
            && let Some(old) = self.by_id.get(&(p.image, p.id)).copied()
        {
            self.unplace(old);
        }
        p.rows = p.rows.min(MAX_PLACEMENT_ROWS);
        while self.placements.len() >= MAX_PLACEMENTS {
            // The cap (§4.1): the topmost placement goes.
            let Some(first) = self.placements.keys().next().copied() else {
                break;
            };
            self.unplace(first);
        }
        let key = (p.line, self.next_serial);
        self.next_serial += 1;
        self.tallest = self.tallest.max(p.rows);
        if p.id != 0 {
            self.by_id.insert((p.image, p.id), key);
        }
        self.by_image.entry(p.image).or_default().push(key);
        if let Some(i) = self.images.get_mut(&p.image) {
            i.refs.placements += 1;
        }
        let image = p.image;
        self.placements.insert(key, p);
        self.sync_placed(image);
        key
    }

    /// Remove a direct placement. An id-less image goes with its last one
    /// (§4.3).
    pub fn unplace(&mut self, key: (u64, u64)) -> Option<Placement> {
        let p = self.placements.remove(&key)?;
        if p.id != 0 {
            self.by_id.remove(&(p.image, p.id));
        }
        if let Some(v) = self.by_image.get_mut(&p.image) {
            v.retain(|k| *k != key);
            if v.is_empty() {
                self.by_image.remove(&p.image);
            }
        }
        if let Some(i) = self.images.get_mut(&p.image) {
            i.refs.placements = i.refs.placements.saturating_sub(1);
            if matches!(p.image, ImageKey::Anonymous(_)) && i.refs.placements == 0 {
                self.remove(p.image);
            } else {
                self.sync_placed(p.image);
            }
        }
        if self.placements.is_empty() {
            self.tallest = 0;
        }
        Some(p)
    }

    /// Placements whose rows intersect lines `top..bottom`.
    pub fn placements_in(&self, top: u64, bottom: u64) -> impl Iterator<Item = (&(u64, u64), &Placement)> {
        let from = top.saturating_sub(self.tallest as u64);
        self.placements
            .range((from, 0)..(bottom, 0))
            .filter(move |(_, p)| p.line + p.rows as u64 > top)
    }

    pub fn placements(&self) -> impl Iterator<Item = (&(u64, u64), &Placement)> {
        self.placements.iter()
    }

    /// Drop the placements whose bottom row is above `first` (their lines
    /// left history).
    pub fn drop_above(&mut self, first: u64) {
        let gone: Vec<_> = self
            .placements
            .range(..(first, 0))
            .filter(|(_, p)| p.line + p.rows as u64 <= first)
            .map(|(k, _)| *k)
            .collect();
        for k in gone {
            self.unplace(k);
        }
    }

    /// Move the placements whose top line is `from` or later to where `f`
    /// says their top-left cell went (reflow, §12.2), or drop them when it
    /// says None.
    pub fn remap_placements(&mut self, from: u64, mut f: impl FnMut(u64, u16) -> Option<(u64, u16)>) {
        let keys: Vec<(u64, u64)> = self.placements.range((from, 0)..).map(|(k, _)| *k).collect();
        for old in keys {
            let p = &self.placements[&old];
            match f(p.line, p.col) {
                Some((line, col)) if (line, col) == (p.line, p.col) => {}
                Some((line, col)) => {
                    let mut p = self.placements.remove(&old).expect("placement");
                    p.line = line;
                    p.col = col;
                    let new = (line, old.1);
                    if p.id != 0 {
                        self.by_id.insert((p.image, p.id), new);
                    }
                    if let Some(v) = self.by_image.get_mut(&p.image) {
                        for k in v.iter_mut().filter(|k| **k == old) {
                            *k = new;
                        }
                    }
                    self.placements.insert(new, p);
                }
                None => {
                    self.unplace(old);
                }
            }
        }
    }

    pub fn set_virtual(&mut self, v: VirtualPlacement) {
        let key = (v.image, v.id);
        if self.virtual_placements.insert(key, v).is_none()
            && let Some(i) = self.images.get_mut(&key.0)
            && !matches!(key.0, ImageKey::Own(_))
        {
            i.refs.placements += 1;
            self.sync_placed(key.0);
        }
    }

    /// The virtual placement placeholder cells for `image` with placement
    /// id `id` use: that one, or any of the image's when `id` is 0.
    pub fn virtual_for(&self, image: ImageKey, id: u32) -> Option<&VirtualPlacement> {
        if id != 0 {
            return self.virtual_placements.get(&(image, id));
        }
        self.virtual_placements.values().filter(|v| v.image == image).min_by_key(|v| v.id)
    }

    /// Whether any direct placement exists (a cheap test for the scroll
    /// paths).
    pub fn has_placements(&self) -> bool {
        !self.placements.is_empty()
    }

    pub fn virtual_placements(&self) -> impl Iterator<Item = &VirtualPlacement> {
        self.virtual_placements.values()
    }

    /// Tell the pool whether `key` is placed (its eviction class, §5.3).
    fn sync_placed(&self, key: ImageKey) {
        if let Some(i) = self.images.get(&key)
            && let Some(id) = i.pool
        {
            self.pool.set_placed(id, i.refs.placements > 0);
        }
    }

    /// Delete the direct placements `f` picks among those whose top line
    /// is in `lines` (all if None). With `free`, kitty images left with
    /// no placement go too (upper-case deletes, §3.5); id-less ones go
    /// with their last placement anyway.
    pub fn delete_placements(&mut self, lines: Option<(u64, u64)>, free: bool, mut f: impl FnMut(&Placement) -> bool) {
        let keys: Vec<(u64, u64)> = match lines {
            Some((top, bottom)) => self.placements_in(top, bottom).filter(|(_, p)| f(p)).map(|(k, _)| *k).collect(),
            None => self.placements.iter().filter(|(_, p)| f(p)).map(|(k, _)| *k).collect(),
        };
        let mut images = Vec::new();
        for k in keys {
            if let Some(p) = self.unplace(k) {
                images.push(p.image);
            }
        }
        if free {
            images.sort_unstable();
            images.dedup();
            for key in images {
                self.free_if_unplaced(key);
            }
        }
    }

    /// Delete the placements (direct, everywhere, and virtual) of the
    /// images `f` picks, only placement `placement` if not 0; with `free`,
    /// images left with none go too, placed or never placed (§3.5 `i`,
    /// `n`, `r`).
    pub fn delete_images(&mut self, placement: u32, free: bool, f: impl Fn(ImageKey) -> bool) {
        let mut keys: Vec<ImageKey> = self.images.keys().copied().filter(|k| f(*k)).collect();
        keys.sort_unstable();
        for key in keys {
            let direct: Vec<(u64, u64)> = self
                .by_image
                .get(&key)
                .map(|v| {
                    v.iter()
                        .copied()
                        .filter(|k| placement == 0 || self.placements[k].id == placement)
                        .collect()
                })
                .unwrap_or_default();
            for k in direct {
                self.unplace(k);
            }
            let virtuals: Vec<(ImageKey, u32)> = self
                .virtual_placements
                .keys()
                .copied()
                .filter(|(i, id)| *i == key && (placement == 0 || *id == placement))
                .collect();
            for v in virtuals {
                self.unset_virtual(v);
            }
            if free {
                self.free_if_unplaced(key);
            }
        }
    }

    /// Remove a virtual placement.
    pub fn unset_virtual(&mut self, key: (ImageKey, u32)) -> Option<VirtualPlacement> {
        let v = self.virtual_placements.remove(&key)?;
        if !matches!(key.0, ImageKey::Own(_))
            && let Some(i) = self.images.get_mut(&key.0)
        {
            i.refs.placements = i.refs.placements.saturating_sub(1);
            if matches!(key.0, ImageKey::Anonymous(_)) && i.refs.placements == 0 {
                self.remove(key.0);
                return Some(v);
            }
        }
        self.sync_placed(key.0);
        Some(v)
    }

    fn free_if_unplaced(&mut self, key: ImageKey) {
        if !matches!(key, ImageKey::Own(_)) && self.images.get(&key).is_some_and(|i| i.refs.placements == 0) {
            self.remove(key);
        }
    }

    /// Make room for one more image under the cap (§4.1): the oldest
    /// unplaced image goes first, else the oldest.
    pub fn make_room_for_image(&mut self) {
        while self.images.len() >= MAX_IMAGES {
            let victim = self
                .images
                .values()
                .filter(|i| !matches!(i.key, ImageKey::Own(_)))
                .min_by_key(|i| (i.refs.placements > 0, i.generation))
                .map(|i| i.key);
            match victim {
                Some(k) => {
                    self.remove(k);
                }
                None => break,
            }
        }
    }

    /// A kitty id not in use, from the counter (`I=` with no `i`).
    pub fn free_kitty_id(&mut self) -> u32 {
        loop {
            let key = self.new_key(ImageKey::Kitty);
            if !self.images.contains_key(&key) {
                return key_id(key);
            }
        }
    }

    /// Scroll the placements entirely inside screen rows `top..=bottom`
    /// by `n` rows, up or down, with the screen's first line at
    /// `top_line` before the scroll and `new_top_line` after (cells §8.2):
    /// those scrolled out of the region go, unless they scroll into
    /// history (`to_history`, a region at the top of the primary screen);
    /// those below the region keep their row, so their line follows the
    /// screen's; those crossing a margin stay where they are.
    #[allow(clippy::too_many_arguments)]
    pub fn scroll_region(
        &mut self,
        top_line: u64,
        new_top_line: u64,
        lines: usize,
        (top, bottom): (usize, usize),
        n: usize,
        up: bool,
        to_history: bool,
    ) {
        let keys: Vec<(u64, u64)> = self.placements_in(top_line, top_line + lines as u64).map(|(k, _)| *k).collect();
        for k in keys {
            let (old_line, rows) = (self.placements[&k].line, self.placements[&k].rows);
            if old_line < top_line {
                continue;
            }
            let r0 = (old_line - top_line) as usize;
            let r1 = r0 + rows.max(1) as usize - 1;
            let new_row: Option<isize> = if r0 >= top && r1 <= bottom {
                if up {
                    let to = r0 as isize - n as isize;
                    if to < top as isize && !to_history { None } else { Some(to) }
                } else if r1 + n > bottom {
                    None
                } else {
                    Some((r0 + n) as isize)
                }
            } else if r0 > bottom {
                Some(r0 as isize)
            } else {
                continue;
            };
            match new_row {
                None => {
                    self.unplace(k);
                }
                Some(row) => {
                    let line = (new_top_line as i64 + row as i64).max(0) as u64;
                    if line != old_line {
                        self.move_placement(k, line);
                    }
                }
            }
        }
    }

    fn move_placement(&mut self, old: (u64, u64), line: u64) {
        let Some(mut p) = self.placements.remove(&old) else { return };
        p.line = line;
        let new = (line, old.1);
        if p.id != 0 {
            self.by_id.insert((p.image, p.id), new);
        }
        if let Some(v) = self.by_image.get_mut(&p.image) {
            for k in v.iter_mut().filter(|k| **k == old) {
                *k = new;
            }
        }
        self.placements.insert(new, p);
    }

    /// Free everything (ED 3 of history placements is the caller's; this
    /// is RIS, and leaving the alternate screen).
    pub fn clear(&mut self) {
        let keys: Vec<ImageKey> = self.images.keys().copied().collect();
        for k in keys {
            self.remove(k);
        }
        self.placements.clear();
        self.by_id.clear();
        self.by_image.clear();
        self.virtual_placements.clear();
        self.numbers.clear();
        self.tallest = 0;
    }

    /// Mark the images the pool evicted since the last call `Lost`
    /// (§5.2), forgetting entries the pool dropped altogether.
    pub fn apply_evictions(&mut self) {
        let mut ids = Vec::new();
        self.pool.take_evicted(self.owner, &mut ids);
        if ids.is_empty() {
            return;
        }
        for image in self.images.values_mut() {
            if let Some(id) = image.pool
                && ids.contains(&id)
            {
                image.state = ImageState::Lost;
                if !self.pool.contains(id) {
                    image.pool = None;
                }
            }
        }
    }

    /// Check the maps agree with each other.
    pub fn check(&self) -> Result<(), String> {
        let mut count = 0;
        for (image, keys) in &self.by_image {
            for k in keys {
                match self.placements.get(k) {
                    Some(p) if p.image == *image => count += 1,
                    _ => return Err(format!("by_image {image:?} names a missing placement {k:?}")),
                }
            }
        }
        if count != self.placements.len() {
            return Err(format!("{} placements, {count} in by_image", self.placements.len()));
        }
        for (k, key) in &self.by_id {
            if self.placements.get(key).is_none_or(|p| (p.image, p.id) != *k) {
                return Err(format!("by_id {k:?} is stale"));
            }
        }
        if self.images.len() > MAX_IMAGES || self.placements.len() > MAX_PLACEMENTS {
            return Err(format!(
                "{} images, {} placements: past the caps",
                self.images.len(),
                self.placements.len()
            ));
        }
        for (key, image) in &self.images {
            let virtuals = if matches!(key, ImageKey::Own(_)) {
                0
            } else {
                self.virtual_placements.keys().filter(|v| v.0 == *key).count()
            };
            let direct = self.by_image.get(key).map_or(0, |v| v.len());
            if image.refs.placements as usize != direct + virtuals {
                return Err(format!(
                    "{key:?} counts {} placements, has {}",
                    image.refs.placements,
                    direct + virtuals
                ));
            }
        }
        for (k, p) in &self.placements {
            if !self.images.contains_key(&p.image) {
                return Err(format!("placement {k:?} of a missing image {:?}", p.image));
            }
            if k.0 != p.line || p.rows > self.tallest {
                return Err(format!(
                    "placement {k:?} at line {} with {} rows (tallest {})",
                    p.line, p.rows, self.tallest
                ));
            }
        }
        Ok(())
    }
}

fn key_id(key: ImageKey) -> u32 {
    match key {
        ImageKey::Kitty(n) | ImageKey::Anonymous(n) | ImageKey::Own(n) => n,
    }
}

impl Drop for Graphics {
    fn drop(&mut self) {
        for image in self.images.values() {
            if let Some(id) = image.pool {
                self.pool.release(id);
            }
        }
    }
}

/// Pixels a host allocated: a shared-storage Metal buffer in the app, a
/// `Vec` in tests. Premultiplied BGRA8, written through
/// [`term_image::PixelTarget`].
pub trait PixelBuffer: term_image::PixelTarget + Send + Sync {
    /// What macOS really holds for it (whole pages): what the pool counts.
    fn allocated_size(&self) -> usize;
    /// Make it purgeable (hidden windows, §5.5) or not. False when turning
    /// it back finds macOS already took it.
    fn set_volatile(&self, volatile: bool) -> bool;
    /// For the renderer, to get its Metal buffer back.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// How a kitty transfer names its data (§3.4).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Medium {
    /// `t=f`: a file, read where it is.
    File,
    /// `t=t`: a temporary file, deleted after reading.
    TempFile,
    /// `t=s`: a POSIX shared memory object.
    SharedMemory,
}

/// A request to open a file or shared memory object, made at once, under
/// the lock (§3.4).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MediumRequest {
    pub medium: Medium,
    /// The path or shared memory name, as sent (decoded from base64).
    pub name: Vec<u8>,
    /// kitty `O=` and `S=`: where to start, and how much (0 = to the end).
    pub offset: u64,
    pub size: u64,
}

/// An opened, checked medium, read later with the lock released.
pub struct OpenMedium {
    /// How many bytes it holds from the requested offset.
    pub len: u64,
    /// The host's handle (a file descriptor, a mapping).
    pub handle: Box<dyn std::any::Any + Send + Sync>,
}

impl fmt::Debug for OpenMedium {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenMedium").field("len", &self.len).finish_non_exhaustive()
    }
}

/// Why an image operation failed; kitty replies name these (§3.6).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImageError {
    /// `ENOENT`: no such image, file or object.
    NotFound,
    /// `EPERM`: a file the rules refuse (§3.4).
    Refused,
    /// `EFBIG`: bigger than the budget or the caps.
    TooBig,
    /// `EBADF`/`EINVAL`: data that doesn't decode.
    BadData(&'static str),
    /// `EIO`: the read failed.
    Io,
}

/// Implemented by the app (ImageIO, files, shared memory) and by a
/// pure-Rust host in tests.
pub trait ImageHost: Send + Sync {
    fn alloc(&self, width: u32, height: u32) -> Option<Box<dyn PixelBuffer>>;
    /// Open and check a medium at once (§3.4).
    fn open_medium(&self, req: &MediumRequest) -> Result<OpenMedium, ImageError>;
    /// Read up to `max` bytes, with the terminal's lock released.
    fn read_head(&self, medium: &OpenMedium, max: usize) -> Result<Vec<u8>, ImageError>;
    /// Decode `bytes` into `into`, no larger than `max_size`.
    fn decode(&self, bytes: &[u8], into: &mut dyn PixelBuffer, max_size: (u32, u32)) -> Result<(), ImageError>;
}

/// A job number, unique in the process.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct JobId(pub u64);

static JOB_IDS: AtomicU64 = AtomicU64::new(1);

impl JobId {
    pub fn next() -> JobId {
        JobId(JOB_IDS.fetch_add(1, Ordering::Relaxed))
    }
}

/// What happens once a medium is read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AfterRead {
    /// Decode it (PNG and the other formats).
    Decode,
    /// Raw pixels (kitty `f=24`/`f=32`), converted into the buffer.
    Raw { has_alpha: bool },
}

/// Work queued on a terminal for the session to run off the lock (§4.2).
#[derive(Debug)]
pub enum ImageJob {
    Decode {
        job: JobId,
        image: ImageKey,
        generation: u32,
        bytes: Arc<Encoded>,
        size: (u32, u32),
    },
    Read {
        job: JobId,
        image: ImageKey,
        generation: u32,
        medium: OpenMedium,
        then: AfterRead,
        /// The image's size (raw: `s` × `v`; encoded: from its header).
        size: (u32, u32),
        /// kitty `o=z`: the data is zlib-compressed.
        compressed: bool,
    },
}

impl ImageJob {
    pub fn id(&self) -> JobId {
        match self {
            ImageJob::Decode { job, .. } | ImageJob::Read { job, .. } => *job,
        }
    }
}

/// A finished job, handed back with [`crate::Terminal::image_job_done`].
pub struct JobDone {
    pub job: JobId,
    /// The terminal that queued it; a job for another is dropped.
    pub terminal_id: u64,
    pub image: ImageKey,
    pub generation: u32,
    /// The pixels, allocated by the host when the job ran.
    pub result: Result<Arc<dyn PixelBuffer>, ImageError>,
    /// Every pixel had alpha 255 (known for raw data only).
    pub opaque: bool,
}

impl fmt::Debug for JobDone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobDone")
            .field("job", &self.job)
            .field("terminal_id", &self.terminal_id)
            .field("image", &self.image)
            .field("generation", &self.generation)
            .field("ok", &self.result.is_ok())
            .finish()
    }
}

/// An image as the renderer gets it: no pool lock needed to reach its
/// pixels.
#[derive(Clone)]
pub struct SnapImage {
    pub pool: PoolId,
    pub generation: u32,
    pub pixels: Arc<dyn PixelBuffer>,
    pub width: u32,
    pub height: u32,
}

impl fmt::Debug for SnapImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapImage")
            .field("pool", &self.pool)
            .field("generation", &self.generation)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

/// A direct placement in view.
#[derive(Clone, Debug)]
pub struct SnapPlacement {
    pub image: SnapImage,
    /// Top-left cell relative to the view (the row may be negative).
    pub view_row: i32,
    pub col: u16,
    pub cols: u16,
    pub rows: u16,
    pub offset: (u16, u16),
    pub src: [u32; 4],
    pub layer: Layer,
    pub z: i32,
    pub serial: u64,
}

/// A virtual placement that placeholder cells in view refer to.
#[derive(Clone, Debug)]
pub struct SnapVirtual {
    pub key: (ImageKey, u32),
    pub image: SnapImage,
    pub cols: u16,
    pub rows: u16,
    pub fit: Fit,
}

/// The hidden switch (images.md §11): images are off unless `TD_IMAGES=1`
/// was in the environment when this was first asked. Off, terminals don't
/// answer kitty graphics, report Sixel or keep iTerm2 files.
pub fn images_switch() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("TD_IMAGES").is_some_and(|v| v == "1"))
}

/// A `Vec`-backed [`PixelBuffer`] counted at 16 KB pages, as Metal counts
/// its buffers: for tests and the pure-Rust test host.
pub struct VecBuffer {
    pixels: term_image::VecPixels,
    volatile: std::sync::atomic::AtomicBool,
}

/// The page size buffers are counted in.
pub const PAGE: usize = 16 * 1024;

impl VecBuffer {
    pub fn new(width: u32, height: u32) -> VecBuffer {
        VecBuffer {
            pixels: term_image::VecPixels::new(width, height),
            volatile: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn pixels(&self) -> &term_image::VecPixels {
        &self.pixels
    }
}

impl term_image::PixelTarget for VecBuffer {
    fn width(&self) -> u32 {
        self.pixels.width()
    }
    fn height(&self) -> u32 {
        self.pixels.height()
    }
    fn row_pitch(&self) -> usize {
        self.pixels.row_pitch()
    }
    fn bytes_mut(&mut self) -> &mut [u8] {
        self.pixels.bytes_mut()
    }
}

impl PixelBuffer for VecBuffer {
    fn allocated_size(&self) -> usize {
        let bytes = self.pixels.row_pitch() * self.pixels.height() as usize;
        bytes.div_ceil(PAGE).max(1) * PAGE
    }
    fn set_volatile(&self, volatile: bool) -> bool {
        self.volatile.store(volatile, Ordering::Relaxed);
        true
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graphics() -> Graphics {
        Graphics::new(1, ImagePool::new(32 << 20))
    }

    fn placement(image: ImageKey, id: u32, line: u64, rows: u16) -> Placement {
        Placement {
            image,
            id,
            line,
            col: 0,
            cols: 4,
            rows,
            offset: (0, 0),
            src: [0, 0, 10, 10],
            z: 0,
        }
    }

    #[test]
    fn layers_follow_z() {
        assert_eq!(Layer::of_z(0), Layer::AboveText);
        assert_eq!(Layer::of_z(-1), Layer::BelowText);
        assert_eq!(Layer::of_z(-(1 << 30)), Layer::BelowText);
        assert_eq!(Layer::of_z(-(1 << 30) - 1), Layer::BelowBackground);
    }

    #[test]
    fn job_ids_and_generations_are_unique() {
        let a = JobId::next();
        let b = JobId::next();
        assert!(b > a);
        assert_ne!(next_generation(), next_generation());
    }

    #[test]
    fn placements_by_id_replace_and_zero_ids_add() {
        let mut g = graphics();
        let k = ImageKey::Kitty(7);
        g.insert(Image::new(k, 10, 10));
        g.place(placement(k, 1, 5, 2));
        g.place(placement(k, 1, 9, 2));
        g.place(placement(k, 0, 5, 2));
        g.place(placement(k, 0, 5, 2));
        g.check().unwrap();
        assert_eq!(g.placements().count(), 3);
        assert_eq!(g.image(k).unwrap().refs.placements, 3);
        assert_eq!(g.placements_in(9, 10).count(), 1);
        assert_eq!(g.placements_in(6, 7).count(), 2, "rows 5–6 reach line 6");
        g.remove(k);
        g.check().unwrap();
        assert!(g.is_empty());
    }

    #[test]
    fn tall_placements_above_the_view_are_found() {
        let mut g = graphics();
        let k = ImageKey::Kitty(1);
        g.insert(Image::new(k, 10, 10));
        g.place(placement(k, 0, 0, 50));
        g.place(placement(k, 0, 10, 2));
        assert_eq!(g.placements_in(40, 60).count(), 1);
        g.drop_above(20);
        assert_eq!(g.placements().count(), 1, "the tall one reaches line 49");
        g.drop_above(50);
        assert!(g.placements().next().is_none());
        g.check().unwrap();
    }

    #[test]
    fn anonymous_images_go_with_their_last_placement() {
        let mut g = graphics();
        let k = g.new_key(ImageKey::Anonymous);
        g.insert(Image::new(k, 10, 10));
        let a = g.place(placement(k, 0, 0, 1));
        let b = g.place(placement(k, 0, 1, 1));
        g.unplace(a);
        assert!(g.image(k).is_some());
        g.unplace(b);
        assert!(g.image(k).is_none());
        g.check().unwrap();
    }

    #[test]
    fn numbers_name_the_newest_image() {
        let mut g = graphics();
        let mut a = Image::new(ImageKey::Kitty(3), 1, 1);
        a.number = 9;
        g.insert(a);
        let mut b = Image::new(ImageKey::Kitty(4), 1, 1);
        b.number = 9;
        g.insert(b);
        assert_eq!(g.by_number(9), Some(4));
        g.remove(ImageKey::Kitty(3));
        assert_eq!(g.by_number(9), Some(4));
        g.remove(ImageKey::Kitty(4));
        assert_eq!(g.by_number(9), None);
    }

    #[test]
    fn dropping_graphics_gives_its_bytes_back() {
        let pool = ImagePool::new(32 << 20);
        let mut g = Graphics::new(5, pool.clone());
        let mut image = Image::new(ImageKey::Own(1), 100, 100);
        image.pool = Some(pool.reserve(5, 40_000, None).unwrap());
        g.insert(image);
        assert_eq!(pool.used(), 40_000);
        drop(g);
        assert_eq!(pool.used(), 0);
    }

    #[test]
    fn evictions_mark_images_lost() {
        let pool = ImagePool::new(PAGE * 4);
        let mut g = Graphics::new(5, pool.clone());
        let mut image = Image::new(ImageKey::Kitty(1), 64, 64);
        let id = pool.reserve(5, PAGE * 3, None).unwrap();
        pool.set_pixels(id, Arc::new(VecBuffer::new(64, 64 * 3)));
        pool.set_busy(id, false);
        image.pool = Some(id);
        image.state = ImageState::Ready;
        g.insert(image);
        pool.reserve(6, PAGE * 3, None).unwrap();
        g.apply_evictions();
        let image = g.image(ImageKey::Kitty(1)).unwrap();
        assert_eq!(image.state, ImageState::Lost);
        assert_eq!(image.pool, None);
    }

    #[test]
    fn vec_buffers_count_whole_pages() {
        assert_eq!(VecBuffer::new(1, 1).allocated_size(), PAGE);
        assert_eq!(VecBuffer::new(64, 64).allocated_size(), PAGE);
        assert_eq!(VecBuffer::new(64, 65).allocated_size(), 2 * PAGE);
    }
}
