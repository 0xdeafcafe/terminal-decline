//! Restarting into a new build, with no AppKit: [`reload`] (a dogfood
//! build watching for a newer one, when it may restart, and the saved
//! window list it restarts with), [`panes`] (each window's tree of
//! splits, which the list saves) and [`saved_history`] (each pane's
//! history across the restart). The app's windows feed them and act on
//! their answers.

pub mod panes;
pub mod reload;
pub mod saved_history;

use term_activity::running;
