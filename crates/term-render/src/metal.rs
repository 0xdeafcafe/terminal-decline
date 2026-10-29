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
use crate::packer::{NoRoom, ShelfPacker};

type Device = ProtocolObject<dyn MTLDevice>;
type Buffer = ProtocolObject<dyn MTLBuffer>;
type Pipeline = ProtocolObject<dyn MTLRenderPipelineState>;
pub type CommandBuffer = ProtocolObject<dyn MTLCommandBuffer>;
/// A Metal texture, as taken by [`Renderer::encode`].
pub type Texture = ProtocolObject<dyn MTLTexture>;

const FRAMES_IN_FLIGHT: usize = 2;
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
        Gpu {
            device,
            queue,
            rect_pipeline,
            glyph_pipeline,
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
    /// Add fonts made elsewhere; never shared with another caller.
    fn add(&mut self, fonts: Fonts, config: Option<(FontConfig, f64)>) -> (FontsId, Rc<Fonts>) {
        let id = FontsId(self.next);
        self.next += 1;
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
    /// Returns whether any texture grew.
    fn make_room(&mut self, gpu: &Gpu) -> bool {
        let mut grew = false;
        for color in [false, true] {
            let atlas = if color { &mut self.color } else { &mut self.mask };
            if !atlas.packer.is_full() {
                continue;
            }
            if atlas.grow(gpu) {
                grew = true;
            } else {
                atlas.packer.clear();
                self.forget(|slot| slot.is_some_and(|s| s.color == color));
            }
        }
        grew
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
        let slot = self.lookup(cp, style)?;
        self.chars.other.insert((cp, style), slot);
        slot
    }

    /// Changes when glyphs are forgotten and with the fonts.
    fn generation(&self) -> u64 {
        self.generation << 32 ^ self.id.0 as u64
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
/// instances, placed relative to the pane's top-left corner, and the row
/// cache they are built with (cells design §9.3). B.1's shaping cache
/// (ligatures design §4.2) goes here too.
#[derive(Default)]
pub struct PaneRender {
    pub frame: Frame,
    /// The glyph textures' generation and the fonts it was built against.
    built_with: Option<(u64, FontsId)>,
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

struct InstanceBuffer {
    buffer: Option<Retained<Buffer>>,
    capacity: usize,
}

impl InstanceBuffer {
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
    rects: Range<usize>,
    glyphs: Range<usize>,
}

/// A window's own drawing state: instance buffers, in flight by turns,
/// and the count of its frames on the GPU.
struct Window {
    rect_buffers: [InstanceBuffer; FRAMES_IN_FLIGHT],
    glyph_buffers: [InstanceBuffer; FRAMES_IN_FLIGHT],
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
    /// [`Renderer::encode_panes`].
    fn encode(
        &mut self,
        shared: &Shared,
        fonts: FontsId,
        target: &Texture,
        clear: Rgba,
        panes: &[PaneDraw],
        dividers: &[RectInstance],
    ) -> Retained<CommandBuffer> {
        self.in_flight.begin_below(FRAMES_IN_FLIGHT);
        let gpu = &shared.gpu;
        let cache = shared.glyphs.borrow();
        let current = Some(cache.built_with(fonts));
        let (width, height) = (target.width() as u32, target.height() as u32);
        let viewport = [width as f32, height as f32];

        // Lay the panes out back to back in the two buffers: each pane's
        // fill (when its background isn't the pass's clear colour) and
        // rects, then the dividers; each pane's glyphs.
        self.spans.clear();
        let (mut rects, mut glyphs) = (0, 0);
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
            });
            let n_rects = fill.is_some() as usize + frame.rects.len();
            // Glyphs placed in textures since emptied would draw other
            // glyphs: such a pane draws its backgrounds until rebuilt.
            let n_glyphs = if p.pane.built_with == current { frame.glyphs.len() } else { 0 };
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
                rects: rects..rects + n_rects,
                glyphs: glyphs..glyphs + n_glyphs,
            });
            rects += n_rects;
            glyphs += n_glyphs;
        }
        let divider_rects = rects..rects + dividers.len();
        rects += dividers.len();

        let i = self.frame_index;
        self.frame_index = (i + 1) % FRAMES_IN_FLIGHT;
        let rect_buf = self.rect_buffers[i].reserve(&gpu.device, rects * size_of::<RectInstance>());
        let glyph_buf = self.glyph_buffers[i].reserve(&gpu.device, glyphs * size_of::<GlyphInstance>());
        // SAFETY: every span lies within the counts the buffers were
        // reserved for, and the dividers come after them.
        unsafe {
            for span in &self.spans {
                let frame = &panes[span.pane].pane.frame;
                if let Some(buf) = &rect_buf {
                    let mut at = span.rects.start;
                    if let Some(fill) = span.fill {
                        write(buf, at, &[fill]);
                        at += 1;
                    }
                    write(buf, at, &frame.rects);
                }
                if let Some(buf) = &glyph_buf
                    && !span.glyphs.is_empty()
                {
                    write(buf, span.glyphs.start, &frame.glyphs);
                }
            }
            if let Some(buf) = &rect_buf {
                write(buf, divider_rects.start, dividers);
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
        if glyph_buf.is_some() {
            unsafe {
                enc.setFragmentTexture_atIndex(Some(&cache.mask.texture), 0);
                enc.setFragmentTexture_atIndex(Some(&cache.color.texture), 1);
            }
        }
        for span in &self.spans {
            enc.setScissorRect(span.scissor);
            let uniforms = [viewport[0], viewport[1], span.origin[0], span.origin[1]];
            if let Some(buf) = &rect_buf
                && !span.rects.is_empty()
            {
                draw(&gpu.rect_pipeline, buf, &uniforms, &span.rects);
            }
            if let Some(buf) = &glyph_buf
                && !span.glyphs.is_empty()
            {
                draw(&gpu.glyph_pipeline, buf, &uniforms, &span.glyphs);
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
    /// The surface last shown on screen, so it can be marked purgeable while
    /// the window is hidden.
    last_surface: Option<Retained<objc2_io_surface::IOSurfaceRef>>,
    hidden: bool,
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
                glyph_buffers: std::array::from_fn(|_| InstanceBuffer { buffer: None, capacity: 0 }),
                frame_index: 0,
                in_flight: Arc::new(InFlight::default()),
                spans: Vec::new(),
                submitted: 0,
                completed: Arc::new(AtomicU64::new(0)),
            },
            pane: PaneRender::default(),
            last_surface: None,
            hidden: false,
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
        {
            let mut cache = self.shared.glyphs.borrow_mut();
            cache.hidden += 1;
            if cache.hidden == cache.windows {
                cache.make_purgeable();
            }
        }
        if !keep_surface && let Some(s) = &self.last_surface {
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
        if let Some(s) = &self.last_surface {
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

    /// Whether `pane` was built against the glyph textures as they are and
    /// this window's fonts. A pane that isn't must be built again before
    /// its glyphs are drawn: until then it draws its backgrounds only.
    /// Emptying a full texture (at its cap) or new fonts make every pane
    /// built before stale, also panes not built this frame.
    pub fn is_current(&self, pane: &PaneRender) -> bool {
        pane.built_with == Some(self.shared.glyphs.borrow().built_with(self.fonts_id))
    }

    /// Build `pane`'s instances from a snapshot of `term` (cells design
    /// §9), relative to the pane's top-left corner. Call with the terminal
    /// locked; it touches no GPU state except new glyphs (and, rarely, a
    /// glyph texture that grows).
    pub fn build_pane(&mut self, pane: &mut PaneRender, term: &mut Terminal, input: &FrameInput) {
        build(&self.shared, self.fonts_id, &self.fonts, pane, term, input);
    }

    /// Build the full-window pane's instances ([`build_pane`](Self::build_pane)
    /// on [`Renderer::pane`]).
    pub fn prepare(&mut self, term: &mut Terminal, input: &FrameInput) {
        build(&self.shared, self.fonts_id, &self.fonts, &mut self.pane, term, input);
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
        self.last_surface = texture.iosurface();
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
        self.last_surface = texture.iosurface();
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
        self.window
            .encode(&self.shared, self.fonts_id, target, self.pane.frame.clear, &[pane], &[])
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
        self.window.encode(&self.shared, self.fonts_id, target, clear, panes, dividers)
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
fn build(shared: &Shared, id: FontsId, fonts: &Fonts, pane: &mut PaneRender, term: &mut Terminal, input: &FrameInput) {
    let mut cache = shared.glyphs.borrow_mut();
    pane.frame.build(term, input, &mut cache.source(id, fonts));
    // Out of glyph texture space: grow the texture that filled up and
    // build again. This ends: each pass either grows a texture (a few
    // doublings to the cap) or, at the cap, empties it once and draws
    // what fits.
    while cache.needs_room() {
        let grew = cache.make_room(&shared.gpu);
        // The rows built with the glyphs that didn't fit are cached: drop
        // them, so this snapshot copies every row again.
        pane.frame.forget_rows();
        pane.frame.build(term, input, &mut cache.source(id, fonts));
        if !grew {
            break;
        }
    }
    pane.built_with = Some(cache.built_with(id));
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
