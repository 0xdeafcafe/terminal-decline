//! Giving memory back when macOS says it is short (roadmap R.5).
//!
//! A dispatch source on the main queue hears the system's memory-pressure
//! warnings (warning and critical; not the return to normal). On each, the
//! app gives back what it can make again later, without changing what any
//! window shows:
//!
//! - every window's spare drawable (as the idle trim does);
//! - the glyph textures, which start again empty at their starting sizes,
//!   and each window's instance buffers and image textures
//!   ([`term_render::Renderer::purge`]); panes are built again before
//!   their glyphs are drawn;
//! - every pane's history: each sealed block compressed now, the cache
//!   of decompressed blocks and the search filters dropped
//!   (`Terminal::shrink_history`; filters are built again when idle);
//! - decoded images not drawn in their window's latest frame that can be
//!   decoded again from their encoded copy ([`term_core::ImagePool::trim`]).
//!
//! Then every window draws again. The test control's `memory-pressure`
//! command runs the same steps, so a smoke test can check the footprint
//! drops and the windows draw as before.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use dispatch2::{DispatchObject, DispatchQueue, DispatchRetained, DispatchSource, dispatch_source_memorypressure_flags_t as Flags};

/// What one relief gave back, bytes. `render` and `images` are what the
/// textures, buffers and pixels held; `history` is the drop in what
/// history counts; the footprint is the process's, before and after (the
/// GPU and the allocator may give pages back later than this).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Freed {
    pub windows: usize,
    pub panes: usize,
    pub render: usize,
    pub history: usize,
    pub images: usize,
    pub footprint_before: u64,
    pub footprint_after: u64,
}

impl Freed {
    /// Add one window's part.
    pub fn add(&mut self, other: Freed) {
        self.windows += other.windows;
        self.panes += other.panes;
        self.render += other.render;
        self.history += other.history;
        self.images += other.images;
    }
}

static RELIEFS: AtomicU64 = AtomicU64::new(0);

/// Times memory was given back since launch (warnings and test commands).
pub fn reliefs() -> u64 {
    RELIEFS.load(Ordering::Relaxed)
}

/// Start listening for memory-pressure warnings: once, on the main thread
/// at launch. The handler runs on the main queue.
pub fn listen() {
    static SOURCE: OnceLock<DispatchRetained<DispatchSource>> = OnceLock::new();
    SOURCE.get_or_init(|| {
        let mask = (Flags::DISPATCH_MEMORYPRESSURE_WARN.0 | Flags::DISPATCH_MEMORYPRESSURE_CRITICAL.0) as usize;
        // SAFETY: the memory-pressure type takes no handle and this mask;
        // the main queue lives for the process.
        let source = unsafe {
            DispatchSource::new(
                (&raw const dispatch2::_dispatch_source_type_memorypressure).cast_mut(),
                0,
                mask,
                Some(DispatchQueue::main()),
            )
        };
        source.set_event_handler_f(warned);
        source.activate();
        source
    });
}

/// The source fired (on the main queue).
extern "C" fn warned(_context: *mut std::ffi::c_void) {
    relieve("memory pressure");
}

/// Give back what every window can make again, and draw them again.
/// Main thread only. `why` goes in the log line.
pub fn relieve(why: &str) -> Freed {
    RELIEFS.fetch_add(1, Ordering::Relaxed);
    let mut freed = Freed {
        footprint_before: crate::control::phys_footprint(),
        ..Freed::default()
    };
    for view in crate::view::all_views() {
        freed.add(view.give_back_memory());
    }
    freed.images = term_core::ImagePool::process().trim();
    freed.footprint_after = crate::control::phys_footprint();
    eprintln!(
        "terminal-decline: {why}: gave back {:.1} MB of glyph textures and buffers, {:.1} MB of history ({} panes), {:.1} MB of images; footprint {:.1} -> {:.1} MB",
        mb(freed.render),
        mb(freed.history),
        freed.panes,
        mb(freed.images),
        freed.footprint_before as f64 / 1e6,
        freed.footprint_after as f64 / 1e6,
    );
    freed
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relief_without_windows_still_counts_and_trims_images() {
        let before = reliefs();
        let freed = relieve("test");
        assert_eq!((freed.windows, freed.panes, freed.render, freed.history), (0, 0, 0, 0));
        assert!(freed.footprint_before > 0 && freed.footprint_after > 0);
        assert!(reliefs() > before);
    }

    #[test]
    fn listening_twice_makes_one_source() {
        listen();
        listen();
    }
}
