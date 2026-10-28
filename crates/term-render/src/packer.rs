//! Where glyphs go in a glyph texture: filled shelf by shelf, left to right,
//! with a 1-pixel gap so neighbours never bleed into each other. No Metal
//! here, so it can be tested on its own.

/// Pixels left empty between glyphs, horizontally and vertically.
pub const GAP: u32 = 1;

/// Why a glyph wasn't placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoRoom {
    /// The texture has run out of space; [`ShelfPacker::is_full`] is now set,
    /// and [`ShelfPacker::reset`] makes room (growing the texture if it can).
    Full,
    /// The glyph is bigger than the largest texture allowed, so it can never
    /// be placed. The texture is not marked full.
    TooBig,
}

/// Packs rectangles into a square texture of `size` pixels, which may grow
/// (by doubling) up to `max`.
#[derive(Clone, Debug)]
pub struct ShelfPacker {
    size: u32,
    max: u32,
    full: bool,
    x: u32,
    y: u32,
    shelf_h: u32,
}

impl ShelfPacker {
    pub fn new(size: u32, max: u32) -> ShelfPacker {
        ShelfPacker {
            size: size.min(max),
            max,
            full: false,
            x: 0,
            y: 0,
            shelf_h: 0,
        }
    }

    /// Current texture size (width and height), pixels.
    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn max(&self) -> u32 {
        self.max
    }

    /// An insert failed for lack of space since the last reset.
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// Place a `w`×`h` rectangle; returns its top-left corner.
    pub fn insert(&mut self, w: u32, h: u32) -> Result<(u32, u32), NoRoom> {
        if w > self.max || h > self.max {
            return Err(NoRoom::TooBig);
        }
        if w > self.size || h > self.size {
            // Fits a bigger texture: ask for one.
            self.full = true;
            return Err(NoRoom::Full);
        }
        if self.x + w > self.size {
            self.y += self.shelf_h + GAP;
            self.x = 0;
            self.shelf_h = 0;
        }
        if self.y + h > self.size {
            self.full = true;
            return Err(NoRoom::Full);
        }
        let at = (self.x, self.y);
        self.x += w + GAP;
        self.shelf_h = self.shelf_h.max(h);
        Ok(at)
    }

    /// Empty the texture. If it was full it doubles in size, up to `max`; at
    /// `max` it is simply emptied. Returns whether the size changed.
    pub fn reset(&mut self) -> bool {
        let old = self.size;
        if self.full {
            self.size = (self.size * 2).min(self.max);
        }
        *self = ShelfPacker::new(self.size, self.max);
        self.size != old
    }
}
