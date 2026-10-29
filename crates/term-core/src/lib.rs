//! Terminal state with no platform code: escape-code handling, the screen
//! grid, scrollback and selection.
//!
//! Other crates read the screen through [`RowRef`] and [`CellRef`] and the
//! [`Terminal`] accessors. [`Cell`], [`Row`] and [`flags`] are today's
//! storage, public only for scrollback tests; they will change.

pub mod cell;
pub mod rows;
pub mod scrollback;
pub mod style;
pub mod term;
pub mod unicode;
pub mod word;

pub use cell::{Cell, Color, ColorKind, Row, flags};
pub use rows::{CellRef, Content, RowRef, StyleKey, Width};
pub use style::{Attrs, LinkId, Style, Underline};
pub use term::{
    Cursor, CursorShape, LinkValue, Modes, MouseMode, Notification, Point, Progress, Selection, SelectionKind, Term, Terminal, ViewTop,
};
