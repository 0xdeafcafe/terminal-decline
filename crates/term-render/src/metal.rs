//! Metal: the GPU and glyph textures every window shares, each window's
//! instance buffers, and the draws of a frame: each pane's backgrounds and
//! glyphs inside its own rectangle, then the dividers (splits design §3).
//!
//! What one process (the main thread) shares: the device, queue and
//! pipelines ([`Gpu`]), one `Fonts` per font setting ([`FontSet`]) and one
//! pair of glyph textures ([`GlyphCache`]). Each [`Renderer`] (a window)
//! has its own instance buffers and in-flight count; each [`PaneRender`] (a
//! terminal) keeps its instances and row cache.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::ops::Range;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlitCommandEncoder, MTLBuffer, MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLPurgeableState,
    MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResource,
    MTLResourceOptions, MTLScissorRect, MTLSize, MTLStorageMode, MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use term_core::Terminal;
use term_font::{FontConfig, Fonts, GlyphKey, Style};

use crate::frame::{Frame, FrameInput, GlyphInstance, GlyphSlot, GlyphSource, RectInstance, Rgba};
use crate::images::ImageInstance;
use crate::packer::{NoRoom, ShelfPacker};
use crate::pixels::{MetalPixels, PixelAlloc};
use crate::transition::{Blend, Uniforms};
use term_core::image::{Layer, PoolId};

type Device = ProtocolObject<dyn MTLDevice>;
type Buffer = ProtocolObject<dyn MTLBuffer>;
type Pipeline = ProtocolObject<dyn MTLRenderPipelineState>;
pub type CommandBuffer = ProtocolObject<dyn MTLCommandBuffer>;
/// A Metal texture, as taken by [`Renderer::encode`].
pub type Texture = ProtocolObject<dyn MTLTexture>;

const FRAMES_IN_FLIGHT: usize = 2;
/// Drawables a `CAMetalLayer` holds at most (its default
/// `maximumDrawableCount`), so the surfaces a window may have shown.
const SURFACES: usize = 3;
/// Starting sizes of the glyph textures, pixels.
const MASK_ATLAS: u32 = 1024;
const COLOR_ATLAS: u32 = 256;
/// Caps keep the glyph textures within 16 MB each (R8 at 4096², BGRA at 2048²).
const MAX_MASK_ATLAS: u32 = 4096;
const MAX_COLOR_ATLAS: u32 = 2048;

/// Command buffers on the GPU, counted so the CPU can wait for them.
#[derive(Default)]
struct InFlight {
    n: Mutex<usize>,
    cv: Condvar,
}

impl InFlight {
    /// Wait until fewer than `max` are in flight, then count one more.
    fn begin_below(&self, max: usize) {
        let mut n = self.n.lock().unwrap();
        while *n >= max {
            n = self.cv.wait(n).unwrap();
        }
        *n += 1;
    }

    fn begin(&self) {
        *self.n.lock().unwrap() += 1;
    }

    fn end(&self) {
        *self.n.lock().unwrap() -= 1;
        self.cv.notify_all();
    }

    fn wait_idle(&self) {
        let mut n = self.n.lock().unwrap();
        while *n > 0 {
            n = self.cv.wait(n).unwrap();
        }
    }
}

/// The Metal device, its queue and the two pipelines: one per process, the
/// shader library loaded once.
pub struct Gpu {
    device: Retained<Device>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    rect_pipeline: Retained<Pipeline>,
    glyph_pipeline: Retained<Pipeline>,
    image_pipeline: Retained<Pipeline>,
}

impl Gpu {
    fn new() -> Gpu {
        let device = MTLCreateSystemDefaultDevice().expect("no Metal device");
        let queue = device.newCommandQueue().expect("command queue");
        let library = load_shaders(&device);
        let pipeline = |vertex: &str, fragment: &str| {
            let desc = MTLRenderPipelineDescriptor::new();
            desc.setVertexFunction(library.newFunctionWithName(&NSString::from_str(vertex)).as_deref());
            desc.setFragmentFunction(library.newFunctionWithName(&NSString::from_str(fragment)).as_deref());
            let att = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
            att.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
            att.setBlendingEnabled(true);
            // Premultiplied alpha.
            att.setSourceRGBBlendFactor(MTLBlendFactor::One);
            att.setSourceAlphaBlendFactor(MTLBlendFactor::One);
            att.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
            att.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
            device.newRenderPipelineStateWithDescriptor_error(&desc).expect("pipeline")
        };
        let rect_pipeline = pipeline("rect_vertex", "rect_fragment");
        let glyph_pipeline = pipeline("glyph_vertex", "glyph_fragment");
        let image_pipeline = pipeline("image_vertex", "image_fragment");
        Gpu {
            device,
            queue,
            rect_pipeline,
            glyph_pipeline,
            image_pipeline,
        }
    }
}

/// Names one `Fonts` in the process's [`FontSet`]. Never reused, so glyphs
/// cached for fonts no window uses any more can't be mistaken for another's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontsId(u32);

struct FontSlot {
    id: FontsId,
    fonts: Rc<Fonts>,
    /// The setting it was loaded from, when it was loaded by the set:
    /// windows asking for the same setting share it.
    config: Option<(FontConfig, f64)>,
    users: usize,
}

/// One `Fonts` per font setting (`FontConfig` and backing scale), shared by
/// the windows that use it, with its shaped-cluster cache.
#[derive(Default)]
pub struct FontSet {
    next: u32,
    slots: Vec<FontSlot>,
}

impl FontSet {
    /// Add fonts made elsewhere; never shared with another caller. Faces
    /// with ligatures shape runs from here on (`Fonts::allow_runs`), as
    /// frames draw them in pieces (ligatures design §4.1).
    fn add(&mut self, mut fonts: Fonts, config: Option<(FontConfig, f64)>) -> (FontsId, Rc<Fonts>) {
        let id = FontsId(self.next);
        self.next += 1;
        fonts.allow_runs(true);
        let fonts = Rc::new(fonts);
        self.slots.push(FontSlot {
            id,
            fonts: fonts.clone(),
            config,
            users: 1,
        });
        (id, fonts)
    }

    /// The fonts for `config` at `scale`, loaded unless a window has them.
    /// The flag is whether they were just loaded.
    fn load(&mut self, config: &FontConfig, scale: f64) -> (FontsId, Rc<Fonts>, bool) {
        if let Some(s) = self
            .slots
            .iter_mut()
            .find(|s| s.config.as_ref().is_some_and(|(c, sc)| c == config && *sc == scale))
        {
            s.users += 1;
            return (s.id, s.fonts.clone(), false);
        }
        let (id, fonts) = self.add(Fonts::with_config(config, scale), Some((config.clone(), scale)));
        (id, fonts, true)
    }

    /// One user fewer; true when nobody uses them and they were dropped.
    fn release(&mut self, id: FontsId) -> bool {
        let Some(i) = self.slots.iter().position(|s| s.id == id) else {
            return false;
        };
        self.slots[i].users -= 1;
        if self.slots[i].users > 0 {
            return false;
        }
        self.slots.swap_remove(i);
        true
    }
}

/// One glyph texture; [`ShelfPacker`] decides where glyphs go.
struct Atlas {
    texture: Retained<Texture>,
    packer: ShelfPacker,
    /// The size it starts at, pixels.
    start: u32,
    format: MTLPixelFormat,
    bpp: usize,
}

fn new_texture(device: &Device, format: MTLPixelFormat, size: u32) -> Retained<Texture> {
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(format, size as usize, size as usize, false)
    };
    desc.setUsage(MTLTextureUsage::ShaderRead);
    device.newTextureWithDescriptor(&desc).expect("glyph texture")
}

impl Atlas {
    fn new(device: &Device, format: MTLPixelFormat, bpp: usize, size: u32, max: u32) -> Atlas {
        let packer = ShelfPacker::new(size, max);
        Atlas {
            texture: new_texture(device, format, packer.size()),
            start: packer.size(),
            packer,
            format,
            bpp,
        }
    }

    /// Empty the texture and give it back its starting size.
    fn restart(&mut self, device: &Device) {
        if self.packer.size() == self.start {
            self.packer.clear();
        } else {
            self.packer = ShelfPacker::new(self.start, self.packer.max());
            self.texture = new_texture(device, self.format, self.start);
        }
    }

