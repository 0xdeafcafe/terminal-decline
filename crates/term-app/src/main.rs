mod app;
mod config;
mod keys;
mod pin;
mod view;

use std::path::PathBuf;

pub const USAGE: &str = "\
usage: decline [--config FILE] [-e PROGRAM [ARGS...]]
       decline pin NAME [--icon FILE] [--config FILE] [--cwd DIR] [--dir DIR] [--dock] -- PROGRAM [ARGS...]
       decline pin --list | --remove NAME | --update

Runs a terminal window. With -e, runs PROGRAM instead of your login shell.
`decline pin` makes PROGRAM its own macOS app, with its own Dock icon.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("pin") {
        std::process::exit(match pin::run(&args[1..]) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("decline pin: {e}");
                1
            }
        });
    }

    let mut layers: Vec<PathBuf> = Vec::new();
    // Running as a pin: its config sits in the bundle's Resources.
    let pin = pin::own_pin_config();
    if let Some(p) = &pin {
        layers.push(p.clone());
    }
    let mut command = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            "-V" | "--version" => {
                println!("decline {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--config" => {
                i += 1;
                match args.get(i) {
                    Some(p) => layers.push(PathBuf::from(p)),
                    None => die("--config needs a file"),
                }
            }
            "-e" | "--command" => {
                command = Some(args[i + 1..].to_vec());
                break;
            }
            // Finder passes this when launching an app by double-click.
            a if a.starts_with("-psn_") => {}
            a => die(&format!("unknown argument {a:?}\n{USAGE}")),
        }
        i += 1;
    }

    let refs: Vec<&std::path::Path> = layers.iter().map(|p| p.as_path()).collect();
    let (mut config, errors) = config::load(&refs);
    for e in errors {
        eprintln!("decline: {e}");
    }
    if let Some(cmd) = command.filter(|c| !c.is_empty()) {
        config.command = cmd;
    }
    let name = pin::own_app_name().unwrap_or_else(|| "decline".into());
    app::run(config, &name);
}

fn die(msg: &str) -> ! {
    eprintln!("decline: {msg}");
    std::process::exit(2);
}
