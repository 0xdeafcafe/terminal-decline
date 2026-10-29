//! Whether a display-link tick draws, and at what refresh rate.
//!
//! With panes, each tick asks [`pane`] about every visible pane (build its
//! frame, reuse its last one, hold it in a synchronized update, or wait
//! for an echo), then [`combine`] decides for the window, which has one
//! display link and one idle counter. [`decide`] is the same for a window
//! of one pane. The headless tests drive these from a fake draw loop.

use std::time::Duration;

/// Longest we hold drawing while a program is mid synchronized update (2026).
pub const SYNC_TIMEOUT: Duration = Duration::from_millis(150);
/// How long after input the display runs at its full refresh rate.
pub const INPUT_BOOST: Duration = Duration::from_millis(250);
/// Idle time before the spare drawable is given back.
pub const IDLE_TRIM: Duration = Duration::from_secs(2);
/// Ticks with nothing new before the display link is paused.
pub const IDLE_TICKS_BEFORE_PAUSE: u8 = 2;

/// How long a tick waits for the echo of input before drawing without it.
pub const ECHO_WAIT: Duration = Duration::from_millis(8);

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
    let frame = pane(dirty, synchronized, sync_elapsed, None);
    combine(
        &[frame],
        &WindowTick {
            visible,
            minimised,
            focused,
            idle_ticks,
            last_input_elapsed,
        },
    )
}

/// What one pane does this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneFrame {
    /// Changed: build its frame.
    Build,
    /// Unchanged: draw its last frame.
    Reuse,
    /// Changed, but its program is mid synchronized update: draw its last
    /// frame and keep the change.
    Held,
    /// Input was sent and its echo hasn't come: draw its last frame and
    /// keep ticking until the echo or [`ECHO_WAIT`].
    Waiting,
}

/// One pane's part of a tick.
///
/// - `dirty`: it changed since its last frame (the flag has just been
///   taken).
/// - `synchronized`, `sync_elapsed`: in synchronized-update mode, and for
///   how long.
/// - `echo_elapsed`: time since input was sent to it, while its echo is
///   awaited. At [`ECHO_WAIT`] the pane builds anyway, to show what the
///   input changed (cursor, scroll); the caller stops waiting then.
pub fn pane(dirty: bool, synchronized: bool, sync_elapsed: Duration, echo_elapsed: Option<Duration>) -> PaneFrame {
    let waited_out = echo_elapsed.is_some_and(|e| e >= ECHO_WAIT);
    if !dirty && !waited_out {
        return if echo_elapsed.is_some() {
            PaneFrame::Waiting
        } else {
            PaneFrame::Reuse
        };
    }
    if synchronized && sync_elapsed < SYNC_TIMEOUT {
        return PaneFrame::Held;
    }
    PaneFrame::Build
}

/// The window's part of a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowTick {
    pub visible: bool,
    pub minimised: bool,
    /// The window is key. The rate follows the window's focus, not the
    /// pane's, so output watched in an unfocused pane isn't slowed.
    pub focused: bool,
    /// Ticks in a row with nothing built, held or awaited before this one.
    pub idle_ticks: u8,
    /// Time since the last key, paste, click or scroll in any pane.
    pub last_input_elapsed: Duration,
}