    /// Double the texture, copying the old one into its top-left corner on
    /// the GPU, so every slot handed out stays valid. Frames already on the
    /// GPU keep sampling the old texture. False at the cap.
    fn grow(&mut self, gpu: &Gpu) -> bool {
        let old = self.packer.size() as usize;
        if !self.packer.grow() {
            return false;
        }
        let texture = new_texture(&gpu.device, self.format, self.packer.size());
        let cmd = gpu.queue.commandBuffer().expect("command buffer");
        let blit = cmd.blitCommandEncoder().expect("blit encoder");
        let zero = MTLOrigin { x: 0, y: 0, z: 0 };
        unsafe {
            blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                &self.texture,
                0,
                0,
                zero,
                MTLSize {
                    width: old,
                    height: old,
                    depth: 1,
                },
                &texture,
                0,
                0,
                zero,
            );
            // A managed texture (the default on macOS) keeps a CPU copy and a
            // GPU copy. The copy above wrote only the GPU one; glyphs are
            // then written into the CPU one with replaceRegion, and Metal
            // leaves it undefined what the GPU sees of a CPU copy that's
            // stale: on a GPU with its own memory the old glyphs can come
            // back blank. Bring the CPU copy up to date. On Apple silicon
            // there's one copy and this costs nothing.
            if texture.storageMode() == MTLStorageMode::Managed {
                blit.synchronizeResource(ProtocolObject::from_ref(&*texture));
            }
        }
        blit.endEncoding();
        cmd.commit();
        // Glyphs are uploaded from the CPU next: let the copy land first.
        // Growing happens a few times in a process's life.
        cmd.waitUntilCompleted();
        self.texture = texture;
        true
    }

    fn insert(&mut self, w: u32, h: u32, data: &[u8], in_flight: &InFlight) -> Result<(u16, u16), NoRoom> {
        let (x, y) = self.packer.insert(w, h)?;
        // replaceRegion doesn't synchronise with the GPU: wait until no frame
        // that samples this texture is in flight, in any window. New glyphs
        // are rare.
        in_flight.wait_idle();
        let region = MTLRegion {
            origin: MTLOrigin {
                x: x as usize,
                y: y as usize,
                z: 0,
            },
            size: MTLSize {
                width: w as usize,
                height: h as usize,
                depth: 1,
            },
        };
        unsafe {
            self.texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                region,
                0,
                NonNull::new(data.as_ptr() as *mut c_void).unwrap(),
                w as usize * self.bpp,
            );
        }
        Ok((x as u16, y as u16))
    }
}

/// One font set's glyphs by character, in front of the cache by key, so
/// a cell's lookup is an array index (ASCII) or one hash lookup.
struct Chars {
    /// `None`: not looked up yet; `Some(None)`: draws nothing.
    ascii: Box<[Option<Option<GlyphSlot>>; 128 * 4]>,
    other: HashMap<(u32, Style), Option<GlyphSlot>>,
}

impl Chars {
    fn new() -> Chars {
        Chars {
            ascii: Box::new([None; 512]),
            other: HashMap::new(),
        }
    }
}

/// The process's glyph textures: one coverage (R8) and one colour (BGRA)
/// texture, holding every window's glyphs, by (`FontsId`, `GlyphKey`).
pub struct GlyphCache {
    mask: Atlas,
    color: Atlas,
    by_key: HashMap<(FontsId, GlyphKey), Option<GlyphSlot>>,
    by_char: Vec<(FontsId, Chars)>,
    /// Command buffers on the GPU that sample these textures, from every
    /// window. Uploads, emptying and purging wait for it to drain; a
    /// window's own frames wait on its own count (so windows don't wait on
    /// each other's frames).
    in_flight: Arc<InFlight>,
    /// Bumped whenever glyphs are forgotten, so slots handed out before
    /// may be wrong (see [`GlyphSource::generation`]). Growing a texture
    /// keeps every slot, so it doesn't bump this.
    generation: u64,
    /// Windows using the textures, and how many of them are hidden: the
    /// textures go purgeable only when every one is.
    windows: usize,
    hidden: usize,
    volatile: bool,
    /// The time the frame being built may spend on new glyphs.
    budget: Budget,
}

/// The time one pane's build may spend finding and rasterising glyphs it
/// hasn't drawn before (roadmap R.1). A screenful of new CJK or emoji
/// takes 10–200 ms on the main thread (two thirds of it finding the
/// fallback font, a third rasterising), which held up typing; past the
/// budget, new glyphs wait for the next frames, which the pane builds
/// until it has them all. Printable ASCII never waits.
#[derive(Default)]
struct Budget {
    /// `None`: no limit (tests, and anything that isn't a window's frame).
    limit: Option<Duration>,
    /// Set by the first new glyph.
    deadline: Option<Instant>,
    /// A glyph was left for a later frame.
    deferred: bool,
}

impl Budget {
    fn begin(&mut self, limit: Option<Duration>) {
        *self = Budget {
            limit,
            ..Budget::default()
        };
    }

    /// Whether a new glyph must wait for a later frame. The first new
    /// glyph of a build never does, so every build makes progress.
    fn spent(&mut self) -> bool {
        let Some(limit) = self.limit else { return false };
        let now = Instant::now();
        let Some(deadline) = self.deadline else {
            self.deadline = Some(now + limit);
            return false;
        };
        if now >= deadline {
            self.deferred = true;
            return true;
        }
        false
    }

    /// Whether a glyph was left for later; the limit goes.
    fn end(&mut self) -> bool {
        std::mem::take(self).deferred
    }
}

thread_local! {
    /// Glyph texture sizes for the next process-wide cache on this thread
    /// (tests): (start, cap) of the coverage, then the colour texture.
    static TEXTURE_SIZES: Cell<Option<[(u32, u32); 2]>> = const { Cell::new(None) };
}

/// Glyph texture sizes for the shared state this thread creates next, when
/// no renderer is alive on it: `(start, cap)` for the coverage and the
/// colour texture. Test aid, so growing and emptying can be reached with a
/// few glyphs.
#[doc(hidden)]
pub fn set_glyph_texture_sizes(mask: (u32, u32), color: (u32, u32)) {
    TEXTURE_SIZES.set(Some([mask, color]));
}

impl GlyphCache {
    fn new(device: &Device) -> GlyphCache {
        let [(mask, max_mask), (color, max_color)] = TEXTURE_SIZES
            .take()
            .unwrap_or([(MASK_ATLAS, MAX_MASK_ATLAS), (COLOR_ATLAS, MAX_COLOR_ATLAS)]);
        GlyphCache {
            mask: Atlas::new(device, MTLPixelFormat::R8Unorm, 1, mask, max_mask),
            color: Atlas::new(device, MTLPixelFormat::BGRA8Unorm, 4, color, max_color),
            by_key: HashMap::new(),
            by_char: Vec::new(),
            in_flight: Arc::new(InFlight::default()),
            generation: 0,
            windows: 0,
            hidden: 0,
            volatile: false,
            budget: Budget::default(),
        }
    }

    /// The glyph source for one font set's frames.
    fn source<'a>(&'a mut self, id: FontsId, fonts: &'a Fonts) -> PaneGlyphs<'a> {
        let GlyphCache {
            mask,
            color,
            by_key,
            by_char,
            in_flight,
            generation,
            budget,
            ..
        } = self;
        // A handful of font sets at most: a search beats hashing.
        let i = match by_char.iter().position(|(f, _)| *f == id) {
            Some(i) => i,
            None => {
                by_char.push((id, Chars::new()));
                by_char.len() - 1
            }
        };
        PaneGlyphs {
            chars: &mut by_char[i].1,
            by_key,
            mask,
            color,
            in_flight,
            fonts,
            id,
            generation: *generation,
            budget,
        }
    }

    /// What a frame built now for `id` is built against.
    fn built_with(&self, id: FontsId) -> (u64, FontsId) {
        (self.generation, id)
    }

    /// A glyph texture ran out of room since the last [`make_room`](Self::make_room).
    fn needs_room(&self) -> bool {
        self.mask.packer.is_full() || self.color.packer.is_full()
    }

