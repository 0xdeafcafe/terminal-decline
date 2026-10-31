//! The agent API (docs/design/agent-api.md): `terminal-decline ctl`
//! drives headless terminals for agents and tests through a Unix socket
//! only the user can reach, served by the app or by a headless server
//! started on demand.
//!
//! - [`protocol`]: the requests, the socket's place and its checks;
//! - `client`: the command line;
//! - `server`: the socket, who may connect, the sessions;
//! - `headless`: a session without a view;
//! - `keys`, `screen`, `shot`: keys through the app's encoding, the
//!   screen read back, screenshots on the GPU;
//! - [`window`]: sessions shown in a window of the app (`spawn --window`).

mod client;
mod headless;
mod keys;
pub mod protocol;
mod screen;
mod server;
mod shot;
pub mod window;

use std::time::Duration;

pub use client::{HELP, request};

/// How long the headless server stays with nothing running.
const IDLE: Duration = Duration::from_secs(10);

/// How long a server that leaves when idle stays with nothing running.
fn idle() -> Duration {
    std::env::var(protocol::IDLE_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(IDLE)
}

/// `terminal-decline ctl ARGS…`: its exit status.
pub fn run(args: &[String]) -> i32 {
    client::run(args)
}

/// The config source sessions are made from: the user's config file.
fn source() -> crate::config::Source {
    let paths = crate::Paths::current();
    crate::config::Source {
        layers: vec![paths.config_file()],
        themes_dir: paths.themes_dir(),
        command: None,
        edit: 0,
    }
}

/// `terminal-decline ctl serve`: the headless server. No `NSApplication`,
/// so no Dock icon, menu bar or windows; it exits when idle.
fn serve_headless() -> i32 {
    let (config, _) = source().load();
    if !config.agent_api {
        eprintln!("terminal-decline ctl: the agent API is off (agent_api = false)");
        return protocol::exit::FAILED;
    }
    crate::images::apply_config(config.images, config.image_memory, config.image_file_transfer);
    let dir = protocol::dir();
    let (listener, lock) = match server::bind(&dir) {
        Ok(Some(l)) => l,
        // Another server serves.
        Ok(None) => return protocol::exit::OK,
        Err(e) => {
            eprintln!("terminal-decline ctl serve: {}: {e}", dir.display());
            return protocol::exit::FAILED;
        }
    };
    let server = server::Server::new(server::Kind::Headless, &dir, source(), lock);
    server.exit_when_idle(idle());
    server.serve(listener);
    protocol::exit::FAILED
}

/// Serve from the app (role App with `agent_api` on, or an app `ctl`
/// started to host windows), on a thread of its own. Nothing happens
/// when a headless server already serves: its clients keep it until it
/// exits. A hosting app leaves once it is idle, as the headless server
/// does.
pub fn serve_in_app() {
    let dir = protocol::dir();
    match server::bind(&dir) {
        Ok(Some((listener, lock))) => {
            let server = server::Server::new(server::Kind::App, &dir, source(), lock);
            if window::hosted() {
                server.exit_when_idle(idle());
            }
            let _ = std::thread::Builder::new()
                .name("ctl-accept".into())
                .spawn(move || server.serve(listener));
        }
        Ok(None) => {}
        Err(e) => eprintln!("terminal-decline: agent API: {}: {e}", dir.display()),
    }
}
