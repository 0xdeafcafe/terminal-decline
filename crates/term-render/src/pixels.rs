//! Image pixels in Metal buffers (images.md §6.2): the decoders write
//! into a shared-storage `MTLBuffer` whose rows are padded to what Metal
//! needs to make a texture over it, so drawing an image needs no copy and
//! no upload.

use std::any::Any;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{
    MTLBuffer, MTLCreateSystemDefaultDevice, MTLDevice, MTLPixelFormat, MTLPurgeableState, MTLResource, MTLResourceOptions, MTLStorageMode,
    MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use term_core::PixelBuffer;
use term_image::PixelTarget;

type Device = ProtocolObject<dyn MTLDevice>;
type Buffer = ProtocolObject<dyn MTLBuffer>;

/// The largest side Metal makes a 2D texture with on the Macs we run on.
pub const MAX_SIDE: u32 = 16384;

/// Premultiplied BGRA8 pixels in a shared-storage Metal buffer, rows
/// `row_pitch` bytes apart.
pub struct MetalPixels {
    buffer: Retained<Buffer>,
    width: u32,
    height: u32,
    pitch: usize,
}

// SAFETY: Metal buffers may be used from any thread (Apple: "MTLResource
// objects are thread-safe"); the bytes are only written through
// `&mut self`, before the image is ready to be drawn.
unsafe impl Send for MetalPixels {}
unsafe impl Sync for MetalPixels {}

impl MetalPixels {
    /// A `width` × `height` buffer on `device`, zeroed (transparent);
    /// `None` for an empty or too large image, or when Metal has no room.
    pub fn new(device: &Device, width: u32, height: u32) -> Option<MetalPixels> {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return None;
        }
        let align = device
            .minimumLinearTextureAlignmentForPixelFormat(MTLPixelFormat::BGRA8Unorm)
            .max(1);
        let pitch = (width as usize * 4).next_multiple_of(align);
        let buffer = device.newBufferWithLength_options(pitch * height as usize, MTLResourceOptions::StorageModeShared)?;
        Some(MetalPixels {
            buffer,
            width,
            height,
            pitch,
        })
    }

    /// A texture over the buffer, sharing its memory (no copy).
    pub fn texture(&self) -> Option<Retained<ProtocolObject<dyn MTLTexture>>> {
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                self.width as usize,
                self.height as usize,
                false,
            )
        };
        desc.setStorageMode(MTLStorageMode::Shared);
        desc.setUsage(MTLTextureUsage::ShaderRead);
        self.buffer.newTextureWithDescriptor_offset_bytesPerRow(&desc, 0, self.pitch)
    }

    /// The pixels' bytes, for reading back in tests.
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the buffer is `pitch * height` bytes of shared memory.
        unsafe { std::slice::from_raw_parts(self.buffer.contents().as_ptr() as *const u8, self.pitch * self.height as usize) }
    }
}

impl PixelTarget for MetalPixels {
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
    fn row_pitch(&self) -> usize {
        self.pitch
    }
    fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: as `bytes`, and `&mut self` is the only writer.
        unsafe { std::slice::from_raw_parts_mut(self.buffer.contents().as_ptr() as *mut u8, self.pitch * self.height as usize) }
    }
}

impl PixelBuffer for MetalPixels {
    fn allocated_size(&self) -> usize {
        self.buffer.allocatedSize()
    }
    fn set_volatile(&self, volatile: bool) -> bool {
        let state = if volatile {
            MTLPurgeableState::Volatile
        } else {
            MTLPurgeableState::NonVolatile
        };
        self.buffer.setPurgeableState(state) != MTLPurgeableState::Empty
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Makes [`MetalPixels`] from any thread (the app's decode queue, I.5a).
#[derive(Clone)]
pub struct PixelAlloc {
    device: Retained<Device>,
}

impl PixelAlloc {
    /// On the system's default Metal device, which the renderers draw with.
    pub fn system_default() -> Option<PixelAlloc> {
        MTLCreateSystemDefaultDevice().map(|device| PixelAlloc { device })
    }

    pub(crate) fn new(device: Retained<Device>) -> PixelAlloc {
        PixelAlloc { device }
    }

    pub fn alloc(&self, width: u32, height: u32) -> Option<Box<dyn PixelBuffer>> {
        Some(Box::new(MetalPixels::new(&self.device, width, height)?))
    }

    /// [`alloc`](Self::alloc), as its own type.
    pub fn alloc_metal(&self, width: u32, height: u32) -> Option<MetalPixels> {
        MetalPixels::new(&self.device, width, height)
    }
}
