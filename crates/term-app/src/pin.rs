//! Dock pins: make any program its own macOS app (docs/design/pins.md).
//!
//! The pins themselves are term-pins; this module keeps the names
//! `crate::pin::…` and passes in what only the app has: its [`Paths`], its
//! usage text and the Secure Keyboard Entry carry-over, which reads Core
//! Foundation preferences ([`crate::secure_input`]).

use std::path::PathBuf;

use crate::paths::Paths;

pub use term_pins::{
    Confirm, Created, ID_PREFIX, OLD_ID_PREFIX, OWN_NAMES, Options, REGISTER, Request, Row, Runner, bundle, check_name, dock_tile,
    file_url, info_plist, installed_app, is_pin_bundle, list_json, list_text, parse_args, pin_table, record, slug, status, system_runner,
};

/// Hand term-pins what it needs from the app (once).
fn hooks() {
    term_pins::set_hooks(term_pins::Hooks {
        usage: crate::USAGE,
        carry_over: crate::secure_input::carry_over,
    });
}

pub fn own_pin_config(paths: &Paths) -> Option<PathBuf> {
    term_pins::own_pin_config(&paths.pin_paths())
}

pub fn own_app_name(paths: &Paths) -> Option<String> {
    term_pins::own_app_name(&paths.pin_paths())
}

/// `terminal-decline pin …` (the arguments after `pin`).
pub fn run(args: &[String], paths: &Paths, run: &mut Runner) -> Result<(), String> {
    hooks();
    term_pins::run(args, &paths.pin_paths(), run)
}

pub fn remove(name: &str, paths: &Paths) -> Result<PathBuf, String> {
    term_pins::remove(name, &paths.pin_paths())
}

pub fn create(o: Options, paths: &Paths, run: &mut Runner) -> Result<Created, String> {
    term_pins::create(o, &paths.pin_paths(), run)
}

pub fn pins(paths: &Paths) -> Vec<(String, PathBuf)> {
    term_pins::pins(&paths.pin_paths())
}

pub fn list(paths: &Paths, lookup: &dyn Fn(&str) -> Vec<PathBuf>) -> Vec<Row> {
    term_pins::list(&paths.pin_paths(), lookup)
}

pub fn repair(
    name: Option<&str>,
    paths: &Paths,
    run: &mut Runner,
    confirm: &mut Confirm,
    lookup: &dyn Fn(&str) -> Vec<PathBuf>,
) -> Result<(), String> {
    hooks();
    term_pins::repair(name, &paths.pin_paths(), run, confirm, lookup)
}

pub fn update(paths: &Paths, run: &mut Runner, confirm: &mut Confirm) -> Result<(), String> {
    hooks();
    term_pins::update(&paths.pin_paths(), run, confirm)
}

pub fn edit(name: &str, paths: &Paths, editor: Option<&str>, run: &mut Runner) -> Result<Vec<String>, String> {
    term_pins::edit(name, &paths.pin_paths(), editor, run)
}

pub fn migration_notice(paths: &Paths) -> Option<String> {
    term_pins::migration_notice(&paths.pin_paths())
}
