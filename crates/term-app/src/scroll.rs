//! Scroll-wheel and trackpad input for the view: a trackpad (precise
//! deltas) moves it by its exact pixels, the part of a line between whole
//! lines drawn as a shift of the pane's content ([`Smooth`]); a mouse
//! wheel, and any scrolling a program takes (its mouse reports, arrow keys
//! on the alternate screen), go by whole lines.

use std::time::{Duration, Instant};

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

/// How long after the last scroll event a gesture counts as over when no
/// phase said so (and between a lifted finger and its momentum).
pub const SETTLE_AFTER: Duration = Duration::from_millis(60);
/// How long a finger resting on the trackpad, or momentum without events,
/// counts as still moving the view.
pub const RESTING: Duration = Duration::from_secs(1);

/// The view's position under a trackpad: whole lines in the terminal
/// (its view offset), and between them `shift`, the device pixels the
/// content is drawn higher than the whole-line view shows it, with the
/// row below the view (term-core's overscan) coming up under it.
#[derive(Clone, Debug, Default)]
pub struct Smooth {
    /// 0 ≤ shift < the cell height; 0 while following output.
    shift: f64,
    /// A finger is on the trackpad (phase began, changed or stationary).
    touching: bool,
    /// Momentum after the finger lifted.
    momentum: bool,
    /// When the last scroll event of a gesture came.
    last: Option<Instant>,
    /// Lines not yet whole: from a wheel, or pixels while the view moves
    /// by lines only (a program's mouse reports, arrow keys).
    carry: f64,
}

impl Smooth {
    /// Device pixels the content is drawn higher than its frame places it.
    pub fn shift(&self) -> f64 {
        self.shift
    }

    /// [`shift`](Self::shift) in whole pixels, as drawn: under the cell
    /// height `cell`.
    pub fn shift_px(&self, cell: f64) -> i32 {
        (self.shift.round() as i32).min(cell as i32 - 1).max(0)
    }

    /// A scroll event's phases: whether a finger or momentum is moving the
    /// view, and when it last did.
    pub fn event(&mut self, input: &Input, now: Instant) {
        match input.phase {
            Phase::Began | Phase::Changed | Phase::Stationary | Phase::MayBegin => {
                self.touching = true;
                self.momentum = false;
            }
            Phase::Ended | Phase::Cancelled => self.touching = false,
            Phase::None => {}
        }
        match input.momentum {
            Phase::Began | Phase::Changed | Phase::Stationary => self.momentum = true,
            Phase::Ended | Phase::Cancelled => {
                self.momentum = false;
                // Its last event: settle from the next tick.
                self.last = None;
                return;
            }
            Phase::MayBegin | Phase::None => {}
        }
        self.last = Some(now);
    }

    /// The gesture, or its momentum, may still move the view: the display
    /// link keeps ticking for it, and the shift isn't settled yet.
    pub fn active(&self, now: Instant) -> bool {
        let since = self.last.map(|l| now.saturating_duration_since(l));
        // A finger resting without moving for a while doesn't keep the
        // display link ticking: its next move starts it again.
        let held = (self.touching || self.momentum) && since.is_some_and(|s| s < RESTING);
        held || since.is_some_and(|s| s < SETTLE_AFTER)
    }

    /// Move the view by `px` device pixels (positive: back into history)
    /// from `offset` whole lines back, with cells `cell` pixels high: the
    /// lines to scroll the terminal's view by; the rest is the shift. Never
    /// below following output.
    pub fn pixels(&mut self, px: f64, cell: f64, offset: usize) -> isize {
        let pos = offset as f64 * cell - self.shift;
        let to = (pos + px).max(0.0);
        let lines = (to / cell).ceil();
        self.shift = lines * cell - to;
        if self.shift >= cell - 1e-9 {
            self.shift = 0.0;
        }
        lines as isize - offset as isize
    }

    /// The view was scrolled, asked to land `want` lines back, and is
    /// `offset` back: short of it (the top of history) or following output,
    /// there is nothing to draw between lines.
    pub fn landed(&mut self, want: usize, offset: usize) {
        if offset != want || offset == 0 {
            self.shift = 0.0;
        }
    }

    /// A display-link tick: once the gesture is over, ease the shift to
    /// the nearest whole line, a step a tick. `Some(lines)` while it moves
    /// (the terminal's view to scroll by: -1 when it ends a line further
    /// on), `None` when there is nothing to do.
    pub fn settle(&mut self, now: Instant, cell: f64) -> Option<isize> {
        if self.shift == 0.0 || self.active(now) {
            return None;
        }
        let target = if self.shift < cell / 2.0 { 0.0 } else { cell };
        self.shift += (target - self.shift) * 0.5;
        if (target - self.shift).abs() < 1.0 {
            self.shift = 0.0;
            return Some(if target == 0.0 { 0 } else { -1 });
        }
        Some(0)
    }

    /// End any gesture and drop to the nearest whole line at once: the
    /// lines to scroll the view by (0 or -1). For a click (cells are hit
    /// at whole lines), a wheel, or a program that takes the scrolling.
    pub fn snap(&mut self, cell: f64) -> isize {
        let lines = if self.shift >= cell / 2.0 { -1 } else { 0 };
        *self = Smooth {
            carry: self.carry,
            ..Smooth::default()
        };
        lines
    }

    /// Following output again (or a new terminal): nothing between lines.
    pub fn reset(&mut self) {
        let carry = self.carry;
        *self = Smooth::default();
        self.carry = carry;
    }

    /// Whole lines out of `lines` (a wheel's, or pixels in lines), the
    /// rest kept for the next event.
    pub fn whole(&mut self, lines: f64) -> isize {
        self.carry += lines;
        let whole = self.carry.trunc();
        self.carry -= whole;
        whole as isize
    }

