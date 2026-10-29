//! The `terminal-decline` binary: reads its arguments and starts the app;
//! everything else is in the library.

use std::path::PathBuf;

use terminal_decline::{Paths, USAGE, app, config, crash, pin};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = Paths::from_env();
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    crash::install(crash::logs_dir(&home.unwrap_or_else(std::env::temp_dir)), true);
    // First run after the rename: ~/.config/decline →
    // ~/.config/terminal-decline, before anything reads either.
    let moved = match paths.move_old_config() {
        Ok(done) => done.notice(&paths),
        Err(e) => Some(format!(
            "config: couldn't move the old config to {}: {e}",
            paths.config_dir.display()
        )),
    };
    if matches!(args.first().map(String::as_str), Some("pin" | "import"))
        && let Some(m) = &moved
    {
        eprintln!("terminal-decline: {m}");
    }
    if args.first().map(String::as_str) == Some("pin") {
        std::process::exit(match pin::run(&args[1..], &paths, &mut pin::system_runner) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("terminal-decline pin: {e}");
                1
            }
        });
    }

    if args.first().map(String::as_str) == Some("import") {
        std::process::exit(match terminal_decline::import::run(&args[1..], &paths) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("terminal-decline import: {e}");
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
    let mut restore = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            "-V" | "--version" => {
                println!("terminal-decline {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--config" => {
                i += 1;
                match args.get(i) {
                    Some(p) => layers.push(PathBuf::from(p)),
                    None => die("--config needs a file"),
                }
            }
            // A dogfood build restarting into a newer one.
            "--restore" => {
                i += 1;
                if let Some(p) = args.get(i) {
                    restore = terminal_decline::reload::take(std::path::Path::new(p));
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

    let from = config::Source {
        layers,
        themes_dir: paths.themes_dir(),
        command,
    };
    let (config, mut errors) = from.load();
    errors.splice(0..0, moved);
    for e in &errors {
        eprintln!("terminal-decline: {e}");
    }
    let name = pin::own_app_name(&paths).unwrap_or_else(|| "Terminal Decline".into());
    // Shown in each window too: an app started from the Dock has no stderr.
    app::run(config, errors, &name, restore, from);
}

fn die(msg: &str) -> ! {
    eprintln!("terminal-decline: {msg}");
    std::process::exit(2);
}