    /// Grow each texture that ran out of room, keeping its glyphs; at the
    /// cap, empty it instead (it refills with what's on screen) and forget
    /// the glyphs that were in it. The other texture keeps its glyphs.
    /// Returns whether every glyph was kept: false when a texture was
    /// emptied (with both full, even if the other grew), so instances
    /// built before are wrong.
    fn make_room(&mut self, gpu: &Gpu) -> bool {
        let mut kept = true;
        for color in [false, true] {
            let atlas = if color { &mut self.color } else { &mut self.mask };
            if !atlas.packer.is_full() {
                continue;
            }
            if !atlas.grow(gpu) {
                atlas.packer.clear();
                self.forget(|slot| slot.is_some_and(|s| s.color == color));
                kept = false;
            }
        }
        kept
    }

    /// Forget every glyph and empty both textures (keeping their sizes).
    fn clear(&mut self) {
        self.mask.packer.clear();
        self.color.packer.clear();
        self.forget(|_| true);
    }

    /// Fonts no window uses any more were dropped. Their glyphs can't be
    /// taken out of the textures one by one, so both are emptied and go
    /// back to their starting sizes, as a closed window's own textures
    /// were freed before they were shared: closing windows and changing
    /// font size never leave them bigger than the open windows need. The
    /// other windows' panes are built again (they are no longer current).
    fn fonts_dropped(&mut self, id: FontsId, device: &Device) {
        self.by_char.retain(|(f, _)| *f != id);
        if self.windows == 0 {
            return; // the last window: everything goes with it
        }
        self.mask.restart(device);
        self.color.restart(device);
        self.forget(|_| true);
    }

    fn forget(&mut self, drop: impl Fn(Option<GlyphSlot>) -> bool) {
        self.generation += 1;
        for (_, chars) in &mut self.by_char {
            for entry in chars.ascii.iter_mut() {
                if entry.is_some_and(&drop) {
                    *entry = None;
                }
            }
            chars.other.retain(|_, slot| !drop(*slot));
        }
        self.by_key.retain(|_, slot| !drop(*slot));
    }

    /// Under memory pressure: empty both textures into new ones at their
    /// starting sizes, so the old ones (and the pages written in them) go
    /// once the GPU lets them go, and forget every glyph; they are
    /// rasterised again as frames need them. Returns the bytes the old
    /// textures held, 0 if they were already empty at their starting
    /// sizes (another window purged them this turn), or purgeable
    /// already (every window hidden: macOS may take them as it is).
    fn purge(&mut self, device: &Device) -> usize {
        if self.volatile {
            return 0;
        }
        let mut freed = 0;
        for atlas in [&mut self.mask, &mut self.color] {
            if atlas.packer.size() == atlas.start && atlas.packer.is_empty() {
                continue;
            }
            let size = atlas.packer.size() as usize;
            freed += size * size * atlas.bpp;
            atlas.packer = ShelfPacker::new(atlas.start, atlas.packer.max());
            atlas.texture = new_texture(device, atlas.format, atlas.start);
        }
        if freed > 0 {
            self.forget(|_| true);
        }
        freed
    }

    /// Let macOS reclaim both textures (every window is hidden).
    fn make_purgeable(&mut self) {
        if self.volatile {
            return;
        }
        self.in_flight.wait_idle();
        self.mask.texture.setPurgeableState(MTLPurgeableState::Volatile);
        self.color.texture.setPurgeableState(MTLPurgeableState::Volatile);
        self.volatile = true;
    }

    /// Undo [`make_purgeable`](Self::make_purgeable); if macOS took the
    /// textures, glyphs are rasterised again as they are needed.
    fn make_resident(&mut self) {
        if !self.volatile {
            return;
        }
        self.volatile = false;
        let mask = self.mask.texture.setPurgeableState(MTLPurgeableState::NonVolatile);
        let color = self.color.texture.setPurgeableState(MTLPurgeableState::NonVolatile);
        if mask == MTLPurgeableState::Empty || color == MTLPurgeableState::Empty {
            self.clear();
        }
    }

    /// Side lengths in pixels of the coverage (text) and colour (emoji)
    /// glyph textures.
    pub fn texture_sizes(&self) -> (u32, u32) {
        (self.mask.packer.size(), self.color.packer.size())
    }
}

/// The shared glyph textures as one font set's frames see them.
pub(crate) struct PaneGlyphs<'a> {
    chars: &'a mut Chars,
    by_key: &'a mut HashMap<(FontsId, GlyphKey), Option<GlyphSlot>>,
    mask: &'a mut Atlas,
    color: &'a mut Atlas,
    in_flight: &'a InFlight,
    fonts: &'a Fonts,
    id: FontsId,
    generation: u64,
    budget: &'a mut Budget,
}

impl PaneGlyphs<'_> {
    /// `None` when there was no room in the texture right now (so the result
    /// must not be cached); `Some(None)` for glyphs that draw nothing or can
    /// never fit.
    fn lookup(&mut self, cp: u32, style: Style) -> Option<Option<GlyphSlot>> {
        match char::from_u32(cp).and_then(|ch| self.fonts.glyph_key(ch, style)) {
            Some(key) => self.by_key(key),
            None => Some(None),
        }
    }

    /// The slot for `key`, rasterised and uploaded on first use.
    fn by_key(&mut self, key: GlyphKey) -> Option<Option<GlyphSlot>> {
        if let Some(slot) = self.by_key.get(&(self.id, key)) {
            return Some(*slot);
        }
        let slot = self.rasterize(key)?;
        self.by_key.insert((self.id, key), slot);
        Some(slot)
    }

    fn rasterize(&mut self, key: GlyphKey) -> Option<Option<GlyphSlot>> {
        let Some(g) = self.fonts.rasterize_key(key) else {
            return Some(None);
        };
        let atlas = if g.color { &mut *self.color } else { &mut *self.mask };
        let (u, v) = match atlas.insert(g.width, g.height, &g.data, self.in_flight) {
            Ok(at) => at,
            Err(NoRoom::Full) => return None,
            Err(NoRoom::TooBig) => return Some(None),
        };
        Some(Some(GlyphSlot {
            u,
            v,
            w: g.width as u16,
            h: g.height as u16,
            left: g.left as i16,
            top: g.top as i16,
            color: g.color,
        }))
    }
}

impl GlyphSource for PaneGlyphs<'_> {
    #[inline]
    fn glyph(&mut self, cp: u32, style: Style) -> Option<GlyphSlot> {
        if cp < 128 {
            let i = cp as usize * 4 + style as usize;
            if let Some(slot) = self.chars.ascii[i] {
                return slot;
            }
            let slot = self.lookup(cp, style)?;
            self.chars.ascii[i] = Some(slot);
            return slot;
        }
        if let Some(slot) = self.chars.other.get(&(cp, style)) {
            return *slot;
        }
        if self.budget.spent() {
            return None;
        }
        let slot = self.lookup(cp, style)?;
        self.chars.other.insert((cp, style), slot);
        slot
    }

    /// Changes when glyphs are forgotten and with the fonts.
    fn generation(&self) -> u64 {
        self.generation << 32 ^ self.id.0 as u64
    }

    #[inline]
    fn glyph_key(&mut self, key: GlyphKey) -> Option<GlyphSlot> {
        if !self.by_key.contains_key(&(self.id, key)) && self.budget.spent() {
            return None;
        }
        self.by_key(key).flatten()
    }

    fn fonts(&self) -> Option<&Fonts> {
        Some(self.fonts)
    }

    fn fonts_key(&self) -> u64 {
        self.id.0 as u64 + 1
    }
}

/// What one process shares between its windows, on the main thread.
struct Shared {
    gpu: Gpu,
    fonts: RefCell<FontSet>,
    glyphs: RefCell<GlyphCache>,
}

thread_local! {
    /// Kept while any renderer holds it; the next renderer after the last
    /// one closes starts afresh.
    static SHARED: RefCell<Weak<Shared>> = const { RefCell::new(Weak::new()) };
}

fn shared() -> Rc<Shared> {
    SHARED.with_borrow_mut(|weak| {
        if let Some(s) = weak.upgrade() {
            return s;
        }
        let gpu = Gpu::new();
        let glyphs = GlyphCache::new(&gpu.device);
        let s = Rc::new(Shared {
            gpu,
            fonts: RefCell::new(FontSet::default()),
            glyphs: RefCell::new(glyphs),
        });
        *weak = Rc::downgrade(&s);
        s
    })
}

