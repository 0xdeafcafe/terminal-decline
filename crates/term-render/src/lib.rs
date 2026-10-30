//! GPU rendering: terminal state in, one frame out.

pub mod frame;
mod images;
mod metal;
pub mod packer;
mod pixels;
mod row_cache;
mod shape_cache;
pub mod transition;

pub use frame::{
    CURSOR_TEXT_CONTRAST, Frame, FrameInput, GlyphInstance, GlyphSlot, GlyphSource, RectInstance, RectSplit, Rgba, SELECTED_TEXT_CONTRAST,
    SELECTION_CONTRAST, Theme, contrast, rgb,
};
pub use images::{ImageDraw, ImageInstance, Images};
pub use metal::{CommandBuffer, FontsId, PaneDraw, PaneRender, PxRect, Renderer, Texture, configure_layer, set_glyph_texture_sizes};
pub use packer::ShelfPacker;
pub use pixels::{MetalPixels, PixelAlloc};
pub use transition::{Blend, Kind as TransitionKind, Transition};
