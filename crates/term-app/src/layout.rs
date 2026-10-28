//! Window geometry: padding, how many cells fit, the window size for a grid,
//! and which cell is under a point. Sizes in points are `(width, height)`;
//! pixels are points × `scale`.

/// Padding in pixels. The top includes the title bar, which the content
/// extends under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Padding {
    /// Left and right.
    pub x: u32,
    pub top: u32,
    pub bottom: u32,
}

/// Cell size in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

impl From<term_font::Metrics> for CellSize {
    fn from(m: term_font::Metrics) -> CellSize {
        CellSize {
            width: m.cell_width,
            height: m.cell_height,
        }
    }
}

/// GPU positions are 16-bit; no display is near 32k pixels, but cap anyway.
pub const MAX_PIXELS: u32 = 32_000;
pub const MIN_COLS: usize = 2;
pub const MAX_COLS: usize = 4000;
pub const MIN_LINES: usize = 1;
pub const MAX_LINES: usize = 2000;

/// `padding` and `titlebar` are in points.
pub fn padding(padding: f64, titlebar: f64, scale: f64) -> Padding {
    let p = (padding * scale).round() as u32;
    Padding {
        x: p,
        top: ((padding + titlebar) * scale).round() as u32,
        bottom: p,
    }
}

/// Columns and lines that fit in a view of `size` points.
pub fn grid_size(size: (f64, f64), scale: f64, cell: CellSize, pad: Padding) -> (usize, usize) {
    let w = (size.0 * scale) as u32;
    let h = (size.1 * scale) as u32;
    let (w, h) = (w.min(MAX_PIXELS), h.min(MAX_PIXELS));
    let cols = ((w.saturating_sub(2 * pad.x) / cell.width) as usize).clamp(MIN_COLS, MAX_COLS);
    let lines = ((h.saturating_sub(pad.top + pad.bottom) / cell.height) as usize).clamp(MIN_LINES, MAX_LINES);
    (cols, lines)
}

/// View size in points for a grid of `cols` × `lines`.
pub fn content_size(cols: usize, lines: usize, scale: f64, cell: CellSize, pad: Padding) -> (f64, f64) {
    let w = cols as u32 * cell.width + 2 * pad.x;
    let h = lines as u32 * cell.height + pad.top + pad.bottom;
    (w as f64 / scale, h as f64 / scale)
}

/// The visible (line, column) under `point` (points, from the top left),
/// clamped to a `cols` × `lines` grid; a point in the padding gives the
/// nearest cell.
pub fn cell_at(point: (f64, f64), scale: f64, cell: CellSize, pad: Padding, cols: usize, lines: usize) -> (usize, usize) {
    let x = (point.0 * scale - pad.x as f64).max(0.0);
    let y = (point.1 * scale - pad.top as f64).max(0.0);
    let col = ((x / cell.width as f64) as usize).min(cols.saturating_sub(1));
    let line = ((y / cell.height as f64) as usize).min(lines.saturating_sub(1));
    (line, col)
}

/// Pixel origin of the cell at (line, col), for placing the input method's
/// candidate window.
pub fn cell_origin(line: usize, col: usize, cell: CellSize, pad: Padding) -> (u32, u32) {
    (pad.x + col as u32 * cell.width, pad.top + line as u32 * cell.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Menlo 13 pt cells at each scale, roughly.
    fn cell(scale: f64) -> CellSize {
        CellSize {
            width: (8.0 * scale) as u32,
            height: (16.0 * scale) as u32,
        }
    }

    #[test]
    fn padding_includes_the_titlebar_at_the_top() {
        for (pad, titlebar, scale, want) in [
            (6.0, 0.0, 1.0, (6, 6, 6)),
            (6.0, 28.0, 1.0, (6, 34, 6)),
            (6.0, 28.0, 2.0, (12, 68, 12)),
            (6.0, 28.0, 3.0, (18, 102, 18)),
            (0.5, 0.0, 3.0, (2, 2, 2)),
            (0.0, 0.0, 2.0, (0, 0, 0)),
        ] {
            let p = padding(pad, titlebar, scale);
            assert_eq!((p.x, p.top, p.bottom), want, "pad={pad} titlebar={titlebar} scale={scale}");
        }
    }

    /// The window opened for a configured grid shows exactly that grid,
    /// including cell sizes that aren't a multiple of the scale (Menlo 13
    /// at 3× is 23×46 px, say).
    #[test]
    fn content_size_and_grid_size_round_trip() {
        let mut checked = 0;
        for scale in [1.0, 2.0, 3.0] {
            for c in [cell(scale), CellSize { width: 23, height: 46 }, CellSize { width: 7, height: 17 }] {
                for (pad_pt, titlebar) in [(6.0, 0.0), (6.0, 28.0), (2.5, 27.5)] {
                    for (cols, lines) in [(100, 30), (80, 24), (2, 1), (333, 77), (7, 3)] {
                        let pad = padding(pad_pt, titlebar, scale);
                        let size = content_size(cols, lines, scale, c, pad);
                        assert_eq!(
                            grid_size(size, scale, c, pad),
                            (cols, lines),
                            "{cols}x{lines} scale={scale} cell={c:?} pad={pad:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 3 * 3 * 3 * 5);
    }

    #[test]
    fn grid_size_rounds_down_and_clamps() {
        let c = CellSize { width: 10, height: 20 };
        let pad = Padding { x: 5, top: 30, bottom: 5 };
        for (size, want) in [
            // 1000 - 10 = 990 px → 99 cols; 800 - 35 = 765 px → 38 lines.
            ((1000.0, 800.0), (99, 38)),
            ((1009.0, 814.0), (99, 38)),
            ((1010.0, 815.0), (100, 39)),
            // Tiny windows still get a 2×1 grid.
            ((0.0, 0.0), (2, 1)),
            ((5.0, 5.0), (2, 1)),
            ((24.0, 50.0), (2, 1)),
            ((45.0, 75.0), (3, 2)),
            // Huge windows are capped.
            ((1e9, 1e9), (3199, 1598)),
        ] {
            assert_eq!(grid_size(size, 1.0, c, pad), want, "{size:?}");
        }
        let tiny = CellSize { width: 1, height: 1 };
        let none = Padding { x: 0, top: 0, bottom: 0 };
        assert_eq!(grid_size((1e9, 1e9), 1.0, tiny, none), (MAX_COLS, MAX_LINES));
    }

    #[test]
    fn cell_under_a_point() {
        let c = CellSize { width: 16, height: 32 };
        let pad = padding(6.0, 28.0, 2.0); // 12 px sides, 68 px top
        let at = |x: f64, y: f64| cell_at((x, y), 2.0, c, pad, 80, 24);
        for (point, want) in [
            ((6.0, 34.0), (0, 0)),
            ((13.9, 49.9), (0, 0)),
            ((14.0, 50.0), (1, 1)),
            // In the padding: the nearest cell.
            ((0.0, 0.0), (0, 0)),
            ((-50.0, 10.0), (0, 0)),
            ((10_000.0, 34.0), (0, 79)),
            ((6.0, 10_000.0), (23, 0)),
            ((10_000.0, 10_000.0), (23, 79)),
        ] {
            assert_eq!(at(point.0, point.1), want, "{point:?}");
        }
        assert_eq!(cell_origin(0, 0, c, pad), (12, 68));
        assert_eq!(cell_origin(2, 3, c, pad), (12 + 48, 68 + 64));
    }
}
