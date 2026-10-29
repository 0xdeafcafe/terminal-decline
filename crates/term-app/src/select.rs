//! Mouse selection, the parts the view decides: which term-core
//! selection call a click makes, how fast a drag past the top or bottom
//! edge scrolls, and rectangular (Option-drag) selections.
//!
//! term-core's selection is a run of text from one point to another, with
//! no rectangular mode, so a rectangle is kept here: its corners, and the
//! text in it, read one line at a time through `selection_text`. While one
//! is made, term-core holds (and the renderer draws) the run of text
//! between its corners.

use term_core::{Point, Selection, Terminal};

/// What a left click does when the program isn't taking the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// Start a selection at the cell (a click); dragging extends it.
    Start,
    /// Start a rectangular selection (Option-click).
    Block,
    /// Select the word (double-click, `select_word`).
    Word,
    /// Select the logical line (triple-click, `select_line`).
    Line,
    /// Move the end of the selection here (Shift-click,
    /// `extend_selection`), growing by words or lines if it was made by
    /// a double- or triple-click.
    Extend,
}

/// The gesture for a click: Shift extends whatever the click count, then
/// two clicks select a word, three or more a line, and Option makes a
/// single click start a rectangle.
pub fn gesture(clicks: isize, shift: bool, option: bool) -> Gesture {
    match clicks {
        _ if shift => Gesture::Extend,
        2 => Gesture::Word,
        c if c >= 3 => Gesture::Line,
        _ if option => Gesture::Block,
        _ => Gesture::Start,
    }
}

/// Whether letting go of the button copies the selection (with
/// `copy_on_select` on): a word, line or Shift-click always made one; a
/// plain or rectangular click only if the mouse was dragged.
pub fn copies_on_release(gesture: Gesture, dragged: bool) -> bool {
    match gesture {
        Gesture::Word | Gesture::Line | Gesture::Extend => true,
        Gesture::Start | Gesture::Block => dragged,
    }
}

/// Auto-scroll speed per cell height past the edge, in lines a second.
pub const SPEED_PER_LINE: f64 = 10.0;
/// Slowest auto-scroll, just past the edge (lines a second).
pub const MIN_SPEED: f64 = 3.0;
/// Fastest auto-scroll (lines a second).
pub const MAX_SPEED: f64 = 150.0;
/// How often auto-scroll moves while the mouse is held past an edge.
pub const INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);
/// The longest time one step makes up for, so a stalled main thread
/// doesn't jump the view.
const MAX_STEP: f64 = 0.1;

/// Auto-scroll speed in lines a second for a drag at `y` pixels from the
/// top of the view, when the grid's rows run from `top` to `bottom`
/// pixels: positive above the top (back into history), negative below
/// the bottom, 0 between. Proportional to the distance past the edge,
/// between [`MIN_SPEED`] and [`MAX_SPEED`].
pub fn autoscroll_speed(y: f64, top: f64, bottom: f64, cell_height: f64) -> f64 {
    let past = if y < top {
        top - y
    } else if y >= bottom {
        y - bottom + 1.0
    } else {
        return 0.0;
    };
    let speed = (past / cell_height.max(1.0) * SPEED_PER_LINE).clamp(MIN_SPEED, MAX_SPEED);
    if y < top { speed } else { -speed }
}

/// Whole lines to scroll after `dt` seconds at `speed` lines a second
/// (positive: back into history), carrying the part line over in `carry`.
pub fn autoscroll_step(speed: f64, dt: f64, carry: &mut f64) -> isize {
    *carry += speed * dt.clamp(0.0, MAX_STEP);
    let whole = carry.trunc();
    *carry -= whole;
    whole as isize
}

/// A rectangular selection: the columns between its corners' columns, on
/// every line between their lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub anchor: Point,
    pub head: Point,
}

impl Block {
    pub fn at(p: Point) -> Block {
        Block { anchor: p, head: p }
    }

    /// The selection term-core holds while this rectangle is made: the
    /// text from one corner to the other. The rectangle counts only while
    /// term-core still holds exactly this (a new selection, typing or
    /// output clearing it ends it).
    pub fn stream(&self) -> Selection {
        Selection::new(self.anchor, self.head)
    }

