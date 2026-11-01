//! What the completion list offers for the word under the cursor
//! (docs/design/completion.md §6, §9): commands where a command goes,
//! a command's subcommands and flags from its man pages, files and
//! folders elsewhere, matched by prefix (case-sensitive ones first, then
//! the rest; flags in their page's order). Pure: the sources' listings come in already
//! read; nothing here touches the disk.

use std::path::{Path, PathBuf};

use term_core::ShellEnv;

use super::lex::At;
use super::man::Page;
use super::sources::{Commands, Folder, Subcommands};

/// At most this many choices are kept.
pub const MAX_CHOICES: usize = 200;

/// What a choice is, for its label on the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Command,
    Alias,
    Function,
    Builtin,
    Keyword,
    Folder,
    File,
    Executable,
    Subcommand,
    Flag,
    /// One of zsh's own matches.
    Match,
}

impl What {
    /// The label shown beside the choice.
    pub fn label(self) -> &'static str {
        match self {
            What::Command => "command",
            What::Alias => "alias",
            What::Function => "function",
            What::Builtin => "builtin",
            What::Keyword => "keyword",
            What::Folder => "folder",
            What::File => "file",
            What::Executable => "executable",
            What::Subcommand => "subcommand",
            What::Flag => "flag",
            What::Match => "",
        }
    }
}

/// One choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The name offered: the word's last part (after its last `/` for a
    /// file), with a `/` after a folder.
    pub name: String,
    pub what: What,
    /// What it does, from its man page; empty when unknown.
    pub about: String,
}

/// The choices for a word, and how to put one in its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub choices: Vec<Choice>,
    /// The part of the word being matched (after its last `/` for a file).
    pub prefix: String,
    /// What comes before the matched part in the word, as typed and kept
    /// (`src/`, `~/Doc/`): a taken choice goes after it.
    pub keep: String,
}

impl Offer {
    /// What taking choice `i` makes of the word: `keep`, the name, and a
    /// space after anything but a folder.
    pub fn word(&self, i: usize) -> Option<String> {
        let c = self.choices.get(i)?;
        let space = if c.what == What::Folder { "" } else { " " };
        Some(format!("{}{}{space}", self.keep, c.name))
    }

    /// The rest of the first choice after what is typed, for showing grey
    /// after the cursor: only when it starts with the typed part exactly.
    pub fn ghost(&self) -> Option<&str> {
        let c = self.choices.first()?;
        c.name.strip_prefix(&self.prefix).filter(|rest| !rest.is_empty())
    }
}

/// What the sources have read for an offer: `None` while still reading.
pub struct Listings<'a> {
    pub env: Option<&'a ShellEnv>,
    pub commands: Option<&'a Commands>,
    /// The folder a file prefix points into, read.
    pub folder: Option<&'a Folder>,
    /// The command's man page, for a flag.
    pub flags: Option<&'a Page>,
    /// The command's subcommands, for its first argument.
    pub subcommands: Option<&'a Subcommands>,
}

/// The folder (absolute) the word `prefix` points into, and the part
/// typed before its last `/`; `None` without one to read.
pub fn folder_of(prefix: &str, cwd: Option<&str>, home: Option<&Path>) -> Option<(PathBuf, String)> {
    let keep = match prefix.rfind('/') {
        Some(i) => &prefix[..=i],
        None => "",
    };
    let dir = if let Some(rest) = keep.strip_prefix("~/") {
        home?.join(rest)
    } else if keep == "~" {
        home?.to_path_buf()
    } else if keep.starts_with('/') {
        PathBuf::from(keep)
    } else if keep.contains('$') {
        // An expansion: its value isn't known here.
        return None;
    } else {
        Path::new(cwd?).join(keep)
    };
    Some((dir, keep.to_string()))
}

