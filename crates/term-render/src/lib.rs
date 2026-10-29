//! GPU rendering: terminal state in, one frame out.

pub mod frame;
mod metal;
pub mod packer;
mod row_cache;
mod shape_cache;

pub use frame::{Frame, FrameInput, GlyphInstance, GlyphSlot, GlyphSource, RectInstance, Rgba, Theme, rgb};
pub use metal::{CommandBuffer, FontsId, PaneDraw, PaneRender, PxRect, Renderer, Texture, configure_layer, set_glyph_texture_sizes};
pub use packer::ShelfPacker;