    /// The text in the rectangle, a line for each line, trailing blanks
    /// dropped. Leaves `t`'s selection as it was.
    pub fn text(&self, t: &mut Terminal) -> String {
        let saved = t.selection();
        let (top, bottom) = (self.anchor.line.min(self.head.line), self.anchor.line.max(self.head.line));
        let (left, right) = (self.anchor.col.min(self.head.col), self.anchor.col.max(self.head.col));
        let mut lines = Vec::new();
        for line in top..=bottom {
            t.set_selection(Some(Selection::new(Point { line, col: left }, Point { line, col: right })));
            lines.push(t.selection_text().unwrap_or_default());
        }
        t.set_selection(saved);
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use term_core::Term;

    #[test]
    fn clicks_map_to_gestures() {
        use Gesture::*;
        // (clicks, shift, option, gesture)
        for (clicks, shift, option, want) in [
            (1, false, false, Start),
            (2, false, false, Word),
            (3, false, false, Line),
            (4, false, false, Line),
            (1, false, true, Block),
            (2, false, true, Word),
            (3, false, true, Line),
            (1, true, false, Extend),
            (2, true, false, Extend),
            (3, true, false, Extend),
            (1, true, true, Extend),
            (0, false, false, Start),
        ] {
            assert_eq!(
                gesture(clicks, shift, option),
                want,
                "clicks={clicks} shift={shift} option={option}"
            );
        }
    }

    #[test]
    fn which_releases_copy() {
        use Gesture::*;
        for (g, dragged, want) in [
            (Start, false, false),
            (Start, true, true),
            (Block, false, false),
            (Block, true, true),
            (Word, false, true),
            (Line, false, true),
            (Extend, false, true),
        ] {
            assert_eq!(copies_on_release(g, dragged), want, "{g:?} dragged={dragged}");
        }
    }

    /// Rows from 100 to 500 pixels, 20-pixel cells.
    #[test]
    fn autoscroll_speed_grows_with_the_distance() {
        let speed = |y| autoscroll_speed(y, 100.0, 500.0, 20.0);
        for y in [100.0, 300.0, 499.9] {
            assert_eq!(speed(y), 0.0, "{y}");
        }
        // Just past the edge: the slowest speed, up or down.
        assert_eq!(speed(99.5), MIN_SPEED);
        assert_eq!(speed(500.0), -MIN_SPEED);
        // A cell past: 10 lines a second; two cells: twice as fast.
        assert_eq!(speed(80.0), SPEED_PER_LINE);
        assert_eq!(speed(60.0), 2.0 * SPEED_PER_LINE);
        assert_eq!(speed(539.0), -2.0 * SPEED_PER_LINE);
        // Far away (above the window, off the bottom of the screen): capped.
        assert_eq!(speed(-5000.0), MAX_SPEED);
        assert_eq!(speed(9000.0), -MAX_SPEED);
        // Speed only grows with the distance.
        let mut last = 0.0;
        for y in (0..100).rev() {
            let s = speed(y as f64);
            assert!(s >= last, "{y}: {s} < {last}");
            last = s;
        }
    }

    #[test]
    fn autoscroll_steps_carry_part_lines() {
        let mut carry = 0.0;
        // 10 lines a second, 62.5 ms steps: 5/8 of a line each.
        let lines: Vec<isize> = (0..8).map(|_| autoscroll_step(10.0, 0.0625, &mut carry)).collect();
        assert_eq!(lines, [0, 1, 0, 1, 1, 0, 1, 1]);
        assert_eq!(carry, 0.0);
        let mut carry = 0.0;
        assert_eq!(autoscroll_step(-30.0, 0.05, &mut carry), -1);
        assert!((carry + 0.5).abs() < 1e-9, "{carry}");
        assert_eq!(autoscroll_step(-30.0, 0.05, &mut carry), -2);
        // A stalled step makes up for at most 100 ms.
        let mut carry = 0.0;
        assert_eq!(autoscroll_step(100.0, 5.0, &mut carry), 10);
        assert_eq!(autoscroll_step(100.0, -1.0, &mut carry), 0);
    }

    #[test]
    fn block_text_takes_the_columns_on_each_line() {
        let mut t = Term::new(20, 5, 100);
        t.advance(b"abcdefgh\r\n0123456789\r\nxy\r\nlast line");
        let top = t.state.point_at(0, 0).line;
        let p = |view_line: u64, col| Point {
            line: top + view_line,
            col,
        };
        // From (0, 2) to (3, 5), corners given either way round.
        let block = Block {
            anchor: p(3, 5),
            head: p(0, 2),
        };
        let before = Some(Selection::new(p(1, 1), p(1, 1)));
        t.state.set_selection(before);
        assert_eq!(block.text(&mut t.state), "cdef\n2345\n\nst l");
        assert_eq!(t.state.selection(), before, "selection restored");
        assert_eq!(Block::at(p(1, 9)).text(&mut t.state), "9");
        assert_eq!(block.stream(), Selection::new(p(3, 5), p(0, 2)));
    }
}
