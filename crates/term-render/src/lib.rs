//! GPU rendering: terminal state in, one frame out.

pub mod frame;
mod metal;
pub mod packer;
mod row_cache;

pub use frame::{Frame, FrameInput, GlyphInstance, GlyphSlot, GlyphSource, RectInstance, Rgba, Theme, rgb};
pub use metal::{CommandBuffer, GlyphCache, Renderer, Texture, configure_layer};
pub use packer::ShelfPacker;
