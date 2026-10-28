//! The `decline` binary: reads its arguments and starts the app; everything
//! else is in the library.

use std::path::PathBuf;

use decline::{Paths, USAGE, app, config, crash, pin};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = Paths::from_env();
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    crash::install(crash::logs_dir(&home.unwrap_or_else(std::env::temp_dir)), true);
    if args.first().map(String::as_str) == Some("pin") {
        std::process::exit(match pin::run(&args[1..], &paths, &mut pin::system_runner) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("decline pin: {e}");
                1
            }
        });
    }

    let mut layers: Vec<PathBuf> = vec![paths.config_file()];
    // Running as a pin: its settings, on top of the user's config.
    if let Some(p) = pin::own_pin_config(&paths) {
        layers.push(p);
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
    for e in &errors {
        eprintln!("decline: {e}");
    }
    if let Some(cmd) = command.filter(|c| !c.is_empty()) {
        config.command = cmd;
    }
    let name = pin::own_app_name(&paths).unwrap_or_else(|| "decline".into());
    // Shown in each window too: an app started from the Dock has no stderr.
    app::run(config, errors, &name);
}

fn die(msg: &str) -> ! {
    eprintln!("decline: {msg}");
    std::process::exit(2);
}
