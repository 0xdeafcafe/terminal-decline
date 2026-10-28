//! Metal: glyph textures, instance buffers and the two draws per frame.

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Condvar, Mutex};

use block2::RcBlock;
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBuffer, MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLCreateSystemDefaultDevice,
    MTLDevice, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder,
    MTLRenderPassDescriptor, MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResourceOptions, MTLSize, MTLStoreAction, MTLTexture,
    MTLTextureDescriptor, MTLTextureUsage,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use term_font::{Fonts, Style};

use crate::frame::{Frame, FrameInput, GlyphSlot, GlyphSource};
use crate::packer::{NoRoom, ShelfPacker};

type Device = ProtocolObject<dyn MTLDevice>;
type Texture = ProtocolObject<dyn MTLTexture>;
type Buffer = ProtocolObject<dyn MTLBuffer>;

const FRAMES_IN_FLIGHT: usize = 2;
/// Caps keep the glyph textures within 16 MB each (R8 at 4096², BGRA at 2048²).
const MAX_MASK_ATLAS: u32 = 4096;
const MAX_COLOR_ATLAS: u32 = 2048;

/// One glyph texture; [`ShelfPacker`] decides where glyphs go.
struct Atlas {
    texture: Retained<Texture>,
    packer: ShelfPacker,
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
            packer,
            format,
            bpp,
        }
    }

    /// Empty the texture, growing it first if it ran out of room. Returns
    /// whether it grew.
    fn reset(&mut self, device: &Device) -> bool {
        let grew = self.packer.reset();
        if grew {
            self.texture = new_texture(device, self.format, self.packer.size());
        }
        grew
    }

    fn insert(&mut self, w: u32, h: u32, data: &[u8], in_flight: &(Mutex<usize>, Condvar)) -> Result<(u16, u16), NoRoom> {
        let (x, y) = self.packer.insert(w, h)?;
        // replaceRegion doesn't synchronise with the GPU: wait until no frame
        // that samples this texture is in flight. New glyphs are rare.
        let (lock, cvar) = in_flight;
        let mut n = lock.lock().unwrap();
        while *n > 0 {
            n = cvar.wait(n).unwrap();
        }
        drop(n);
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

/// Rasterises glyphs on first use and remembers where they went.
pub struct GlyphCache {
    fonts: Fonts,
    mask: Atlas,
    color: Atlas,
    /// ASCII lookups skip the hash map.
    ascii: Box<[Option<Option<GlyphSlot>>; 128 * 4]>,
    other: HashMap<(u32, Style), Option<GlyphSlot>>,
    in_flight: Arc<(Mutex<usize>, Condvar)>,
}

impl GlyphCache {
    fn new(device: &Device, fonts: Fonts, in_flight: Arc<(Mutex<usize>, Condvar)>) -> GlyphCache {
        let mut cache = GlyphCache {
            fonts,
            mask: Atlas::new(device, MTLPixelFormat::R8Unorm, 1, 1024, MAX_MASK_ATLAS),
            color: Atlas::new(device, MTLPixelFormat::BGRA8Unorm, 4, 256, MAX_COLOR_ATLAS),
            ascii: Box::new([None; 512]),
            other: HashMap::new(),
            in_flight,
        };
        for c in 0x21..0x7F {
            cache.glyph(c, Style::Regular);
        }
        cache
    }

    /// A glyph texture ran out of room since the last [`make_room`](Self::make_room).
    fn needs_room(&self) -> bool {
        self.mask.packer.is_full() || self.color.packer.is_full()
    }

    /// Empty each texture that ran out of room, growing it up to its cap (at
    /// the cap it is simply emptied and refilled with what's on screen), and
    /// forget the glyphs that were in it. The other texture keeps its glyphs.
    /// Returns whether any texture grew.
    fn make_room(&mut self, device: &Device) -> bool {
        let mut grew = false;
        for color in [false, true] {
            let atlas = if color { &mut self.color } else { &mut self.mask };
            if !atlas.packer.is_full() {
                continue;
            }
            grew |= atlas.reset(device);
            self.forget(|slot| slot.is_some_and(|s| s.color == color));
        }
        grew
    }

    /// Forget every glyph and empty both textures (keeping their sizes).
    fn clear(&mut self) {
        self.mask.packer = ShelfPacker::new(self.mask.packer.size(), self.mask.packer.max());
        self.color.packer = ShelfPacker::new(self.color.packer.size(), self.color.packer.max());
        self.forget(|_| true);
    }

    fn forget(&mut self, drop: impl Fn(Option<GlyphSlot>) -> bool) {
        for entry in self.ascii.iter_mut() {
            if entry.is_some_and(&drop) {
                *entry = None;
            }
        }
        self.other.retain(|_, slot| !drop(*slot));
    }

    /// `None` when there was no room in the texture right now (so the result
    /// must not be cached); `Some(None)` for glyphs that draw nothing or can
    /// never fit.
    fn rasterize(&mut self, cp: u32, style: Style) -> Option<Option<GlyphSlot>> {
        let Some(g) = char::from_u32(cp).and_then(|ch| self.fonts.rasterize(ch, style)) else {
            return Some(None);
        };
        let atlas = if g.color { &mut self.color } else { &mut self.mask };
        let (u, v) = match atlas.insert(g.width, g.height, &g.data, &self.in_flight) {
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

impl GlyphSource for GlyphCache {
    #[inline]
    fn glyph(&mut self, cp: u32, style: Style) -> Option<GlyphSlot> {
        if cp < 128 {
            let i = cp as usize * 4 + style as usize;
            if let Some(slot) = self.ascii[i] {
                return slot;
            }
            let slot = self.rasterize(cp, style)?;
            self.ascii[i] = Some(slot);
            return slot;
        }
        if let Some(slot) = self.other.get(&(cp, style)) {
            return *slot;
        }
        let slot = self.rasterize(cp, style)?;
        self.other.insert((cp, style), slot);
        slot
    }
}

struct InstanceBuffer {
    buffer: Option<Retained<Buffer>>,
    capacity: usize,
}

impl InstanceBuffer {
    /// Copy `data` in, growing the buffer to the next power of two if needed.
    fn upload<T: Copy>(&mut self, device: &Device, data: &[T]) -> Option<&Buffer> {
        let bytes = std::mem::size_of_val(data);
        if bytes == 0 {
            return None;
        }
        if bytes > self.capacity {
            let cap = bytes.next_power_of_two().max(16 * 1024);
            let opts = MTLResourceOptions::StorageModeShared | MTLResourceOptions::CPUCacheModeWriteCombined;
            self.buffer = Some(device.newBufferWithLength_options(cap, opts).expect("instance buffer"));
            self.capacity = cap;
        }
        let buffer = self.buffer.as_deref()?;
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, buffer.contents().as_ptr() as *mut u8, bytes) };
        Some(buffer)
    }
}

pub struct Renderer {
    device: Retained<Device>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    rect_pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    glyph_pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    pub glyphs: GlyphCache,
    rect_buffers: [InstanceBuffer; FRAMES_IN_FLIGHT],
    glyph_buffers: [InstanceBuffer; FRAMES_IN_FLIGHT],
    frame_index: usize,
    in_flight: Arc<(Mutex<usize>, Condvar)>,
    pub frame: Frame,
}

impl Renderer {
    pub fn new(fonts: Fonts) -> Renderer {
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
        let in_flight = Arc::new((Mutex::new(0), Condvar::new()));
        let glyphs = GlyphCache::new(&device, fonts, in_flight.clone());
        Renderer {
            device,
            queue,
            rect_pipeline,
            glyph_pipeline,
            glyphs,
            rect_buffers: std::array::from_fn(|_| InstanceBuffer { buffer: None, capacity: 0 }),
            glyph_buffers: std::array::from_fn(|_| InstanceBuffer { buffer: None, capacity: 0 }),
            frame_index: 0,
            in_flight,
            frame: Frame::default(),
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn metrics(&self) -> term_font::Metrics {
        self.glyphs.fonts.metrics
    }

    pub fn set_fonts(&mut self, fonts: Fonts) {
        self.glyphs.fonts = fonts;
        self.glyphs.clear();
    }

    /// Build this frame's instances. Call with the terminal locked; it only
    /// reads the terminal and touches no GPU state except new glyphs.
    pub fn prepare(&mut self, input: &FrameInput) {
        self.frame.build(input, &mut self.glyphs);
        // Out of glyph texture space: empty the texture that filled up,
        // growing it, and build again. This ends: each pass either grows a
        // texture (a few doublings to the cap) or, at the cap, empties it once
        // and draws what fits.
        while self.glyphs.needs_room() {
            let grew = self.glyphs.make_room(&self.device);
            self.frame.build(input, &mut self.glyphs);
            if !grew {
                break;
            }
        }
    }

    /// Encode and present the prepared frame.
    pub fn present(&mut self, layer: &CAMetalLayer) {
        {
            let (lock, cvar) = &*self.in_flight;
            let mut n = lock.lock().unwrap();
            while *n >= FRAMES_IN_FLIGHT {
                n = cvar.wait(n).unwrap();
            }
            *n += 1;
        }
        let Some(drawable) = layer.nextDrawable() else {
            self.frame_done();
            return;
        };
        let target = drawable.texture();
        let viewport = [target.width() as f32, target.height() as f32];

        let i = self.frame_index;
        self.frame_index = (i + 1) % FRAMES_IN_FLIGHT;
        let rects = self.rect_buffers[i].upload(&self.device, &self.frame.rects).map(|b| b.retain());
        let glyphs = self.glyph_buffers[i].upload(&self.device, &self.frame.glyphs).map(|b| b.retain());

        let pass = MTLRenderPassDescriptor::renderPassDescriptor();
        let att = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        att.setTexture(Some(&target));
        att.setLoadAction(MTLLoadAction::Clear);
        att.setStoreAction(MTLStoreAction::Store);
        let c = self.frame.clear;
        let a = (c >> 24) as f64 / 255.0;
        let ch = |shift: u32| ((c >> shift) & 0xFF) as f64 / 255.0 * a;
        att.setClearColor(MTLClearColor {
            red: ch(0),
            green: ch(8),
            blue: ch(16),
            alpha: a,
        });

        let cmd = self.queue.commandBuffer().expect("command buffer");
        let enc = cmd.renderCommandEncoderWithDescriptor(&pass).expect("encoder");
        unsafe {
            let uniforms = NonNull::from(&viewport).cast::<c_void>();
            if let Some(buf) = &rects {
                enc.setRenderPipelineState(&self.rect_pipeline);
                enc.setVertexBuffer_offset_atIndex(Some(buf), 0, 0);
                enc.setVertexBytes_length_atIndex(uniforms, 8, 1);
                enc.drawPrimitives_vertexStart_vertexCount_instanceCount(MTLPrimitiveType::TriangleStrip, 0, 4, self.frame.rects.len());
            }
            if let Some(buf) = &glyphs {
                enc.setRenderPipelineState(&self.glyph_pipeline);
                enc.setVertexBuffer_offset_atIndex(Some(buf), 0, 0);
                enc.setVertexBytes_length_atIndex(uniforms, 8, 1);
                enc.setFragmentTexture_atIndex(Some(&self.glyphs.mask.texture), 0);
                enc.setFragmentTexture_atIndex(Some(&self.glyphs.color.texture), 1);
                enc.drawPrimitives_vertexStart_vertexCount_instanceCount(MTLPrimitiveType::TriangleStrip, 0, 4, self.frame.glyphs.len());
            }
        }
        enc.endEncoding();

        let in_flight = self.in_flight.clone();
        let done = RcBlock::new(move |_: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
            let (lock, cvar) = &*in_flight;
            *lock.lock().unwrap() -= 1;
            cvar.notify_one();
        });
        unsafe { cmd.addCompletedHandler(RcBlock::as_ptr(&done)) };
        cmd.presentDrawable(ProtocolObject::from_ref(&*drawable));
        cmd.commit();
    }

    fn frame_done(&self) {
        let (lock, cvar) = &*self.in_flight;
        *lock.lock().unwrap() -= 1;
        cvar.notify_one();
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
}