/// Everything the renderer keeps for one terminal between frames: its
/// instances, placed relative to the pane's top-left corner, the row
/// cache they are built with (cells design §9.3) and, in the frame, the
/// shaping cache of its rows in faces with ligatures (ligatures design
/// §4.2), checked against its fonts by `FontsId`.
#[derive(Default)]
pub struct PaneRender {
    pub frame: Frame,
    /// The glyph textures' generation and the fonts it was built against.
    built_with: Option<(u64, FontsId)>,
    /// Its last build left new glyphs for later (see [`Budget`]).
    deferred: bool,
    /// Its instances on the GPU (roadmap R.2).
    gpu: RefCell<PaneBuffers>,
}

/// A pane's instance buffers, in use by turns, each holding the frame's
/// instances as of a version ([`Frame::changes_since`]): a frame is
/// drawn by copying only what changed since the buffer's version.
#[derive(Default)]
struct PaneBuffers {
    rects: [InstanceBuffer; FRAMES_IN_FLIGHT],
    glyphs: [InstanceBuffer; FRAMES_IN_FLIGHT],
    /// The frame version each holds; 0 for none.
    held: [(u64, u64); FRAMES_IN_FLIGHT],
    next: usize,
    /// Command buffers drawing this pane on the GPU: a buffer is written
    /// only once the frames before that used it are done, whichever
    /// window drew them.
    in_flight: Arc<InFlight>,
}

impl PaneBuffers {
    /// Bring the next buffers up to date with `frame` (its glyphs only
    /// when `glyphs`), and hand them out.
    fn upload(&mut self, device: &Device, frame: &Frame, glyphs: bool) -> (Option<Retained<Buffer>>, Option<Retained<Buffer>>) {
        self.in_flight.begin_below(FRAMES_IN_FLIGHT);
        let k = self.next;
        self.next = (k + 1) % FRAMES_IN_FLIGHT;
        let version = frame.version();
        let (rects, grown) = self.rects[k].reserve_kept(device, frame.rects.len() * size_of::<RectInstance>());
        if grown {
            self.held[k].0 = 0;
        }
        if let Some(buf) = &rects {
            let range = match frame.changes_since(self.held[k].0) {
                Some((r, _)) => r,
                None => 0..frame.rects.len(),
            };
            // SAFETY: the buffer holds every rect.
            unsafe { write(buf, range.start, &frame.rects[range]) };
            self.held[k].0 = version;
        }
        if !glyphs {
            return (rects, None);
        }
        let (glyph_buf, grown) = self.glyphs[k].reserve_kept(device, frame.glyphs.len() * size_of::<GlyphInstance>());
        if grown {
            self.held[k].1 = 0;
        }
        if let Some(buf) = &glyph_buf {
            let range = match frame.changes_since(self.held[k].1) {
                Some((_, g)) => g,
                None => 0..frame.glyphs.len(),
            };
            // SAFETY: the buffer holds every glyph.
            unsafe { write(buf, range.start, &frame.glyphs[range]) };
            self.held[k].1 = version;
        }
        (rects, glyph_buf)
    }
}

impl PaneRender {
    pub fn new() -> PaneRender {
        PaneRender::default()
    }

    /// Forget everything kept, so the next build starts afresh (after a
    /// panic while building, splits design §7).
    pub fn forget(&mut self) {
        self.frame = Frame::default();
        self.built_with = None;
        self.deferred = false;
    }

    /// Its last build drew without some new glyphs, to keep within the
    /// time [`Renderer::set_glyph_budget`] gives: build it again next
    /// frame (every row, so they get their glyphs).
    pub fn glyphs_deferred(&self) -> bool {
        self.deferred
    }
}

/// A rectangle in whole device pixels of the drawable, y down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl PxRect {
    /// The part inside a `width` × `height` target (Metal rejects a
    /// scissor reaching outside it); `None` when nothing is left.
    pub fn clamped(self, width: u32, height: u32) -> Option<PxRect> {
        let x0 = self.x.clamp(0, width as i32);
        let y0 = self.y.clamp(0, height as i32);
        let x1 = self.x.saturating_add(self.w.max(0)).clamp(0, width as i32);
        let y1 = self.y.saturating_add(self.h.max(0)).clamp(0, height as i32);
        (x1 > x0 && y1 > y0).then_some(PxRect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }
}

/// One pane in a frame: its instances drawn at `origin` (pixels) and kept
/// inside `scissor`.
#[derive(Clone, Copy)]
pub struct PaneDraw<'a> {
    pub pane: &'a PaneRender,
    pub origin: (i32, i32),
    pub scissor: PxRect,
}

#[derive(Default)]
struct InstanceBuffer {
    buffer: Option<Retained<Buffer>>,
    capacity: usize,
}

impl InstanceBuffer {
    /// [`reserve`](Self::reserve), and whether the buffer is new (what
    /// was in the old one is gone).
    fn reserve_kept(&mut self, device: &Device, bytes: usize) -> (Option<Retained<Buffer>>, bool) {
        let grown = bytes > self.capacity;
        (self.reserve(device, bytes), grown)
    }

    /// A buffer of at least `bytes`, grown to the next power of two if
    /// needed; `None` for nothing.
    fn reserve(&mut self, device: &Device, bytes: usize) -> Option<Retained<Buffer>> {
        if bytes == 0 {
            return None;
        }
        if bytes > self.capacity {
            let cap = bytes.next_power_of_two().max(16 * 1024);
            let opts = MTLResourceOptions::StorageModeShared | MTLResourceOptions::CPUCacheModeWriteCombined;
            self.buffer = Some(device.newBufferWithLength_options(cap, opts).expect("instance buffer"));
            self.capacity = cap;
        }
        self.buffer.as_deref().map(|b| b.retain())
    }
}

/// Copy `data` into `buffer` from instance `at` on.
///
/// # Safety
/// `buffer` must hold at least `at + data.len()` instances of `T`.
unsafe fn write<T: Copy>(buffer: &Buffer, at: usize, data: &[T]) {
    unsafe {
        let dst = (buffer.contents().as_ptr() as *mut T).add(at);
        std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
    }
}

/// Where one pane's instances are in the frame's buffers.
struct Span {
    /// Index into the frame's panes.
    pane: usize,
    /// The pane's background over its scissor, when the pass is cleared to another colour.
    fill: Option<RectInstance>,
    scissor: MTLScissorRect,
    origin: [f32; 2],
    /// The fill's place in the window's rect buffer.
    fill_at: usize,
    /// The pane's own buffers, and how many of their instances to draw.
    rect_buf: Option<Retained<Buffer>>,
    glyph_buf: Option<Retained<Buffer>>,
    rects: usize,
    glyphs: usize,
    /// The pane's image instances in the frame's image buffer.
    images: Range<usize>,
}

/// A window's own drawing state: instance buffers, in flight by turns,
/// and the count of its frames on the GPU.
struct Window {
    rect_buffers: [InstanceBuffer; FRAMES_IN_FLIGHT],
    image_buffers: [InstanceBuffer; FRAMES_IN_FLIGHT],
    /// Textures over the images drawn last frame, by pool entry and
    /// generation (images.md §6.2): rebuilt each frame with only the
    /// images drawn in it, emptied when the window hides.
    textures: HashMap<(PoolId, u32), Retained<Texture>>,
    /// Last frame's map, while this frame's is made.
    old_textures: HashMap<(PoolId, u32), Retained<Texture>>,
    /// Per span, this frame's texture of each of its pane's images.
    span_textures: Vec<Option<Retained<Texture>>>,
    frame_index: usize,
    in_flight: Arc<InFlight>,
    spans: Vec<Span>,
    /// Command buffers encoded, and those the GPU has finished (counted in
    /// their completion handler).
    submitted: u64,
    completed: Arc<AtomicU64>,
}

