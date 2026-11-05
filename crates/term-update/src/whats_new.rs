//! What's New (docs/design/updater.md §12): the changes between the
//! build that ran before and this one, grouped as git-cliff groups them
//! (`cliff.toml`: Features, Fixes, Performance, …).
//!
//! The notes come as git-cliff's Markdown, put in the bundle at build
//! time (`Contents/Resources/whats-new.md`): a release's has every
//! version's section, a dogfood build's the range `dogfood.sh` picked,
//! named on its first line (`<!-- whats-new from=… to=… -->`). Without
//! one, a build from source groups its checkout's commit subjects by the
//! same rules ([`from_subjects`]). Here: reading them, picking a
//! release's versions ([`Notes::between`]), and the two ways they are
//! shown, the notification's few lines ([`Notes::summary`]) and the page
//! the What's New tab pages through ([`page`]). No files, no git.

use std::cmp::Ordering;

/// One line of the notes: `- **scope:** text`, or `- text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// The module, if the commit named one.
    pub scope: Option<String>,
    /// The commit's subject, with " (breaking)" after it if it was.
    pub text: String,
}

/// A `### Group` of items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub items: Vec<Item>,
}

/// A `## version - date` section; `## Unreleased` has no version.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Section {
    pub version: Option<String>,
    pub date: Option<String>,
    pub groups: Vec<Group>,
}

/// git-cliff's notes, newest section first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notes {
    pub sections: Vec<Section>,
    /// The commit the notes start after, for a dogfood build's.
    pub from: Option<String>,
    /// The commit they end at.
    pub to: Option<String>,
}

/// The groups, in the order shown: cliff.toml's, the ones a user cares
/// most about first. Any other group follows, in the order it came.
pub const ORDER: [&str; 5] = ["Features", "Fixes", "Performance", "Build and packaging", "Internal changes"];

/// git-cliff's Markdown (cliff.toml's template), with or without the
/// header, and the `<!-- whats-new from=… to=… -->` line dogfood.sh puts
/// first. Lines it doesn't know are skipped.
pub fn parse(text: &str) -> Notes {
    let mut notes = Notes::default();
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix("<!-- whats-new ") {
            for word in rest.trim_end_matches("-->").split_whitespace() {
                match word.split_once('=') {
                    Some(("from", v)) if !v.is_empty() => notes.from = Some(v.into()),
                    Some(("to", v)) if !v.is_empty() => notes.to = Some(v.into()),
                    _ => {}
                }
            }
        } else if let Some(head) = trimmed.strip_prefix("## ") {
            let head = head.trim();
            let (version, date) = match head.split_once(" - ") {
                Some((v, d)) => (v.trim(), Some(d.trim().to_string())),
                None => (head, None),
            };
            let version = (!version.eq_ignore_ascii_case("unreleased")).then(|| version.trim_start_matches('v').to_string());
            notes.sections.push(Section {
                version,
                date,
                groups: Vec::new(),
            });
        } else if let Some(name) = trimmed.strip_prefix("### ") {
            section(&mut notes).groups.push(Group {
                name: name.trim().to_string(),
                items: Vec::new(),
            });
        } else if let Some(entry) = trimmed.strip_prefix("- ") {
            let item = item(entry.trim());
            let s = section(&mut notes);
            if s.groups.is_empty() {
                s.groups.push(Group {
                    name: "Other changes".into(),
                    items: Vec::new(),
                });
            }
            if let Some(g) = s.groups.last_mut() {
                g.items.push(item);
            }
        }
    }
    notes.sections.retain(|s| s.groups.iter().any(|g| !g.items.is_empty()));
    for s in &mut notes.sections {
        s.groups.retain(|g| !g.items.is_empty());
    }
    notes
}

fn section(notes: &mut Notes) -> &mut Section {
    if notes.sections.is_empty() {
        notes.sections.push(Section::default());
    }
    notes.sections.last_mut().expect("just made")
}

/// `**scope:** text` or `text`.
fn item(entry: &str) -> Item {
    if let Some(rest) = entry.strip_prefix("**")
        && let Some((scope, text)) = rest.split_once(":**")
    {
        return Item {
            scope: Some(scope.trim().to_string()),
            text: text.trim().to_string(),
        };
    }
    Item {
        scope: None,
        text: entry.to_string(),
    }
}