/// The window's decision from its visible panes' (panes hidden by a zoom
/// aren't asked: they stay dirty until shown). Any pane building draws a
/// frame, the others from their last one; a pane held or waiting with
/// none building keeps the link ticking; otherwise the window is idle, so
/// any number of idle panes cost no wake-ups.
pub fn combine(panes: &[PaneFrame], w: &WindowTick) -> Decision {
    let builds = panes.contains(&PaneFrame::Build);
    let busy = builds || panes.iter().any(|p| matches!(p, PaneFrame::Held | PaneFrame::Waiting));
    if !busy {
        // Nothing new: stop the display link until something changes.
        return if w.idle_ticks.saturating_add(1) == IDLE_TICKS_BEFORE_PAUSE {
            Decision::Pause { trim_after: IDLE_TRIM }
        } else {
            Decision::Skip
        };
    }
    if !w.visible {
        return Decision::Hide { keep_surface: w.minimised };
    }
    if !builds {
        return Decision::Hold;
    }
    // Full refresh rate right after input; 60 Hz while output streams (no
    // one reads text scrolling at 120 Hz, and the window server's cost
    // scales with frames); 30 Hz in a window that isn't focused.
    let rate = if w.last_input_elapsed < INPUT_BOOST {
        RATE_INPUT
    } else if !w.focused {
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
    fn each_pane_builds_reuses_holds_or_waits() {
        use PaneFrame::*;
        let z = Duration::ZERO;
        // (what, dirty, synchronized, sync_elapsed, echo_elapsed, frame)
        type Row = (&'static str, bool, bool, Duration, Option<Duration>, PaneFrame);
        let rows: &[Row] = &[
            ("clean", false, false, z, None, Reuse),
            ("dirty", true, false, z, None, Build),
            ("held", true, true, 149 * MS, None, Held),
            ("held 150 ms builds", true, true, 150 * MS, None, Build),
            ("clean in sync mode", false, true, z, None, Reuse),
            ("awaiting an echo", false, false, z, Some(3 * MS), Waiting),
            ("awaiting 7 ms", false, false, z, Some(7 * MS), Waiting),
            ("no echo after 8 ms", false, false, z, Some(8 * MS), Build),
            ("the echo came", true, false, z, Some(2 * MS), Build),
            ("echo waited out, held", false, true, z, Some(8 * MS), Held),
        ];
        for &(what, dirty, sync, elapsed, echo, want) in rows {
            assert_eq!(pane(dirty, sync, elapsed, echo), want, "{what}");
        }
    }

    fn tick(idle_ticks: u8) -> WindowTick {
        WindowTick {
            visible: true,
            minimised: false,
            focused: true,
            idle_ticks,
            last_input_elapsed: Duration::from_secs(5),
        }
    }

    #[test]
    fn the_window_combines_its_panes() {
        use PaneFrame::*;
        let rows: &[(&str, &[PaneFrame], WindowTick, Decision)] = &[
            (
                "one builds, one held: draw, the held one reused",
                &[Build, Held],
                tick(0),
                Decision::Draw { rate: 60 },
            ),
            (
                "one builds, others clean",
                &[Reuse, Build, Reuse],
                tick(1),
                Decision::Draw { rate: 60 },
            ),
            ("held only: keep ticking", &[Held, Reuse], tick(0), Decision::Hold),
            ("awaiting an echo: keep ticking", &[Waiting, Reuse], tick(1), Decision::Hold),
            ("all clean, first tick", &[Reuse, Reuse], tick(0), Decision::Skip),
            (
                "all clean, second tick",
                &[Reuse, Reuse],
                tick(1),
                Decision::Pause { trim_after: IDLE_TRIM },
            ),
            ("all clean, later", &[Reuse, Reuse], tick(2), Decision::Skip),
            // A zoom hides the flooding pane: it isn't passed in.
            (
                "zoomed, the shown pane clean",
                &[Reuse],
                tick(1),
                Decision::Pause { trim_after: IDLE_TRIM },
            ),
            ("no visible panes", &[], tick(1), Decision::Pause { trim_after: IDLE_TRIM }),
            (
                "hidden",
                &[Build],
                WindowTick { visible: false, ..tick(0) },
                Decision::Hide { keep_surface: false },
            ),
            (
                "minimised, held",
                &[Held],
                WindowTick {
                    visible: false,
                    minimised: true,
                    ..tick(0)
                },
                Decision::Hide { keep_surface: true },
            ),
            // The rate is the window's: from input and the window's focus.
            (
                "just typed",
                &[Reuse, Build],
                WindowTick {
                    last_input_elapsed: 10 * MS,
                    ..tick(0)
                },
                Decision::Draw { rate: 120 },
            ),
            (
                "window not key",
                &[Build, Build],
                WindowTick { focused: false, ..tick(0) },
                Decision::Draw { rate: 30 },
            ),
        ];
        for (what, panes, w, want) in rows {
            assert_eq!(combine(panes, w), *want, "{what}");
        }
    }

    /// Panes going idle out of step: the window pauses once, two ticks
    /// after the last one went quiet, and never while one is busy.
    #[test]
    fn panes_going_idle_out_of_step_pause_once() {
        // Pane 0 is dirty for ticks 0..3, pane 1 for 0..6, pane 2 holds a
        // synchronized update for ticks 2..8 (under the timeout).
        let mut idle = 0u8;
        let mut pauses = Vec::new();
        for t in 0..14u32 {
            let frames = [
                pane(t < 3, false, Duration::ZERO, None),
                pane(t < 6, false, Duration::ZERO, None),
                pane((2..8).contains(&t), (2..8).contains(&t), (t.saturating_sub(2)) * 10 * MS, None),
            ];
            let d = combine(&frames, &tick(idle));
            match d {
                Decision::Skip => idle = idle.saturating_add(1),
                Decision::Pause { .. } => {
                    idle = idle.saturating_add(1);
                    pauses.push(t);
                }
                _ => idle = 0,
            }
            if t < 8 {
                assert!(matches!(d, Decision::Draw { .. } | Decision::Hold), "tick {t}: {d:?}");
            }
        }
        assert_eq!(pauses, [9]);
    }

    #[test]
    fn constants_match_the_design() {
        assert_eq!(ECHO_WAIT, 8 * MS);
        assert_eq!(SYNC_TIMEOUT, 150 * MS);
        assert_eq!(IDLE_TICKS_BEFORE_PAUSE, 2);
        assert_eq!((RATE_INPUT, RATE_STREAMING, RATE_UNFOCUSED), (120, 60, 30));
    }
}
