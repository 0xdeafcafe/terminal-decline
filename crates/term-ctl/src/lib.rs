//! The agent API (docs/design/agent-api.md) without its sessions, with no
//! AppKit: [`protocol`] (the requests, the socket's place and its
//! checks), [`client`] (`terminal-decline ctl`, the command line),
//! [`idle`] (how long a session may go unasked about), [`screen`] (the
//! screen read back) and [`keys`] (keys and text through the app's
//! encoding). The server and its sessions, which run programs, are the
//! app's `ctl` module; `ctl serve` reaches them through the function
//! [`client::run`] is given.

pub mod client;
pub mod idle;
pub mod keys;
pub mod protocol;
pub mod screen;
#[cfg(test)]
mod testutil;