/// The group cliff.toml puts a commit with this subject in, or `None`
/// for one the notes leave out (merges, dependency bumps, docs, tests,
/// CI, chores, and subjects that aren't Conventional Commits).
fn group_of(kind: &str, scope: Option<&str>) -> Option<&'static str> {
    match kind {
        "feat" => Some("Features"),
        "fix" => Some("Fixes"),
        "perf" => Some("Performance"),
        "refactor" => Some("Internal changes"),
        "build" if scope == Some("deps") => None,
        "build" => Some("Build and packaging"),
        _ => None,
    }
}

/// `type(scope)!: text`, if the subject is a Conventional Commit.
fn conventional(subject: &str) -> Option<(&str, Option<&str>, bool, &str)> {
    let (head, text) = subject.split_once(": ")?;
    let (head, breaking) = match head.strip_suffix('!') {
        Some(h) => (h, true),
        None => (head, false),
    };
    let (kind, scope) = match head.split_once('(') {
        Some((k, s)) => (k, Some(s.strip_suffix(')')?)),
        None => (head, None),
    };
    let ok = !kind.is_empty() && kind.bytes().all(|b| b.is_ascii_lowercase());
    ok.then_some((kind, scope, breaking, text.trim()))
}

/// The notes of these commit subjects (oldest first, as `git log
/// --reverse`), when git-cliff didn't make them: grouped and ordered as
/// cliff.toml does (in a group, the commits with a scope by scope, then
/// the rest).
pub fn from_subjects(subjects: &[String]) -> Notes {
    let mut groups: Vec<Group> = Vec::new();
    for subject in subjects {
        if subject.starts_with("Merge ") {
            continue;
        }
        let Some((kind, scope, breaking, text)) = conventional(subject) else {
            continue;
        };
        let Some(name) = group_of(kind, scope) else { continue };
        let text = if breaking { format!("{text} (breaking)") } else { text.to_string() };
        let item = Item {
            scope: scope.filter(|s| !s.is_empty()).map(str::to_string),
            text,
        };
        match groups.iter_mut().find(|g| g.name == name) {
            Some(g) => g.items.push(item),
            None => groups.push(Group {
                name: name.into(),
                items: vec![item],
            }),
        }
    }
    for g in &mut groups {
        // Stable: the commits of one scope stay oldest first.
        g.items.sort_by(|a, b| match (&a.scope, &b.scope) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        });
    }
    let sections = if groups.is_empty() {
        Vec::new()
    } else {
        vec![Section {
            version: None,
            date: None,
            groups,
        }]
    };
    Notes {
        sections,
        from: None,
        to: None,
    }
}

/// `0.10.2` against `0.9.0`, by number; anything after `-` or `+` (a
/// nightly's or a dev build's suffix) doesn't count.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn parts(v: &str) -> Vec<u64> {
        let v = v.trim_start_matches('v');
        let core = v.split(['-', '+']).next().unwrap_or("");
        core.split('.').map(|p| p.parse().unwrap_or(0)).collect()
    }
    let (mut a, mut b) = (parts(a), parts(b));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a.cmp(&b)
}

impl Notes {
    pub fn is_empty(&self) -> bool {
        self.sections.iter().all(|s| s.groups.iter().all(|g| g.items.is_empty()))
    }

    /// A release's notes from just after `from` up to and including
    /// `to`: the versions an update from `from` brought. Without `from`
    /// (no update recorded), `to`'s own. Unreleased sections never count.
    pub fn between(&self, from: Option<&str>, to: &str) -> Notes {
        let sections = self
            .sections
            .iter()
            .filter(|s| {
                let Some(v) = s.version.as_deref() else { return false };
                let upto = compare_versions(v, to) != Ordering::Greater;
                match from {
                    Some(f) => upto && compare_versions(v, f) == Ordering::Greater,
                    None => compare_versions(v, to) == Ordering::Equal,
                }
            })
            .cloned()
            .collect();
        Notes {
            sections,
            from: from.map(str::to_string),
            to: Some(to.to_string()),
        }
    }