    /// Drop the lines not yet whole (a click starts afresh).
    pub fn forget_carry(&mut self) {
        self.carry = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: f64 = 32.0;

    fn input(phase: Phase, momentum: Phase) -> Input {
        Input {
            precise: true,
            phase,
            momentum,
            ..Input::default()
        }
    }

    /// The view's position in pixels from following output.
    fn pos(s: &Smooth, offset: usize) -> f64 {
        offset as f64 * CELL - s.shift()
    }

    #[test]
    fn pixels_move_the_view_by_exactly_their_delta() {
        let mut s = Smooth::default();
        let mut offset = 0usize;
        let mut want = 0.0;
        for px in [5.0, 5.0, 30.0, 0.5, 64.0, -10.0, -3.25, 100.0, -90.0] {
            want = f64::max(want + px, 0.0);
            let lines = s.pixels(px, CELL, offset);
            offset = (offset as isize + lines) as usize;
            s.landed(offset, offset);
            assert!((pos(&s, offset) - want).abs() < 1e-9, "{px}: at {} want {want}", pos(&s, offset));
            assert!((0.0..CELL).contains(&s.shift()), "{px}: shift {}", s.shift());
        }
    }

    #[test]
    fn the_first_pixels_back_take_a_line_and_shift_it_down() {
        let mut s = Smooth::default();
        // 5 px back from following output: a line back, drawn 27 px higher.
        assert_eq!(s.pixels(5.0, CELL, 0), 1);
        assert_eq!(s.shift(), CELL - 5.0);
        assert_eq!(s.shift_px(CELL), 27);
        // A whole line exactly: no shift.
        assert_eq!(s.pixels(27.0, CELL, 1), 0);
        assert_eq!(s.shift(), 0.0);
        // Back to following output, and further down stays there.
        assert_eq!(s.pixels(-40.0, CELL, 1), -1);
        assert_eq!(s.shift(), 0.0);
        assert_eq!(s.pixels(-40.0, CELL, 0), 0);
        assert_eq!(s.shift(), 0.0);
    }

    #[test]
    fn the_top_of_history_drops_the_shift() {
        let mut s = Smooth::default();
        // At the top, 10 lines back: further back is asked for, the view stays.
        let lines = s.pixels(8.0, CELL, 10);
        assert_eq!(lines, 1);
        s.landed(11, 10);
        assert_eq!(s.shift(), 0.0);
        // Following output: nothing between lines either.
        s.pixels(-8.0, CELL, 1);
        s.landed(1, 0);
        assert_eq!(s.shift(), 0.0);
    }

    #[test]
    fn a_gesture_is_active_until_its_momentum_ends() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut s = Smooth::default();
        assert!(!s.active(t));
        s.event(&input(Phase::Began, Phase::None), t);
        // A finger resting on the trackpad keeps it going, for a while.
        assert!(s.active(ms(500)));
        assert!(!s.active(ms(1500)));
        s.event(&input(Phase::Ended, Phase::None), ms(500));
        // Lifted: momentum may follow within a moment.
        assert!(s.active(ms(530)));
        assert!(!s.active(ms(600)));
        s.event(&input(Phase::None, Phase::Began), ms(510));
        assert!(s.active(ms(1400)), "momentum runs on");
        s.event(&input(Phase::None, Phase::Ended), ms(2000));
        assert!(!s.active(ms(2000)), "over at once");
    }

    #[test]
    fn settles_to_the_nearest_line_once_over() {
        let t = Instant::now();
        let later = t + Duration::from_secs(1);
        // Under half a line: eases back to this line.
        let mut s = Smooth::default();
        s.event(&input(Phase::Changed, Phase::None), t);
        s.pixels(-10.0, CELL, 3);
        assert_eq!(s.shift(), 10.0);
        assert_eq!(s.settle(t, CELL), None, "not while the finger is down");
        s.event(&input(Phase::Ended, Phase::None), t);
        let mut steps = 0;
        let mut lines = 0;
        while let Some(l) = s.settle(later, CELL) {
            lines += l;
            steps += 1;
            assert!(steps < 20);
        }
        assert_eq!((s.shift(), lines), (0.0, 0));
        assert!(steps > 1, "eased over a few ticks: {steps}");
        // Over half: eases on to the next line.
        let mut s = Smooth::default();
        s.pixels(-20.0, CELL, 3);
        assert_eq!(s.shift(), 20.0);
        let mut lines = 0;
        while let Some(l) = s.settle(later, CELL) {
            lines += l;
        }
        assert_eq!((s.shift(), lines), (0.0, -1));
    }

    #[test]
    fn snaps_at_once_and_wheels_keep_whole_lines() {
        let mut s = Smooth::default();
        s.pixels(-20.0, CELL, 3);
        assert_eq!(s.snap(CELL), -1);
        assert_eq!(s.shift(), 0.0);
        s.pixels(-10.0, CELL, 3);
        assert_eq!(s.snap(CELL), 0);
        assert!(!s.active(Instant::now()));
        // A wheel's lines (times its multiplier) add up to whole lines.
        assert_eq!(s.whole(1.5), 1);
        assert_eq!(s.whole(1.5), 2);
        assert_eq!(s.whole(-0.25), 0);
        assert_eq!(s.whole(-0.75), -1);
    }

    #[test]
    fn phases_from_their_bits() {
        assert_eq!(Phase::of(0), Phase::None);
        assert_eq!(Phase::of(1), Phase::Began);
        assert_eq!(Phase::of(4), Phase::Changed);
        assert_eq!(Phase::of(8), Phase::Ended);
        assert_eq!(Phase::of(0x20), Phase::MayBegin);
    }
}
