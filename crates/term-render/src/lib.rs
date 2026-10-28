//! GPU rendering: terminal state in, one frame out.

pub mod frame;
mod metal;

pub use frame::{Frame, FrameInput, GlyphInstance, GlyphSlot, GlyphSource, RectInstance, Rgba, Theme, rgb};
pub use metal::{GlyphCache, Renderer, configure_layer};