    /// Every section's groups as one list, in [`ORDER`], each group's
    /// items newest section first.
    pub fn merged(&self) -> Vec<Group> {
        let mut out: Vec<Group> = Vec::new();
        for s in &self.sections {
            for g in &s.groups {
                match out.iter_mut().find(|o| o.name == g.name) {
                    Some(o) => o.items.extend(g.items.iter().cloned()),
                    None => out.push(g.clone()),
                }
            }
        }
        sort_groups(&mut out);
        out
    }

    /// The notification's body: how many of each, then the first few
    /// changes, Features first.
    ///
    /// ```text
    /// 3 features and 2 fixes.
    /// app: a title strip on each pane, and renaming it
    /// app: move a pane or place a new one with the keyboard
    /// and 3 more
    /// ```
    pub fn summary(&self) -> String {
        let groups = self.merged();
        let mut counts: Vec<String> = Vec::new();
        let mut other = 0;
        for g in &groups {
            let n = g.items.len();
            let (one, many) = match g.name.as_str() {
                "Features" => ("feature", "features"),
                "Fixes" => ("fix", "fixes"),
                "Performance" => ("performance change", "performance changes"),
                "Build and packaging" => ("packaging change", "packaging changes"),
                "Internal changes" => ("internal change", "internal changes"),
                _ => {
                    other += n;
                    continue;
                }
            };
            counts.push(format!("{n} {}", if n == 1 { one } else { many }));
        }
        if other > 0 {
            counts.push(format!("{other} other {}", if other == 1 { "change" } else { "changes" }));
        }
        let mut text = format!("{}.", and_list(&counts));
        let all: Vec<&Item> = groups.iter().flat_map(|g| &g.items).collect();
        for it in all.iter().take(SUMMARY_ITEMS) {
            text.push('\n');
            text.push_str(&plain(it));
        }
        if all.len() > SUMMARY_ITEMS {
            text.push_str(&format!("\nand {} more", all.len() - SUMMARY_ITEMS));
        }
        text
    }
}

/// The changes a notification names.
const SUMMARY_ITEMS: usize = 2;

fn sort_groups(groups: &mut [Group]) {
    let rank = |name: &str| ORDER.iter().position(|o| *o == name).unwrap_or(ORDER.len());
    groups.sort_by_key(|g| rank(&g.name));
}