/// What to offer for what the cursor is on, from `listings`; `None` when
/// there is nothing to offer here (yet).
pub fn offer(at: &At, listings: &Listings) -> Option<Offer> {
    match at {
        At::Command { prefix } if !prefix.contains('/') => {
            let env = listings.env?;
            let commands = listings.commands?;
            let mut all: Vec<Choice> = Vec::new();
            let mut add = |names: &mut dyn Iterator<Item = &String>, what| {
                all.extend(names.map(|n| Choice {
                    name: n.clone(),
                    what,
                    about: String::new(),
                }));
            };
            add(&mut env.aliases.iter(), What::Alias);
            add(&mut env.functions.iter(), What::Function);
            add(&mut env.builtins.iter(), What::Builtin);
            add(&mut env.reserved_words.iter(), What::Keyword);
            add(&mut commands.names.iter(), What::Command);
            Some(Offer {
                choices: matching(all, prefix),
                prefix: prefix.clone(),
                keep: String::new(),
            })
        }
        At::Flag { prefix, .. } => {
            let page = listings.flags?;
            let choices = page
                .flags
                .iter()
                .filter(|f| f.name.starts_with(prefix.as_str()))
                .take(MAX_CHOICES)
                .map(|f| Choice {
                    name: f.name.clone(),
                    what: What::Flag,
                    about: f.about.clone(),
                })
                .collect();
            Some(Offer {
                choices,
                prefix: prefix.clone(),
                keep: String::new(),
            })
        }
        At::Argument { position: 0, prefix, .. }
            if listings
                .subcommands
                .is_some_and(|s| s.list.iter().any(|(n, _)| n.starts_with(prefix.as_str()))) =>
        {
            let all = listings
                .subcommands
                .into_iter()
                .flat_map(|s| &s.list)
                .map(|(name, about)| Choice {
                    name: name.clone(),
                    what: What::Subcommand,
                    about: about.clone(),
                })
                .collect();
            Some(Offer {
                choices: matching(all, prefix)
                    .into_iter()
                    .filter(|c| c.name.starts_with(prefix.as_str()))
                    .collect(),
                prefix: prefix.clone(),
                keep: String::new(),
            })
        }
        At::Command { prefix } | At::Argument { prefix, .. } | At::Path { prefix, .. } => {
            let folder = listings.folder?;
            let (keep, name) = match prefix.rfind('/') {
                Some(i) => (&prefix[..=i], &prefix[i + 1..]),
                None => ("", prefix.as_str()),
            };
            let hidden = name.starts_with('.');
            let all = folder
                .entries
                .iter()
                .filter(|e| hidden || !e.name.starts_with('.'))
                .map(|e| match () {
                    _ if e.dir => Choice {
                        name: format!("{}/", e.name),
                        what: What::Folder,
                        about: String::new(),
                    },
                    _ if e.executable => Choice {
                        name: e.name.clone(),
                        what: What::Executable,
                        about: String::new(),
                    },
                    _ => Choice {
                        name: e.name.clone(),
                        what: What::File,
                        about: String::new(),
                    },
                })
                .collect();
            let mut choices = matching(all, name);
            // Folders first, each group in the order matched.
            choices.sort_by_key(|c| c.what != What::Folder);
            Some(Offer {
                choices,
                prefix: name.to_string(),
                keep: keep.to_string(),
            })
        }
        _ => None,
    }
}

/// The choices starting with `prefix`: those matching its case first,
/// then the rest, each A–Z, one per name, at most [`MAX_CHOICES`]. A
/// choice that is exactly the prefix comes first.
fn matching(all: Vec<Choice>, prefix: &str) -> Vec<Choice> {
    let lower = prefix.to_lowercase();
    let mut exact: Vec<Choice> = Vec::new();
    let mut folded: Vec<Choice> = Vec::new();
    for c in all {
        if c.name.starts_with(prefix) {
            exact.push(c);
        } else if c.name.to_lowercase().starts_with(&lower) {
            folded.push(c);
        }
    }
    let order = |v: &mut Vec<Choice>| {
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v.dedup_by(|a, b| a.name == b.name);
    };
    order(&mut exact);
    order(&mut folded);
    if let Some(i) = exact.iter().position(|c| c.name == prefix) {
        let c = exact.remove(i);
        exact.insert(0, c);
    }
    exact.extend(folded.into_iter().filter(|c| !c.name.starts_with(prefix)));
    exact.truncate(MAX_CHOICES);
    exact
}

#[cfg(test)]
mod tests {
    use super::super::sources::Entry;
    use super::*;

