//! Terminal state with no platform code: escape-code handling, the screen
//! grid, scrollback and selection.
//!
//! Other crates read the screen through [`RowRef`] and [`CellRef`] and the
//! [`Terminal`] accessors. [`Cell`] and [`Row`] are the storage, public for
//! the scrollback tests; they will change.

pub mod cell;
pub mod clusters;
pub mod image;
mod links;
pub mod rows;
pub mod scrollback;
pub mod search;
pub mod style;
mod style_table;
pub mod term;
pub mod unicode;
pub mod word;

pub use cell::{Cell, Color, ColorKind, Mark, MarkKind, Row};

pub use image::{ImageHost, ImageJob, ImageKey, ImagePool, JobDone, PixelBuffer};
pub use rows::{CellRef, Content, RowRef, StyleKey, Width};
pub use search::{Hit, HitBatch, Query, SearchJob, WordMatch};
pub use style::{Attrs, LinkId, Style, Underline};
pub use style_table::StyleStats;
pub use term::SearchCount;
pub use term::{
    Cursor, CursorShape, DynamicColor, LinkValue, Modes, MouseMode, Notification, Point, Progress, Selection, SelectionKind, Term,
    Terminal, ViewTop,
};
pub use term::{Highlight, HighlightKind, RowFlags, RowKey, ScreenId, SnapCursor, SnapRow, Snapshot};