impl Window {
    /// Encode `panes` and `dividers` into `target`: see
    /// [`Renderer::encode_panes`], partway through `blend` if given.
    #[allow(clippy::too_many_arguments)]
    fn encode(
        &mut self,
        shared: &Shared,
        fonts: FontsId,
        target: &Texture,
        clear: Rgba,
        panes: &[PaneDraw],
        dividers: &[RectInstance],
        blend: Option<&Blend>,
    ) -> Retained<CommandBuffer> {
        self.in_flight.begin_below(FRAMES_IN_FLIGHT);
        let gpu = &shared.gpu;
        let cache = shared.glyphs.borrow();
        let current = Some(cache.built_with(fonts));
        let (width, height) = (target.width() as u32, target.height() as u32);
        let viewport = [width as f32, height as f32];
        let blend = Uniforms::of(blend, viewport);
        // While a transition plays, the pass is cleared to nothing and the
        // clear colour drawn as a rect over it all, first, so it blends
        // per pixel like the rest (a wave sweeps the background too).
        // It is a pane's background, so it has that pane's tag.
        let clear_tag = match panes {
            [one] if one.pane.frame.clear == clear => one.pane.frame.clear_tag,
            _ => crate::frame::tag_of(crate::transition::slot::BACKGROUND),
        };
        let backdrop = (blend.playing() && clear >> 24 != 0).then(|| RectInstance {
            x: 0,
            y: 0,
            w: width.min(u16::MAX as u32) as u16,
            h: height.min(u16::MAX as u32) as u16,
            color: clear,
            tag: clear_tag,
        });
        let clear = if backdrop.is_some() { 0 } else { clear };

        // Each pane's rects and glyphs are in its own buffers, brought up
        // to date; the window's hold the fills (when a pane's background
        // isn't the pass's clear colour), then the dividers, and the
        // images back to back.
        self.spans.clear();
        let mut pane_flights = Vec::new();
        let (mut rects, mut images) = (backdrop.is_some() as usize, 0);
        for (pane, p) in panes.iter().enumerate() {
            let Some(s) = p.scissor.clamped(width, height) else {
                continue;
            };
            let frame = &p.pane.frame;
            let fill = (frame.clear != clear).then(|| RectInstance {
                x: s16(s.x.saturating_sub(p.origin.0)),
                y: s16(s.y.saturating_sub(p.origin.1)),
                w: s.w.min(u16::MAX as i32) as u16,
                h: s.h.min(u16::MAX as i32) as u16,
                color: frame.clear,
                tag: frame.clear_tag,
            });
            // Glyphs placed in textures since emptied would draw other
            // glyphs: such a pane draws its backgrounds until rebuilt.
            let with_glyphs = p.pane.built_with == current;
            let mut bufs = p.pane.gpu.borrow_mut();
            let (rect_buf, glyph_buf) = bufs.upload(&gpu.device, frame, with_glyphs);
            pane_flights.push(bufs.in_flight.clone());
            let n_images = frame.images.instances.len();
            self.spans.push(Span {
                pane,
                fill,
                scissor: MTLScissorRect {
                    x: s.x as usize,
                    y: s.y as usize,
                    width: s.w as usize,
                    height: s.h as usize,
                },
                origin: [p.origin.0 as f32, p.origin.1 as f32],
                fill_at: rects,
                rects: if rect_buf.is_some() { frame.rects.len() } else { 0 },
                glyphs: if glyph_buf.is_some() { frame.glyphs.len() } else { 0 },
                rect_buf,
                glyph_buf,
                images: images..images + n_images,
            });
            rects += fill.is_some() as usize;
            images += n_images;
        }
        let divider_rects = rects..rects + dividers.len();
        rects += dividers.len();

        let i = self.frame_index;
        self.frame_index = (i + 1) % FRAMES_IN_FLIGHT;
        let rect_buf = self.rect_buffers[i].reserve(&gpu.device, rects * size_of::<RectInstance>());
        let image_buf = self.image_buffers[i].reserve(&gpu.device, images * size_of::<ImageInstance>());
        // This frame's textures: kept from the last frame, or made over
        // the image's buffer. Pixels from elsewhere (tests' `Vec`s) can't
        // be drawn and are skipped.
        std::mem::swap(&mut self.textures, &mut self.old_textures);
        self.textures.clear();
        self.span_textures.clear();
        for span in &self.spans {
            for image in &panes[span.pane].pane.frame.images.images {
                let key = (image.pool, image.generation);
                let texture = match self.textures.get(&key) {
                    Some(t) => Some(t.clone()),
                    None => self.old_textures.remove(&key).or_else(|| {
                        let pixels = image.pixels.as_any().downcast_ref::<MetalPixels>()?;
                        pixels.texture()
                    }),
                };
                if let Some(t) = &texture {
                    self.textures.insert(key, t.clone());
                }
                self.span_textures.push(texture);
            }
        }
        self.old_textures.clear();
        // SAFETY: every span lies within the counts the buffers were
        // reserved for, and the dividers come after them.
        unsafe {
            for span in &self.spans {
                let frame = &panes[span.pane].pane.frame;
                if let Some(buf) = &rect_buf
                    && let Some(fill) = span.fill
                {
                    write(buf, span.fill_at, &[fill]);
                }
                if let Some(buf) = &image_buf
                    && !span.images.is_empty()
                {
                    write(buf, span.images.start, &frame.images.instances);
                }
            }
            if let Some(buf) = &rect_buf {
                write(buf, divider_rects.start, dividers);
                if let Some(b) = backdrop {
                    write(buf, 0, &[b]);
                }
            }
        }

        let pass = MTLRenderPassDescriptor::renderPassDescriptor();
        let att = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        att.setTexture(Some(target));
        att.setLoadAction(MTLLoadAction::Clear);
        att.setStoreAction(MTLStoreAction::Store);
        let a = (clear >> 24) as f64 / 255.0;
        let ch = |shift: u32| ((clear >> shift) & 0xFF) as f64 / 255.0 * a;
        att.setClearColor(MTLClearColor {
            red: ch(0),
            green: ch(8),
            blue: ch(16),
            alpha: a,
        });

        let cmd = gpu.queue.commandBuffer().expect("command buffer");
        let enc = cmd.renderCommandEncoderWithDescriptor(&pass).expect("encoder");
        let draw = |pipeline: &Pipeline, buf: &Buffer, uniforms: &[f32; 4], range: &Range<usize>| unsafe {
            enc.setRenderPipelineState(pipeline);
            enc.setVertexBuffer_offset_atIndex(Some(buf), 0, 0);
            enc.setVertexBytes_length_atIndex(NonNull::from(uniforms).cast::<c_void>(), 16, 1);
            enc.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
                MTLPrimitiveType::TriangleStrip,
                0,
                4,
                range.len(),
                range.start,
            );
        };
        // SAFETY: `Uniforms` is plain data of the shaders' `Blend` layout.
        unsafe {
            let bytes = NonNull::from(&blend).cast::<c_void>();
            enc.setFragmentBytes_length_atIndex(bytes, size_of::<Uniforms>(), 0);
        }
        if let Some(buf) = &rect_buf
            && backdrop.is_some()
        {
            enc.setScissorRect(MTLScissorRect {
                x: 0,
                y: 0,
                width: width as usize,
                height: height as usize,
            });
            draw(&gpu.rect_pipeline, buf, &[viewport[0], viewport[1], 0.0, 0.0], &(0..1));
        }
        let any_glyphs = self.spans.iter().any(|s| s.glyph_buf.is_some());
        if any_glyphs {
            unsafe {
                enc.setFragmentTexture_atIndex(Some(&cache.mask.texture), 0);
                enc.setFragmentTexture_atIndex(Some(&cache.color.texture), 1);
            }
        }
        let set_glyph_textures = || {
            if any_glyphs {
                unsafe {
                    enc.setFragmentTexture_atIndex(Some(&cache.mask.texture), 0);
                    enc.setFragmentTexture_atIndex(Some(&cache.color.texture), 1);
                }
            }
        };
        let mut textures_at = 0;
        for span in &self.spans {
            enc.setScissorRect(span.scissor);
            let uniforms = [viewport[0], viewport[1], span.origin[0], span.origin[1]];
            let frame = &panes[span.pane].pane.frame;
            let pane_textures = &self.span_textures[textures_at..textures_at + frame.images.images.len()];
            textures_at += frame.images.images.len();
            // The layers (images.md §6.3): images under the backgrounds,
            // the backgrounds, images under the text, highlights and
            // decorations, glyphs, images above the text, the cursor again.
            let draws = &frame.images.draws;
            let images_in = |layer: Layer| {
                let Some(buf) = &image_buf else { return false };
                let mut any = false;
                for d in draws.iter().filter(|d| d.layer == layer) {
                    let Some(texture) = &pane_textures[d.image as usize] else {
                        continue;
                    };
                    unsafe { enc.setFragmentTexture_atIndex(Some(texture), 0) };
                    let at = span.images.start + d.instances.start as usize;
                    draw(&gpu.image_pipeline, buf, &uniforms, &(at..at + d.instances.len()));
                    any = true;
                }
                any
            };
            images_in(Layer::BelowBackground);
            // The fill comes before the frame's rects.
            if let Some(buf) = &rect_buf
                && span.fill.is_some()
            {
                draw(&gpu.rect_pipeline, buf, &uniforms, &(span.fill_at..span.fill_at + 1));
            }
            if let Some(buf) = &span.rect_buf
                && span.rects > 0
            {
                let split = frame.split.backgrounds.min(span.rects);
                if draws.iter().any(|d| d.layer == Layer::BelowText) {
                    draw(&gpu.rect_pipeline, buf, &uniforms, &(0..split));
                    images_in(Layer::BelowText);
                    draw(&gpu.rect_pipeline, buf, &uniforms, &(split..span.rects));
                } else {
                    draw(&gpu.rect_pipeline, buf, &uniforms, &(0..span.rects));
                }
            } else {
                images_in(Layer::BelowText);
            }
            let above = draws.iter().any(|d| d.layer == Layer::AboveText);
            if draws.iter().any(|d| d.layer != Layer::AboveText) {
                set_glyph_textures();
            }
            if let Some(buf) = &span.glyph_buf
                && span.glyphs > 0
            {
                draw(&gpu.glyph_pipeline, buf, &uniforms, &(0..span.glyphs));
            }
            if above && images_in(Layer::AboveText) {
                set_glyph_textures();
                if let Some(buf) = &span.rect_buf
                    && !frame.split.cursor.is_empty()
                    && frame.split.cursor.end <= span.rects
                {
                    draw(&gpu.rect_pipeline, buf, &uniforms, &frame.split.cursor);
                }
            }
        }
        if let Some(buf) = &rect_buf
            && !divider_rects.is_empty()
        {
            enc.setScissorRect(MTLScissorRect {
                x: 0,
                y: 0,
                width: width as usize,
                height: height as usize,
            });
            draw(&gpu.rect_pipeline, buf, &[viewport[0], viewport[1], 0.0, 0.0], &divider_rects);
        }
        enc.endEncoding();

