//! Whether a display-link tick draws, and at what refresh rate.
//!
//! The view calls [`decide`] on every tick; the headless tests drive the same
//! function from a fake draw loop.

use std::time::Duration;

/// Longest we hold drawing while a program is mid synchronized update (2026).
pub const SYNC_TIMEOUT: Duration = Duration::from_millis(150);
/// How long after input the display runs at its full refresh rate.
pub const INPUT_BOOST: Duration = Duration::from_millis(250);
/// Idle time before the spare drawable is given back.
pub const IDLE_TRIM: Duration = Duration::from_secs(2);
/// Ticks with nothing new before the display link is paused.
pub const IDLE_TICKS_BEFORE_PAUSE: u8 = 2;

/// Refresh rates (Hz): right after input, while output streams, unfocused.
pub const RATE_INPUT: u16 = 120;
pub const RATE_STREAMING: u16 = 60;
pub const RATE_UNFOCUSED: u16 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Draw a frame, asking the display for `rate` Hz.
    Draw { rate: u16 },
    /// Nothing new; keep ticking.
    Skip,
    /// Nothing new for a while: pause the display link, and give the spare
    /// drawable back if still idle after `trim_after`.
    Pause { trim_after: Duration },
    /// The program is mid synchronized update: keep the change and keep
    /// ticking so the timeout is noticed.
    Hold,
    /// Hidden or fully covered: keep the change, pause, and let the renderer
    /// free what it can (keeping the surface for a minimised window, whose
    /// picture is shown in the Dock).
    Hide { keep_surface: bool },
}

/// One tick's decision.
///
/// - `dirty`: something changed since the last frame (the flag has just been
///   taken).
/// - `sync_elapsed`: how long synchronized-update mode has been on, if it is.
/// - `idle_ticks`: ticks in a row with nothing new before this one.
/// - `last_input_elapsed`: time since the last key, paste, click or scroll.
#[allow(clippy::too_many_arguments)]
pub fn decide(
    dirty: bool,
    visible: bool,
    synchronized: bool,
    sync_elapsed: Duration,
    idle_ticks: u8,
    minimised: bool,
    last_input_elapsed: Duration,
    focused: bool,
) -> Decision {
    if !dirty {
        // Nothing new: stop the display link until something changes.
        return if idle_ticks.saturating_add(1) == IDLE_TICKS_BEFORE_PAUSE {
            Decision::Pause { trim_after: IDLE_TRIM }
        } else {
            Decision::Skip
        };
    }
    if !visible {
        return Decision::Hide { keep_surface: minimised };
    }
    if synchronized && sync_elapsed < SYNC_TIMEOUT {
        return Decision::Hold;
    }
    // Full refresh rate right after input; 60 Hz while output streams (no
    // one reads text scrolling at 120 Hz, and the window server's cost
    // scales with frames); 30 Hz in a window that isn't focused.
    let rate = if last_input_elapsed < INPUT_BOOST {
        RATE_INPUT
    } else if !focused {
        RATE_UNFOCUSED
    } else {
        RATE_STREAMING
    };
    Decision::Draw { rate }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// One row per branch: (what, dirty, visible, synchronized,
    /// sync_elapsed, idle_ticks, minimised, last_input_elapsed, focused,
    /// decision).
    #[test]
    fn every_branch() {
        let long = Duration::from_secs(5);
        type Row = (&'static str, bool, bool, bool, Duration, u8, bool, Duration, bool, Decision);
        let rows: &[Row] = &[
            (
                "first idle tick",
                false,
                true,
                false,
                Duration::ZERO,
                0,
                false,
                long,
                true,
                Decision::Skip,
            ),
            (
                "second idle tick pauses",
                false,
                true,
                false,
                Duration::ZERO,
                1,
                false,
                long,
                true,
                Decision::Pause { trim_after: IDLE_TRIM },
            ),
            (
                "later idle ticks skip",
                false,
                true,
                false,
                Duration::ZERO,
                2,
                false,
                long,
                true,
                Decision::Skip,
            ),
            (
                "idle at the counter's cap",
                false,
                true,
                false,
                Duration::ZERO,
                u8::MAX,
                false,
                long,
                true,
                Decision::Skip,
            ),
            (
                "idle beats hidden",
                false,
                false,
                true,
                Duration::ZERO,
                1,
                true,
                long,
                true,
                Decision::Pause { trim_after: IDLE_TRIM },
            ),
            (
                "hidden",
                true,
                false,
                false,
                Duration::ZERO,
                0,
                false,
                long,
                true,
                Decision::Hide { keep_surface: false },
            ),
            (
                "minimised keeps its surface",
                true,
                false,
                false,
                Duration::ZERO,
                0,
                true,
                long,
                true,
                Decision::Hide { keep_surface: true },
            ),
            (
                "hidden beats synchronized",
                true,
                false,
                true,
                Duration::ZERO,
                0,
                false,
                long,
                true,
                Decision::Hide { keep_surface: false },
            ),
            (
                "synchronized just started",
                true,
                true,
                true,
                Duration::ZERO,
                0,
                false,
                long,
                true,
                Decision::Hold,
            ),
            (
                "synchronized 149 ms",
                true,
                true,
                true,
                149 * MS,
                0,
                false,
                long,
                true,
                Decision::Hold,
            ),
            (
                "synchronized timed out",
                true,
                true,
                true,
                150 * MS,
                0,
                false,
                long,
                true,
                Decision::Draw { rate: RATE_STREAMING },
            ),
            (
                "not synchronized ignores elapsed",
                true,
                true,
                false,
                Duration::ZERO,
                0,
                false,
                long,
                true,
                Decision::Draw { rate: 60 },
            ),
            (
                "just typed",
                true,
                true,
                false,
                Duration::ZERO,
                0,
                false,
                10 * MS,
                true,
                Decision::Draw { rate: 120 },
            ),
            (
                "typed 249 ms ago",
                true,
                true,
                false,
                Duration::ZERO,
                0,
                false,
                249 * MS,
                true,
                Decision::Draw { rate: 120 },
            ),
            (
                "typed 250 ms ago",
                true,
                true,
                false,
                Duration::ZERO,
                0,
                false,
                250 * MS,
                true,
                Decision::Draw { rate: 60 },
            ),
            (
                "just typed, unfocused",
                true,
                true,
                false,
                Duration::ZERO,
                0,
                false,
                10 * MS,
                false,
                Decision::Draw { rate: 120 },
            ),
            (
                "unfocused",
                true,
                true,
                false,
                Duration::ZERO,
                0,
                false,
                long,
                false,
                Decision::Draw { rate: 30 },
            ),
            (
                "dirty resets idle",
                true,
                true,
                false,
                Duration::ZERO,
                1,
                false,
                long,
                true,
                Decision::Draw { rate: 60 },
            ),
        ];
        for &(what, dirty, visible, sync, elapsed, idle, minimised, input, focused, want) in rows {
            assert_eq!(
                decide(dirty, visible, sync, elapsed, idle, minimised, input, focused),
                want,
                "{what}"
            );
        }
    }

    #[test]
    fn constants_match_the_design() {
        assert_eq!(SYNC_TIMEOUT, 150 * MS);
        assert_eq!(IDLE_TICKS_BEFORE_PAUSE, 2);
        assert_eq!((RATE_INPUT, RATE_STREAMING, RATE_UNFOCUSED), (120, 60, 30));
    }
}
