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