        self.submitted += 1;
        cache.in_flight.begin();
        let window = self.in_flight.clone();
        let textures = cache.in_flight.clone();
        let completed = self.completed.clone();
        let done = RcBlock::new(move |_: NonNull<CommandBuffer>| {
            completed.fetch_add(1, Ordering::AcqRel);
            for p in &pane_flights {
                p.end();
            }
            textures.end();
            window.end();
        });
        unsafe { cmd.addCompletedHandler(RcBlock::as_ptr(&done)) };
        cmd
    }
}

#[inline]
fn s16(v: i32) -> i16 {
    v.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// One window's renderer: its fonts, instance buffers and drawables, over
/// the GPU and glyph textures the process shares.
pub struct Renderer {
    shared: Rc<Shared>,
    fonts_id: FontsId,
    fonts: Rc<Fonts>,
    window: Window,
    /// The full-window pane [`prepare`](Self::prepare) builds and
    /// [`present`](Self::present) draws, until windows hold panes (B2.3).
    pub pane: PaneRender,
    /// The surfaces of the last frames presented, newest first, one for
    /// each drawable the layer may hold, so they can be marked purgeable
    /// while the window is hidden. The window server keeps the surface it
    /// showed last, counted in this process's footprint, for as long as
    /// the window lives, and that isn't always the last one presented: a
    /// window behind others (a tab, or a window in the background test
    /// mode) may show an older one, even its first frame. All but the
    /// newest are let go with the spare drawable
    /// ([`forget_spare`](Self::forget_spare)).
    surfaces: [Option<Retained<objc2_io_surface::IOSurfaceRef>>; SURFACES],
    hidden: bool,
    /// Hidden with its surface kept (minimised).
    keep_surface: bool,
    /// The time each pane's build may spend on new glyphs; `None`: no limit.
    glyph_budget: Option<Duration>,
    /// The theme transition the next frames draw, if one plays.
    blend: Option<Blend>,
}

impl Renderer {
    /// A renderer drawing with `fonts`, which only it uses.
    pub fn new(fonts: Fonts) -> Renderer {
        let shared = shared();
        let (id, fonts) = shared.fonts.borrow_mut().add(fonts, None);
        Renderer::with_fonts(shared, id, fonts, true)
    }

    /// A renderer drawing with the fonts for `config` at `scale` device
    /// pixels a point, shared with every window that uses the same.
    pub fn with_font_config(config: &FontConfig, scale: f64) -> Renderer {
        let shared = shared();
        let (id, fonts, new) = shared.fonts.borrow_mut().load(config, scale);
        Renderer::with_fonts(shared, id, fonts, new)
    }

    fn with_fonts(shared: Rc<Shared>, id: FontsId, fonts: Rc<Fonts>, new: bool) -> Renderer {
        {
            let mut cache = shared.glyphs.borrow_mut();
            cache.windows += 1;
            cache.make_resident();
            if new {
                preload(&mut cache, id, &fonts);
            }
        }
        Renderer {
            shared,
            fonts_id: id,
            fonts,
            window: Window {
                rect_buffers: std::array::from_fn(|_| InstanceBuffer { buffer: None, capacity: 0 }),
                image_buffers: std::array::from_fn(|_| InstanceBuffer { buffer: None, capacity: 0 }),
                textures: HashMap::new(),
                old_textures: HashMap::new(),
                span_textures: Vec::new(),
                frame_index: 0,
                in_flight: Arc::new(InFlight::default()),
                spans: Vec::new(),
                submitted: 0,
                completed: Arc::new(AtomicU64::new(0)),
            },
            pane: PaneRender::default(),
            surfaces: [None, None, None],
            hidden: false,
            keep_surface: false,
            glyph_budget: None,
            blend: None,
        }
    }

    /// The window can't be seen: let macOS reclaim its surface on screen
    /// unless `keep_surface` (minimised windows show it in the Dock), and
    /// the glyph textures once every window is hidden. Both come back in
    /// `show`.
    pub fn hide(&mut self, keep_surface: bool) {
        if self.hidden {
            return;
        }
        self.window.in_flight.wait_idle();
        self.hidden = true;
        self.keep_surface = keep_surface;
        self.window.textures.clear();
        {
            let mut cache = self.shared.glyphs.borrow_mut();
            cache.hidden += 1;
            if cache.hidden == cache.windows {
                cache.make_purgeable();
            }
        }
        if !keep_surface {
            for s in self.surfaces.iter().flatten() {
                // kIOSurfacePurgeableVolatile
                unsafe { s.set_purgeable(1, std::ptr::null_mut()) };
            }
        }
    }

    /// The layer let its spare drawable go: so does this renderer, keeping
    /// only the surface on screen.
    pub fn forget_spare(&mut self) {
        for s in &mut self.surfaces[1..] {
            *s = None;
        }
    }

    /// Note the surface of a frame about to be presented.
    fn presenting(&mut self, texture: &Texture) {
        let Some(s) = texture.iosurface() else { return };
        let at = self.surfaces.iter().position(|k| {
            k.as_ref()
                .is_some_and(|k| std::ptr::eq::<objc2_io_surface::IOSurfaceRef>(&**k, &*s))
        });
        if at == Some(0) {
            return;
        }
        // Surfaces of another size are the layer's no longer: it never
        // hands them out again. A hidden window drawn at 1×1 (only under
        // the test control) leaves those it showed before purgeable, as
        // `hide` did, which a `show` for that frame undid.
        let (w, h) = (s.width(), s.height());
        for k in &mut self.surfaces {
            if k.as_ref().is_some_and(|k| (k.width(), k.height()) != (w, h)) {
                if (w, h) == (1, 1) {
                    // kIOSurfacePurgeableVolatile
                    unsafe { k.as_ref().expect("checked").set_purgeable(1, std::ptr::null_mut()) };
                }
                *k = None;
            }
        }
        // Newest first: a surface the layer hands out again moves to the
        // front, a new one pushes the oldest out.
        self.surfaces[..=at.unwrap_or(SURFACES - 1)].rotate_right(1);
        self.surfaces[0] = Some(s);
    }

    /// Mark the hidden window's surfaces purgeable again. Marking one the
    /// window server is still taking up (a frame presented just before
    /// the window was hidden) can leave its pages counted; once it has
    /// settled, marking it again lets them go.
    pub fn hide_again(&mut self) {
        if !self.hidden || self.keep_surface {
            return;
        }
        for s in self.surfaces.iter().flatten() {
            // kIOSurfacePurgeableVolatile
            unsafe { s.set_purgeable(1, std::ptr::null_mut()) };
        }
    }

    /// Undo `hide`. Must run before the next frame is built or drawn.
    pub fn show(&mut self) {
        if !self.hidden {
            return;
        }
        self.hidden = false;
        {
            let mut cache = self.shared.glyphs.borrow_mut();
            cache.hidden -= 1;
            cache.make_resident();
        }
        for s in self.surfaces.iter().flatten() {
            // kIOSurfacePurgeableNonVolatile
            unsafe { s.set_purgeable(0, std::ptr::null_mut()) };
        }
    }

    /// Frames the GPU has finished drawing (counted when each command
    /// buffer completes, not when it is presented).
    pub fn frames_completed(&self) -> u64 {
        self.window.completed.load(Ordering::Acquire)
    }

    /// Frames encoded so far. Frames complete in order, so frame `n` is done
    /// once [`frames_completed`](Self::frames_completed) reaches `n`.
    pub fn frames_submitted(&self) -> u64 {
        self.window.submitted
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    pub fn device(&self) -> &Device {
        &self.shared.gpu.device
    }

    pub fn metrics(&self) -> term_font::Metrics {
        self.fonts.metrics
    }

    /// The fonts this window draws with.
    pub fn fonts_id(&self) -> FontsId {
        self.fonts_id
    }

    /// Side lengths in pixels of the process's coverage (text) and colour
    /// (emoji) glyph textures.
    pub fn glyph_texture_sizes(&self) -> (u32, u32) {
        self.shared.glyphs.borrow().texture_sizes()
    }

    /// Draw with `fonts` from now on. When no window uses the old fonts any
    /// more, the glyph textures are emptied (see `fonts_dropped`): every
    /// pane, in every window, is built again.
    pub fn set_fonts(&mut self, fonts: Fonts) {
        let (id, fonts) = self.shared.fonts.borrow_mut().add(fonts, None);
        self.switch_fonts(id, fonts, true);
    }

    /// Draw with the fonts for `config` at `scale`, shared with every
    /// window that uses the same.
    pub fn set_font_config(&mut self, config: &FontConfig, scale: f64) {
        let (id, fonts, new) = self.shared.fonts.borrow_mut().load(config, scale);
        self.switch_fonts(id, fonts, new);
    }

    fn switch_fonts(&mut self, id: FontsId, fonts: Rc<Fonts>, new: bool) {
        let old = std::mem::replace(&mut self.fonts_id, id);
        self.fonts = fonts;
        let mut cache = self.shared.glyphs.borrow_mut();
        if self.shared.fonts.borrow_mut().release(old) {
            cache.fonts_dropped(old, &self.shared.gpu.device);
        }
        if new {
            preload(&mut cache, id, &self.fonts);
        }
    }

    /// After a panic while building a pane (splits design §7): a glyph
    /// half put in the textures, or a shaping cached half way, can't be
    /// trusted. Both textures are emptied (every pane of every window is
    /// built again before its glyphs are drawn) and this window's fonts
    /// forget the clusters they shaped.
    pub fn forget_glyphs(&mut self) {
        self.shared.glyphs.borrow_mut().clear();
        self.fonts.clear_shaped_clusters();
    }

    /// Under memory pressure (roadmap R.5): give back what the next
    /// frames can make again. The glyph textures, shared by every window,
    /// start again empty at their starting sizes (every pane of every
    /// window is built again before its glyphs are drawn), and this
    /// window's instance buffers and image textures go. Waits for the
    /// GPU to finish this window's frames first. Returns the bytes given
    /// back, roughly: what the textures and buffers held.
    pub fn purge(&mut self) -> usize {
        self.window.in_flight.wait_idle();
        let mut freed = 0;
        let w = &mut self.window;
        for b in w.rect_buffers.iter_mut().chain(w.image_buffers.iter_mut()) {
            freed += b.capacity;
            *b = InstanceBuffer { buffer: None, capacity: 0 };
        }
        freed += w
            .textures
            .values()
            .chain(w.old_textures.values())
            .map(|t| t.width() * t.height() * 4)
            .sum::<usize>();
        w.textures.clear();
        w.old_textures.clear();
        w.span_textures.clear();
        let mut cache = self.shared.glyphs.borrow_mut();
        // Other windows' frames may still sample the textures.
        cache.in_flight.wait_idle();
        freed += cache.purge(&self.shared.gpu.device);
        freed
    }

    /// Whether `pane` was built against the glyph textures as they are and
    /// this window's fonts. A pane that isn't must be built again before
    /// its glyphs are drawn: until then it draws its backgrounds only.
    /// Emptying a full texture (at its cap) or new fonts make every pane
    /// built before stale, also panes not built this frame.
    pub fn is_current(&self, pane: &PaneRender) -> bool {
        pane.built_with == Some(self.shared.glyphs.borrow().built_with(self.fonts_id))
    }

    /// Build `pane`'s instances from a snapshot of `term` (cells design
    /// §9), relative to the pane's top-left corner, all with the terminal
    /// locked: [`snapshot_pane`](Self::snapshot_pane) then
    /// [`build_from_snapshot`](Self::build_from_snapshot). It touches no
    /// GPU state except new glyphs (and, rarely, a glyph texture that
    /// grows).
    pub fn build_pane(&mut self, pane: &mut PaneRender, term: &mut Terminal, input: &FrameInput) {
        build(&self.shared, self.fonts_id, &self.fonts, self.glyph_budget, pane, term, input);
    }

    /// Let each pane's build spend at most `limit` on glyphs it hasn't
    /// drawn before, leaving the rest for later frames
    /// ([`PaneRender::glyphs_deferred`]); `None` (the default): no limit.
    pub fn set_glyph_budget(&mut self, limit: Option<Duration>) {
        self.glyph_budget = limit;
    }

    /// The part of building `pane` that needs `term` locked: the snapshot,
    /// copying the rows it has no instances for. Then, with the lock let
    /// go, [`build_from_snapshot`](Self::build_from_snapshot), which
    /// shapes and builds them (ligatures design §5: shaping is outside the
    /// lock).
    pub fn snapshot_pane(&mut self, pane: &mut PaneRender, term: &mut Terminal, input: &FrameInput) {
        let generation = self.shared.glyphs.borrow_mut().source(self.fonts_id, &self.fonts).generation();
        snapshot(pane, term, input, generation);
    }

    /// Build `pane` from its last [`snapshot_pane`](Self::snapshot_pane),
    /// with the same `input`, without the terminal. False when that can't
    /// be done (the glyph textures were emptied since, so its cached rows
    /// must be copied again): build it with
    /// [`build_pane`](Self::build_pane), with the terminal locked.
    pub fn build_from_snapshot(&mut self, pane: &mut PaneRender, input: &FrameInput) -> bool {
        finish(&self.shared, self.fonts_id, &self.fonts, self.glyph_budget, pane, input)
    }

    /// [`build_pane`](Self::build_pane), with `inject` changing the
    /// snapshot before it is built: how tests put images in a snapshot
    /// (its `placements` and `virtual_placements`) until term-core's
    /// protocols make them. Test aid.
    #[doc(hidden)]
    pub fn build_pane_with(
        &mut self,
        pane: &mut PaneRender,
        term: &mut Terminal,
        input: &FrameInput,
        inject: impl FnOnce(&mut term_core::Snapshot),
    ) {
        self.snapshot_pane(pane, term, input);
        inject(pane.frame.snapshot_mut());
        let built = self.build_from_snapshot(pane, input);
        assert!(built, "the glyph textures were emptied while building");
    }

    /// Makes image pixels this renderer can draw, from any thread.
    pub fn pixel_alloc(&self) -> PixelAlloc {
        PixelAlloc::new(self.shared.gpu.device.clone())
    }

    /// Images with textures this window keeps (those drawn in its last
    /// frame). Test aid.
    #[doc(hidden)]
    pub fn image_textures(&self) -> usize {
        self.window.textures.len()
    }

    /// Build the full-window pane's instances ([`build_pane`](Self::build_pane)
    /// on [`Renderer::pane`]).
    pub fn prepare(&mut self, term: &mut Terminal, input: &FrameInput) {
        build(
            &self.shared,
            self.fonts_id,
            &self.fonts,
            self.glyph_budget,
            &mut self.pane,
            term,
            input,
        );
    }

    /// The reference path (`Frame::build_live`) into the full-window pane.
    /// Test aid.
    #[doc(hidden)]
    pub fn build_live(&mut self, term: &Terminal, input: &FrameInput) {
        let mut cache = self.shared.glyphs.borrow_mut();
        self.pane
            .frame
            .build_live(term, input, &mut cache.source(self.fonts_id, &self.fonts));
        self.pane.built_with = Some(cache.built_with(self.fonts_id));
    }

    /// Encode and present the full-window pane.
    pub fn present(&mut self, layer: &CAMetalLayer) {
        let Some(drawable) = layer.nextDrawable() else {
            return;
        };
        let texture = drawable.texture();
        self.presenting(&texture);
        let cmd = self.encode(&texture);
        cmd.presentDrawable(ProtocolObject::from_ref(&*drawable));
        cmd.commit();
    }

    /// Encode and present `panes` and `dividers` ([`encode_panes`](Self::encode_panes)).
    pub fn present_panes(&mut self, layer: &CAMetalLayer, clear: Rgba, panes: &[PaneDraw], dividers: &[RectInstance]) {
        let Some(drawable) = layer.nextDrawable() else {
            return;
        };
        let texture = drawable.texture();
        self.presenting(&texture);
        let cmd = self.encode_panes(&texture, clear, panes, dividers);
        cmd.presentDrawable(ProtocolObject::from_ref(&*drawable));
        cmd.commit();
    }

    /// Encode the full-window pane into `target`, cleared to its
    /// background. See [`encode_panes`](Self::encode_panes).
    pub fn encode(&mut self, target: &Texture) -> Retained<CommandBuffer> {
        let all = PxRect {
            x: 0,
            y: 0,
            w: target.width() as i32,
            h: target.height() as i32,
        };
        let pane = PaneDraw {
            pane: &self.pane,
            origin: (0, 0),
            scissor: all,
        };
        self.window.encode(
            &self.shared,
            self.fonts_id,
            target,
            self.pane.frame.clear,
            &[pane],
            &[],
            self.blend.as_ref(),
        )
    }

    /// Encode one frame into `target` (a `BGRA8Unorm` render target),
    /// waiting first while two of this window's frames are already on the
    /// GPU: the pass is cleared to `clear`, then each pane draws its
    /// backgrounds and glyphs at its origin, inside its scissor (clamped to
    /// the target), then `dividers` (window pixels) on top. A pane whose
    /// background isn't `clear` first fills its scissor with it: with a
    /// see-through background, clear to transparent (0) so its alpha is
    /// applied once. The returned command buffer is not committed yet: the
    /// caller must commit it (it counts as in flight until it completes).
    pub fn encode_panes(
        &mut self,
        target: &Texture,
        clear: Rgba,
        panes: &[PaneDraw],
        dividers: &[RectInstance],
    ) -> Retained<CommandBuffer> {
        self.window
            .encode(&self.shared, self.fonts_id, target, clear, panes, dividers, self.blend.as_ref())
    }

    /// Draw the next frames partway through a theme transition (see
    /// [`crate::transition`]), or not (`None`). Only the uniforms change:
    /// the frames built with the new theme are drawn as they are.
    pub fn set_blend(&mut self, blend: Option<Blend>) {
        self.blend = blend;
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let mut cache = self.shared.glyphs.borrow_mut();
        cache.windows -= 1;
        if self.hidden {
            cache.hidden -= 1;
        }
        if self.shared.fonts.borrow_mut().release(self.fonts_id) {
            cache.fonts_dropped(self.fonts_id, &self.shared.gpu.device);
        }
    }
}

/// [`Renderer::build_pane`].
fn build(
    shared: &Shared,
    id: FontsId,
    fonts: &Fonts,
    budget: Option<Duration>,
    pane: &mut PaneRender,
    term: &mut Terminal,
    input: &FrameInput,
) {
    let generation = shared.glyphs.borrow_mut().source(id, fonts).generation();
    snapshot(pane, term, input, generation);
    if finish(shared, id, fonts, budget, pane, input) {
        return;
    }
    // A glyph texture was emptied at its cap (by this build or another
    // pane's since the snapshot): the cached rows are wrong, so copy every
    // row again and draw what fits.
    let mut cache = shared.glyphs.borrow_mut();
    pane.frame.forget_rows();
    let mut glyphs = cache.source(id, fonts);
    pane.frame.snapshot(term, input, glyphs.generation());
    pane.frame.build_snapshot(input, &mut glyphs);
    pane.built_with = Some(cache.built_with(id));
    deferred(&mut cache, pane);
}

/// Ends the build's glyph budget, noting whether the pane was drawn
/// without some new glyphs.
fn deferred(cache: &mut GlyphCache, pane: &mut PaneRender) {
    pane.deferred = cache.budget.end();
}

/// Takes `pane`'s snapshot. A pane last drawn without some new glyphs
/// forgets its rows first, so they are all copied and built again (they
/// were cached without those glyphs).
fn snapshot(pane: &mut PaneRender, term: &mut Terminal, input: &FrameInput, generation: u64) {
    if std::mem::take(&mut pane.deferred) {
        pane.frame.forget_rows();
    }
    pane.frame.snapshot(term, input, generation);
}

/// [`Renderer::build_from_snapshot`]: false when a glyph texture was
/// emptied since the snapshot or while building, and the pane needs every
/// row copied again.
fn finish(shared: &Shared, id: FontsId, fonts: &Fonts, budget: Option<Duration>, pane: &mut PaneRender, input: &FrameInput) -> bool {
    let mut cache = shared.glyphs.borrow_mut();
    cache.budget.begin(budget);
    if !pane.frame.build_snapshot(input, &mut cache.source(id, fonts)) {
        return false;
    }
    // Out of glyph texture space: grow the texture that filled up, which
    // keeps every glyph where it was, and build this frame's rows again
    // from the same snapshot. This ends: each pass grows a texture (a few
    // doublings to the cap), and at the cap it is emptied instead.
    while cache.needs_room() {
        if !cache.make_room(&shared.gpu) {
            return false;
        }
        pane.frame.redraw(input, &mut cache.source(id, fonts));
    }
    pane.built_with = Some(cache.built_with(id));
    deferred(&mut cache, pane);
    true
}

/// Rasterise printable ASCII for new fonts, so first frames don't.
fn preload(cache: &mut GlyphCache, id: FontsId, fonts: &Fonts) {
    let mut glyphs = cache.source(id, fonts);
    for c in 0x21..0x7F {
        glyphs.glyph(c, Style::Regular);
    }
}

#[cfg(precompiled_shaders)]
fn load_shaders(device: &Device) -> Retained<ProtocolObject<dyn MTLLibrary>> {
    static METALLIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shaders.metallib"));
    let data = dispatch2::DispatchData::from_static_bytes(METALLIB);
    device
        .newLibraryWithData_error(&data)
        .unwrap_or_else(|e| panic!("loading shaders failed: {e}"))
}

#[cfg(not(precompiled_shaders))]
fn load_shaders(device: &Device) -> Retained<ProtocolObject<dyn MTLLibrary>> {
    let source = NSString::from_str(include_str!("shaders.metal"));
    device
        .newLibraryWithSource_options_error(&source, None)
        .unwrap_or_else(|e| panic!("shader compile failed: {e}"))
}

/// Sets up a layer for this renderer.
pub fn configure_layer(layer: &CAMetalLayer, device: &Device, opaque: bool) {
    layer.setDevice(Some(device));
    layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    layer.setFramebufferOnly(true);
    layer.setMaximumDrawableCount(2);
    layer.setOpaque(opaque);
    // Draw only when told to, not on the layer's own schedule.
    layer.setPresentsWithTransaction(false);
    // Hand each frame to the window server as soon as the GPU is done,
    // instead of holding it for the display's next refresh: the window
    // server composites at the refresh anyway, so a window doesn't tear,
    // and frames reach the glass a refresh or two sooner (see DESIGN.md
    // "Input latency"). It also stops `nextDrawable` blocking the main
    // thread while both drawables wait for refreshes.
    layer.setDisplaySyncEnabled(false);
}
