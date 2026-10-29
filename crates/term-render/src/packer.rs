//! Where glyphs go in a glyph texture: filled shelf by shelf, left to right,
//! with a 1-pixel gap so neighbours never bleed into each other. No Metal
//! here, so it can be tested on its own.

/// Pixels left empty between glyphs, horizontally and vertically.
pub const GAP: u32 = 1;

/// Why a glyph wasn't placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoRoom {
    /// The texture has run out of space; [`ShelfPacker::is_full`] is now set,
    /// and [`ShelfPacker::grow`] (or, at the cap, [`ShelfPacker::clear`])
    /// makes room.
    Full,
    /// The glyph is bigger than the largest texture allowed, so it can never
    /// be placed. The texture is not marked full.
    TooBig,
}

/// One rectangle of free space, filled shelf by shelf.
#[derive(Clone, Debug)]
struct Region {
    x0: u32,
    y0: u32,
    w: u32,
    h: u32,
    /// The next free spot, relative to the region.
    x: u32,
    y: u32,
    shelf_h: u32,
}

impl Region {
    fn new(x0: u32, y0: u32, w: u32, h: u32) -> Region {
        Region {
            x0,
            y0,
            w,
            h,
            x: 0,
            y: 0,
            shelf_h: 0,
        }
    }

    /// Place a `w`×`h` rectangle, changing nothing if it doesn't fit.
    fn insert(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (mut x, mut y, mut shelf_h) = (self.x, self.y, self.shelf_h);
        if x + w > self.w {
            y += shelf_h + GAP;
            x = 0;
            shelf_h = 0;
        }
        if x + w > self.w || y + h > self.h {
            return None;
        }
        self.x = x + w + GAP;
        self.y = y;
        self.shelf_h = shelf_h.max(h);
        Some((self.x0 + x, self.y0 + y))
    }
}

/// Packs rectangles into a square texture of `size` pixels, which may grow
/// (by doubling) up to `max`. Growing keeps every placement: the new space,
/// right of and below the old square, is packed on its own, so the old
/// texture can be copied into the corner of the new one as it is.
#[derive(Clone, Debug)]
pub struct ShelfPacker {
    size: u32,
    max: u32,
    full: bool,
    /// Free space, filled in order: the first square, then each growth's
    /// right and bottom strips.
    regions: Vec<Region>,
}

impl ShelfPacker {
    pub fn new(size: u32, max: u32) -> ShelfPacker {
        let size = size.min(max);
        ShelfPacker {
            size,
            max,
            full: false,
            regions: vec![Region::new(0, 0, size, size)],
        }
    }

    /// Current texture size (width and height), pixels.
    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn max(&self) -> u32 {
        self.max
    }

    /// An insert failed for lack of space since the last grow or clear.
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// Place a `w`×`h` rectangle; returns its top-left corner.
    pub fn insert(&mut self, w: u32, h: u32) -> Result<(u32, u32), NoRoom> {
        if w > self.max || h > self.max {
            return Err(NoRoom::TooBig);
        }
        // Earlier regions keep the space their shelves left; glyphs are
        // added rarely, so trying each in turn costs nothing that matters.
        if let Some(at) = self.regions.iter_mut().find_map(|r| r.insert(w, h)) {
            return Ok(at);
        }
        // Out of room, or it fits only a bigger texture: ask for one.
        self.full = true;
        Err(NoRoom::Full)
    }

    /// Double the texture, up to `max`, keeping everything placed. Returns
    /// false, changing nothing, at `max`.
    pub fn grow(&mut self) -> bool {
        let old = self.size;
        let new = (old * 2).min(self.max);
        if new == old {
            return false;
        }
        self.size = new;
        self.full = false;
        self.regions.push(Region::new(old + GAP, 0, new - old - GAP, old));
        self.regions.push(Region::new(0, old + GAP, new, new - old - GAP));
        true
    }

    /// Empty the texture, keeping its size.
    pub fn clear(&mut self) {
        *self = ShelfPacker::new(self.size, self.max);
    }
}