    fn folder(entries: &[(&str, bool, bool)]) -> Folder {
        let mut entries: Vec<Entry> = entries
            .iter()
            .map(|&(name, dir, executable)| Entry {
                name: name.into(),
                dir,
                executable,
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Folder { entries }
    }

    fn names(o: &Offer) -> Vec<&str> {
        o.choices.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn commands_where_a_command_goes() {
        let env = ShellEnv {
            aliases: vec!["gst".into(), "ll".into()],
            builtins: vec!["cd".into(), "echo".into()],
            functions: vec!["gitfn".into()],
            reserved_words: vec!["if".into()],
            ..ShellEnv::default()
        };
        let commands = Commands::from_names(vec!["git".into(), "grep".into(), "Gzip".into(), "gst".into()]);
        let listings = Listings {
            env: Some(&env),
            commands: Some(&commands),
            folder: None,
            flags: None,
            subcommands: None,
        };
        let o = offer(&At::Command { prefix: "g".into() }, &listings).unwrap();
        assert_eq!(names(&o), ["git", "gitfn", "grep", "gst", "Gzip"]);
        assert_eq!(o.choices[0].what, What::Command);
        // One per name: the alias, listed first, wins over the program.
        assert_eq!(o.choices[3].what, What::Alias);
        assert_eq!(o.ghost(), Some("it"));
        assert_eq!(o.word(0).as_deref(), Some("git "));
        // Exactly what is typed comes first.
        let o = offer(&At::Command { prefix: "gst".into() }, &listings).unwrap();
        assert_eq!(names(&o), ["gst"]);
        assert_eq!(o.ghost(), None);
        // Not read yet: nothing.
        let none = Listings {
            env: Some(&env),
            commands: None,
            folder: None,
            flags: None,
            subcommands: None,
        };
        assert_eq!(offer(&At::Command { prefix: "g".into() }, &none), None);
    }

    #[test]
    fn files_and_folders_elsewhere() {
        let f = folder(&[
            ("main.rs", false, false),
            ("macros", true, false),
            ("Makefile", false, false),
            (".hidden", false, false),
            ("run.sh", false, true),
        ]);
        let listings = Listings {
            env: None,
            commands: None,
            folder: Some(&f),
            flags: None,
            subcommands: None,
        };
        let at = At::Argument {
            command: "cat".into(),
            position: 0,
            prefix: "src/ma".into(),
            quote: None,
        };
        let o = offer(&at, &listings).unwrap();
        assert_eq!(names(&o), ["macros/", "main.rs", "Makefile"]);
        assert_eq!(o.choices[0].what, What::Folder);
        assert_eq!((o.prefix.as_str(), o.keep.as_str()), ("ma", "src/"));
        assert_eq!(o.word(0).as_deref(), Some("src/macros/"));
        assert_eq!(o.word(1).as_deref(), Some("src/main.rs "));
        // Hidden files only for a dot.
        let at = At::Path {
            prefix: "".into(),
            quote: None,
        };
        assert!(!names(&offer(&at, &listings).unwrap()).contains(&".hidden"));
        let at = At::Path {
            prefix: ".".into(),
            quote: None,
        };
        assert_eq!(names(&offer(&at, &listings).unwrap()), [".hidden"]);
        // A command typed as a path: files there.
        let at = At::Command { prefix: "./r".into() };
        let o = offer(&at, &listings).unwrap();
        assert_eq!(names(&o), ["run.sh"]);
        assert_eq!(o.choices[0].what, What::Executable);
    }

    #[test]
    fn which_folder_a_prefix_points_into() {
        let home = Path::new("/Users/me");
        let cwd = Some("/src");
        assert_eq!(folder_of("ma", cwd, Some(home)), Some(("/src".into(), "".into())));
        assert_eq!(folder_of("a/b/c", cwd, Some(home)), Some(("/src/a/b/".into(), "a/b/".into())));
        assert_eq!(folder_of("~/Doc", cwd, Some(home)), Some(("/Users/me/".into(), "~/".into())));
        assert_eq!(folder_of("/etc/pa", cwd, Some(home)), Some(("/etc/".into(), "/etc/".into())));
        assert_eq!(folder_of("$HOME/x", cwd, Some(home)), None);
        assert_eq!(folder_of("x", None, Some(home)), None);
    }

    #[test]
    fn nothing_for_variables_or_flags_not_read() {
        let listings = Listings {
            env: None,
            commands: None,
            folder: None,
            flags: None,
            subcommands: None,
        };
        for at in [
            At::Flag {
                command: "ls".into(),
                subcommand: None,
                prefix: "-".into(),
            },
            At::Variable { prefix: "HO".into() },
            At::Nothing,
        ] {
            assert_eq!(offer(&at, &listings), None, "{at:?}");
        }
    }

    #[test]
    fn flags_and_subcommands_from_man_pages() {
        use super::super::man::Flag;
        let flag = |name: &str, about: &str| Flag {
            name: name.into(),
            about: about.into(),
        };
        let page = Page {
            about: String::new(),
            flags: vec![
                flag("--all", "Fetch all remotes"),
                flag("-a", "Append"),
                flag("--atomic", "Atomically"),
            ],
        };
        let subs = Subcommands {
            list: vec![("fetch".into(), "Download objects".into()), ("fetch-pack".into(), String::new())],
        };
        let f = folder(&[("feature.txt", false, false)]);
        let listings = Listings {
            env: None,
            commands: None,
            folder: Some(&f),
            flags: Some(&page),
            subcommands: Some(&subs),
        };
        let at = At::Flag {
            command: "git".into(),
            subcommand: Some("fetch".into()),
            prefix: "--a".into(),
        };
        let o = offer(&at, &listings).unwrap();
        // In the page's order.
        assert_eq!(names(&o), ["--all", "--atomic"]);
        assert_eq!((o.choices[0].what, o.choices[0].about.as_str()), (What::Flag, "Fetch all remotes"));
        let arg = |prefix: &str| At::Argument {
            command: "git".into(),
            position: 0,
            prefix: prefix.into(),
            quote: None,
        };
        let o = offer(&arg("fet"), &listings).unwrap();
        assert_eq!(names(&o), ["fetch", "fetch-pack"]);
        assert_eq!(o.choices[0].what, What::Subcommand);
        // No subcommand matches: files.
        assert_eq!(names(&offer(&arg("feat"), &listings).unwrap()), ["feature.txt"]);
    }
}
