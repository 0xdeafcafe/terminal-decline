//! Terminal state with no platform code: escape-code handling, the screen
//! grid, scrollback and selection.

pub mod cell;
pub mod scrollback;
pub mod term;

pub use cell::{Cell, Color, ColorKind, Row, flags};
pub use term::{CursorShape, Modes, MouseMode, Point, Selection, Term, Terminal};
