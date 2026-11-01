//! Scroll-wheel and trackpad input for the view.

/// A scroll event's phase, or its momentum phase (`NSEventPhase`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    None,
    MayBegin,
    Began,
    Stationary,
    Changed,
    Ended,
    Cancelled,
}

impl Phase {
    /// From `NSEventPhase` bits.
    pub fn of(bits: usize) -> Phase {
        match bits {
            0x1 => Phase::Began,
            0x2 => Phase::Stationary,
            0x4 => Phase::Changed,
            0x8 => Phase::Ended,
            0x10 => Phase::Cancelled,
            0x20 => Phase::MayBegin,
            _ => Phase::None,
        }
    }
}

/// One scroll event, as `scrollWheel:` gets it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// Pixel deltas (a trackpad, Magic Mouse) rather than lines (a wheel).
    pub precise: bool,
    /// `scrollingDeltaY`: points when precise, lines otherwise; positive
    /// moves the content down, back into history (natural scrolling is
    /// already applied).
    pub dy: f64,
    pub phase: Phase,
    pub momentum: Phase,
    /// The event's timestamp (host seconds).
    pub t0: f64,
}