/// "a", "a and b", "a, b and c".
fn and_list(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn plain(it: &Item) -> String {
    match &it.scope {
        Some(s) => format!("{s}: {}", it.text),
        None => it.text.clone(),
    }
}

const BOLD: &str = "\x1b[1m";
const FAINT: &str = "\x1b[2m";
const HEAD: &str = "\x1b[1;4m";
const RESET: &str = "\x1b[0m";

/// The What's New page, for `less -R`: `title` in bold, `under` (faint
/// lines) below it, the groups, then `footer` (faint). A release update
/// that spans versions keeps a heading per version; otherwise the groups
/// are one list.
pub fn page(title: &str, under: &[String], notes: &Notes, footer: &[String]) -> String {
    let mut out = format!("{BOLD}{title}{RESET}\n");
    for line in under {
        out.push_str(&format!("{FAINT}{line}{RESET}\n"));
    }
    let versions = notes.sections.iter().filter(|s| s.version.is_some()).count();
    if notes.is_empty() {
        out.push_str("\nNo changes to list.\n");
    } else if versions > 1 {
        for s in &notes.sections {
            let name = s.version.as_deref().unwrap_or("Unreleased");
            let date = s.date.as_deref().map(|d| format!(" {FAINT}{d}{RESET}")).unwrap_or_default();
            out.push_str(&format!("\n{BOLD}{name}{RESET}{date}\n"));
            let mut groups = s.groups.clone();
            sort_groups(&mut groups);
            groups_text(&mut out, &groups);
        }
    } else {
        groups_text(&mut out, &notes.merged());
    }
    if !footer.is_empty() {
        out.push('\n');
        for line in footer {
            out.push_str(&format!("{FAINT}{line}{RESET}\n"));
        }
    }
    out
}

fn groups_text(out: &mut String, groups: &[Group]) {
    for g in groups {
        out.push_str(&format!("\n{HEAD}{}{RESET}\n", g.name));
        for it in &g.items {
            match &it.scope {
                Some(s) => out.push_str(&format!("  • {BOLD}{s}:{RESET} {}\n", it.text)),
                None => out.push_str(&format!("  • {}\n", it.text)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `git cliff --strip header A..B` prints for a range that
    /// crosses a tag: no blank line before the second section.
    const CLIFF: &str = "<!-- whats-new from=aaa111 to=bbb222 -->
## Unreleased

### Fixes

- **app:** draw pane title strips as a themed band
- **app:** mark the focused pane's title strip with an accent rail
## 0.2.0 - 2026-11-04

### Features

- **app:** a title strip on each pane, and renaming it
- **complete:** mark the line that says why a command failed
- drop the old config keys (breaking)

### Performance

- **render:** keep each style's pen for the whole build
";

    fn it(scope: Option<&str>, text: &str) -> Item {
        Item {
            scope: scope.map(str::to_string),
            text: text.into(),
        }
    }

    #[test]
    fn reads_git_cliffs_notes() {
        let n = parse(CLIFF);
        assert_eq!(n.from.as_deref(), Some("aaa111"));
        assert_eq!(n.to.as_deref(), Some("bbb222"));
        assert_eq!(n.sections.len(), 2);
        assert_eq!(n.sections[0].version, None);
        assert_eq!(n.sections[1].version.as_deref(), Some("0.2.0"));
        assert_eq!(n.sections[1].date.as_deref(), Some("2026-11-04"));
        let names: Vec<&str> = n.sections[1].groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Features", "Performance"]);
        assert_eq!(
            n.sections[1].groups[0].items,
            [
                it(Some("app"), "a title strip on each pane, and renaming it"),
                it(Some("complete"), "mark the line that says why a command failed"),
                it(None, "drop the old config keys (breaking)"),
            ]
        );
    }

    #[test]
    fn parse_edges() {
        for (text, sections) in [
            ("", 0),
            ("# Changelog\n\nAll notable changes…\n", 0),
            // A heading with nothing under it is dropped.
            ("## 0.3.0 - 2026-12-01\n\n### Fixes\n", 0),
            // Items with no heading at all still count.
            ("- **app:** one\n", 1),
            ("## v0.3.0\n### Fixes\n- two\n", 1),
        ] {
            assert_eq!(parse(text).sections.len(), sections, "{text:?}");
        }
        let n = parse("## v0.3.0\n### Fixes\n- two\n");
        assert_eq!(n.sections[0].version.as_deref(), Some("0.3.0"), "the v goes");
        assert_eq!(n.sections[0].date, None);
        assert_eq!(parse("- **app:** one\n").sections[0].groups[0].name, "Other changes");
        assert_eq!(parse("<!-- whats-new from= to=b -->\n").from, None);
    }

    #[test]
    fn subjects_group_as_cliff_toml_does() {
        let subjects: Vec<String> = [
            "feat(render): b first",
            "fix: unscoped fix",
            "feat(app): a second",
            "docs: measure text size",
            "test(app): smoke",
            "build(deps): bump miniz_oxide",
            "build: ship arm64 only",
            "Merge branch 'main' into rush/x",
            "merge: rush/x",
            "perf(render)!: drop the cache",
            "refactor(core): split grid",
            "fix(app): scoped fix",
            "not conventional",
            "chore: tidy",
            "feat(render): c third",
        ]
        .map(String::from)
        .to_vec();
        let n = from_subjects(&subjects);
        let groups = n.merged();
        let got: Vec<(&str, Vec<String>)> = groups
            .iter()
            .map(|g| (g.name.as_str(), g.items.iter().map(plain).collect()))
            .collect();
        let want: Vec<(&str, Vec<String>)> = vec![
            (
                "Features",
                vec!["app: a second".into(), "render: b first".into(), "render: c third".into()],
            ),
            ("Fixes", vec!["app: scoped fix".into(), "unscoped fix".into()]),
            ("Performance", vec!["render: drop the cache (breaking)".into()]),
            ("Build and packaging", vec!["ship arm64 only".into()]),
            ("Internal changes", vec!["core: split grid".into()]),
        ];
        assert_eq!(got, want);
        assert!(from_subjects(&["docs: x".to_string()]).is_empty());
    }

    #[test]
    fn version_order() {
        for (a, b, want) in [
            ("0.10.0", "0.9.0", Ordering::Greater),
            ("0.2.0", "0.2.0", Ordering::Equal),
            ("v0.2.0", "0.2", Ordering::Equal),
            ("0.2.0-nightly.20261105", "0.2.0", Ordering::Equal),
            ("0.1.9", "0.2.0", Ordering::Less),
        ] {
            assert_eq!(compare_versions(a, b), want, "{a} vs {b}");
        }
    }

    #[test]
    fn a_release_shows_the_versions_it_brought() {
        let all = parse(
            "## Unreleased\n### Fixes\n- u\n\
             ## 0.4.0 - d4\n### Fixes\n- four\n\
             ## 0.3.0 - d3\n### Fixes\n- three\n\
             ## 0.2.0 - d2\n### Fixes\n- two\n\
             ## 0.1.0 - d1\n### Fixes\n- one\n",
        );
        let versions = |n: Notes| -> Vec<String> { n.sections.into_iter().filter_map(|s| s.version).collect() };
        for (from, to, want) in [
            (Some("0.2.0"), "0.4.0", vec!["0.4.0", "0.3.0"]),
            (Some("0.3.0"), "0.4.0", vec!["0.4.0"]),
            (Some("0.1.0"), "0.2.0", vec!["0.2.0"]),
            (None, "0.3.0", vec!["0.3.0"]),
            // Downgrades and unknown versions bring nothing.
            (Some("0.4.0"), "0.2.0", vec![]),
            (None, "0.9.0", vec![]),
        ] {
            assert_eq!(versions(all.between(from, to)), want, "{from:?}..{to}");
        }
    }

    #[test]
    fn the_notifications_lines() {
        let n = parse(CLIFF);
        assert_eq!(
            n.summary(),
            "3 features, 2 fixes and 1 performance change.\n\
             app: a title strip on each pane, and renaming it\n\
             complete: mark the line that says why a command failed\n\
             and 4 more"
        );
        let one = parse("### Fixes\n- **app:** only\n");
        assert_eq!(one.summary(), "1 fix.\napp: only");
        let other = parse("### Security\n- a\n- b\n### Features\n- f\n");
        assert_eq!(other.summary(), "1 feature and 2 other changes.\nf\na\nand 1 more");
    }

    #[test]
    fn the_page() {
        let n = parse(CLIFF);
        let p = page("What's New", &["Since 0.1.0".into()], &n, &["q closes this tab".into()]);
        let plain: String = strip(&p);
        assert_eq!(
            plain,
            "What's New\nSince 0.1.0\n\
             \nFeatures\n  • app: a title strip on each pane, and renaming it\n  • complete: mark the line that says why a command failed\n  • drop the old config keys (breaking)\n\
             \nFixes\n  • app: draw pane title strips as a themed band\n  • app: mark the focused pane's title strip with an accent rail\n\
             \nPerformance\n  • render: keep each style's pen for the whole build\n\
             \nq closes this tab\n"
        );
        // Two versions: a heading each.
        let two = parse("## 0.3.0 - d3\n### Fixes\n- three\n## 0.2.0 - d2\n### Features\n- two\n");
        assert_eq!(
            strip(&page("T", &[], &two, &[])),
            "T\n\n0.3.0 d3\n\nFixes\n  • three\n\n0.2.0 d2\n\nFeatures\n  • two\n"
        );
        assert_eq!(strip(&page("T", &[], &Notes::default(), &[])), "T\n\nNo changes to list.\n");
    }

    /// The page without its SGR sequences.
    fn strip(s: &str) -> String {
        let mut out = String::new();
        let mut in_sgr = false;
        for c in s.chars() {
            if in_sgr {
                in_sgr = c != 'm';
            } else if c == '\x1b' {
                in_sgr = true;
            } else {
                out.push(c);
            }
        }
        out
    }
}
