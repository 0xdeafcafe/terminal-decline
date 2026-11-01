//! A command's flags and subcommands read from its `--help` output
//! (docs/design/completion.md §8), for programs without a man page:
//! GNU-style lines (`-x, --long=ARG   what it does`), and the commands
//! listed under a heading with "commands" in it (`Commands:`, gh's `CORE
//! COMMANDS`, npm's comma-separated list). Pure: the output comes in as
//! text; [`super::sources`] runs the program.

use super::man::{Flag, Page, flag_names, sentence};

/// What `--help` says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Help {
    pub page: Page,
    /// Names and what each does, in the output's order, one per name.
    pub subcommands: Vec<(String, String)>,
}

/// Read `--help` output.
pub fn read(text: &str) -> Help {
    let mut help = Help::default();
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
    let mut in_commands = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let indent = indent_of(line);
        let body = line.trim_start();
        if body.is_empty() {
            continue;
        }
        if help.page.about.is_empty() && indent == 0 && !body.ends_with(':') && !body.starts_with('-') {
            help.page.about = sentence(body);
        }
        // A heading: `Options:`, `Commands:`, `CORE COMMANDS`.
        if indent == 0 && (body.ends_with(':') || is_caps(body)) {
            in_commands = body.to_lowercase().contains("command");
            continue;
        }
        if body.starts_with('-') {
            let (tag, mut about) = split_columns(body);
            let tag = tag.trim_end_matches(':');
            if about.is_empty() {
                // The description on the lines after, indented further.
                while let Some(next) = lines.get(i) {
                    let n_indent = indent_of(next);
                    let n = next.trim();
                    if n.is_empty() || n.starts_with('-') || n_indent <= indent {
                        break;
                    }
                    if !about.is_empty() {
                        about.push(' ');
                    }
                    about.push_str(n);
                    i += 1;
                    if about.ends_with('.') {
                        break;
                    }
                }
            }
            let about = sentence(&about);
            for name in flag_names(tag) {
                if !help.page.flags.iter().any(|f| f.name == name) {
                    help.page.flags.push(Flag {
                        name,
                        about: about.clone(),
                    });
                }
            }
            continue;
        }
        if in_commands && indent > 0 {
            let (left, about) = split_columns(body);
            if about.is_empty() && left.contains(", ") {
                // npm: `access, adduser, audit,`.
                for name in left.split(',').map(str::trim) {
                    add_command(&mut help, name, "");
                }
                continue;
            }
            // `build, b`, `auth:`.
            let mut names = left.split(',').map(|n| n.trim().trim_end_matches(':'));
            if let Some(first) = names.next() {
                add_command(&mut help, first, &sentence(&about));
            }
        }
    }
    help
}

fn add_command(help: &mut Help, name: &str, about: &str) {
    let ok = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'));
    if ok && !help.subcommands.iter().any(|(n, _)| n == name) {
        help.subcommands.push((name.to_string(), about.to_string()));
    }
}

/// How far a line is indented, a tab as eight spaces.
fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 8 } else { 1 })
        .sum()
}

/// A heading in capitals, gh's way: `CORE COMMANDS`.
fn is_caps(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_uppercase()) && !s.chars().any(|c| c.is_ascii_lowercase())
}

/// A line's left column and its description, split at the first gap of
/// two spaces or more (or a tab).
fn split_columns(body: &str) -> (&str, String) {
    let gap = body.find("  ").into_iter().chain(body.find('\t')).min();
    match gap {
        Some(at) => (&body[..at], body[at..].trim().to_string()),
        None => (body, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(h: &Help) -> Vec<&str> {
        h.page.flags.iter().map(|f| f.name.as_str()).collect()
    }

    fn commands(h: &Help) -> Vec<&str> {
        h.subcommands.iter().map(|(n, _)| n.as_str()).collect()
    }

    #[test]
    fn clap_style() {
        let h = read(
            "Rust's package manager

Usage: cargo [OPTIONS] [COMMAND]

Options:
  -V, --version                  Print version info and exit
      --color <WHEN>             Coloring [possible values: auto, always, never]
  -Z <FLAG>                      Unstable (nightly-only) flags to Cargo, see 'cargo -Z help' for
                                 details

Commands:
    build, b    Compile the current package
    new         Create a new cargo package
",
        );
        assert_eq!(h.page.about, "Rust's package manager");
        assert_eq!(names(&h), ["-V", "--version", "--color", "-Z"]);
        assert_eq!(h.page.flags[2].about, "Coloring [possible values: auto, always, never]");
        assert_eq!(commands(&h), ["build", "new"]);
        assert_eq!(h.subcommands[0].1, "Compile the current package");
    }

    #[test]
    fn descriptions_on_the_next_line() {
        // kubectl's.
        let h = read(
            "Options:
    -A, --all-namespaces=false:
\tIf present, list the requested object(s) across all namespaces.

    --chunk-size=500:
\tReturn large lists in chunks rather than all at once.
",
        );
        assert_eq!(names(&h), ["-A", "--all-namespaces", "--chunk-size"]);
        assert_eq!(h.page.flags[0].about, "If present, list the requested object(s) across…");
    }

    #[test]
    fn command_lists() {
        // gh's and npm's.
        let h = read(
            "Work seamlessly with GitHub from the command line.

CORE COMMANDS
  auth:          Authenticate gh and git with GitHub
  pr:            Manage pull requests

USAGE
  gh <command> <subcommand> [flags]

All commands:

    access, adduser, audit,
    cache, ci
",
        );
        assert_eq!(commands(&h), ["auth", "pr", "access", "adduser", "audit", "cache", "ci"]);
        assert_eq!(h.subcommands[1].1, "Manage pull requests");
        assert!(h.page.flags.is_empty());
    }
}
