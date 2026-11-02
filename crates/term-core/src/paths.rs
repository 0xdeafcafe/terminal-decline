//! File paths written in the text, for a click on one: absolute ones,
//! `~/…`, `$VAR/…` and `${VAR}/…`, and relative ones resolved against
//! the shell's folder (OSC 7) and then the other folders in [`Places`]
//! (those of the program running and its child processes). Only a path
//! that exists counts; whether it does is asked of the caller (`exists`),
//! so this stays free of the filesystem.
//!
//! A `:line` or `:line:col` after the path (compilers, grep, test runners)
//! is part of what is found and shown, but not of the path checked and
//! opened. Trailing punctuation from prose (`.`, `,`, `:`, a closing
//! bracket with no opening one) is left out, and a path may follow `=`,
//! `(`, `[` or `{` (`--path=src/a.rs`, `[a](src/a.rs)`). A path holding
//! spaces is found in quotes, with its spaces escaped (`a\ b`), or bare
//! when it starts with `/` or `~`. `a/…` and `b/…` as `git diff` prints
//! them are tried without that prefix too.

/// Where the paths written in the text may be, besides the shell's
/// folder: what `~` stands for, other folders a relative path is tried
/// in (in order), and environment variables' values. A name may come more
/// than once, with different values (each program its own `$TMPDIR`):
/// each is tried in turn, and the first path that exists wins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    pub home: Option<String>,
    pub cwds: Vec<String>,
    pub vars: Vec<(String, String)>,
}

impl Places {
    /// Only `home`.
    pub fn home(home: Option<&str>) -> Places {
        Places {
            home: home.map(str::to_string),
            ..Places::default()
        }
    }

    fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let home = (name == "HOME").then_some(self.home.as_deref()).flatten();
        let own = self.vars.iter().filter(move |(n, _)| n == name).map(|(_, v)| v.as_str());
        home.into_iter().chain(own)
    }
}

/// The shell's folder and the [`Places`]: what a written path is resolved
/// against.
#[derive(Clone, Copy)]
struct Where<'a> {
    cwd: Option<&'a str>,
    places: &'a Places,
}

/// Characters that end a path written without quotes (besides whitespace
/// and controls).
const PATH_BREAKS: &str = "\"'`<>|;";

/// What a path may follow inside a run of text.
const PATH_STARTS: &str = "=([{";

/// The longest run of text looked at, in characters.
const MAX_LEN: usize = 4096;

pub(crate) fn is_path_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !PATH_BREAKS.contains(c) && !crate::word::is_drawing(c)
}

/// A path found in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The indices (inclusive) of what was found: the path as written,
    /// with its `:line:col`.
    pub start: usize,
    pub end: usize,
    /// The path it names, absolute, without `:line:col`.
    pub path: String,
}

/// Whether `text[k]` is part of a path written without quotes: a path
/// character, or a space escaped with a backslash.
fn in_path(text: &[char], k: usize) -> bool {
    is_path_char(text[k]) || (text[k] == ' ' && k > 0 && text[k - 1] == '\\')
}

/// The path around `i` in `text` that `exists` says is there, or `None`.
/// `cwd` is the shell's folder, the first a relative path is tried in
/// (none and no others in `places`: relative paths aren't found).
pub fn path_at(text: &[char], i: usize, cwd: Option<&str>, places: &Places, mut exists: impl FnMut(&str) -> bool) -> Option<Found> {
    let at = Where { cwd, places };
    text.get(i)?;
    if let Some((a, b)) = quoted(text, i)
        && let Some(f) = candidate(text, a, b, i, false, at, &mut exists)
    {
        return Some(f);
    }
    if !in_path(text, i) {
        return None;
    }
    let mut a = i;
    while a > 0 && i - a < MAX_LEN && in_path(text, a - 1) {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < text.len() && b - i < MAX_LEN && in_path(text, b + 1) {
        b += 1;
    }
    // The whole run first, then from after each `=`, `(`, … before `i`.
    let starts = std::iter::once(a).chain((a..i).filter(|&k| PATH_STARTS.contains(text[k])).map(|k| k + 1));
    for s in starts.take(8) {
        if let Some(f) = candidate(text, s, b, i, true, at, &mut exists) {
            return Some(f);
        }
    }
    spaced(text, a, b, i, at, &mut exists)
}

/// Words a path with spaces may take in on either side, written without
/// quotes.
const SPACED_WORDS: usize = 3;

/// A path holding spaces written without quotes, as programs print them
/// (`installed /Applications/Terminal Decline.app`): the run `a..=b` with
/// up to [`SPACED_WORDS`] words on either side, each one space away. It
/// must start with `/` or `~` (a bare name with spaces would be any
/// prose) and exist; the longest from the nearest start wins.
fn spaced(text: &[char], a: usize, b: usize, i: usize, at: Where, exists: &mut impl FnMut(&str) -> bool) -> Option<Found> {
    let mut starts = vec![a];
    while starts.len() <= SPACED_WORDS {
        let s = *starts.last()?;
        if s < 2 || text[s - 1] != ' ' || !is_path_char(text[s - 2]) {
            break;
        }
        let mut k = s - 2;
        while k > 0 && is_path_char(text[k - 1]) {
            k -= 1;
        }
        starts.push(k);
    }
    let mut ends = vec![b];
    while ends.len() <= SPACED_WORDS {
        let e = *ends.last()?;
        if e + 2 >= text.len() || text[e + 1] != ' ' || !is_path_char(text[e + 2]) {
            break;
        }
        let mut k = e + 2;
        while k + 1 < text.len() && is_path_char(text[k + 1]) {
            k += 1;
        }
        ends.push(k);
    }
    for &s in &starts {
        if !matches!(text[s], '/' | '~' | '$') {
            continue;
        }
        for &e in ends.iter().rev() {
            if (s, e) == (a, b) {
                continue;
            }
            if let Some(f) = candidate(text, s, e, i, true, at, exists) {
                return Some(f);
            }
        }
    }
    None
}

/// The text between the quotes around `i` (`"…"` or `'…'`), when it holds
/// a space (else the path without quotes is the same).
fn quoted(text: &[char], i: usize) -> Option<(usize, usize)> {
    if matches!(text[i], '"' | '\'') {
        return None;
    }
    let open = (i.saturating_sub(MAX_LEN)..i).rev().find(|&k| matches!(text[k], '"' | '\''))?;
    let q = text[open];
    let close = (i + 1..text.len().min(i + MAX_LEN)).find(|&k| text[k] == q)?;
    let (a, b) = (open + 1, close - 1);
    let inner = &text[a..=b];
    let spaced = inner.iter().any(|c| c.is_whitespace());
    let trimmed = !inner[0].is_whitespace() && !inner[inner.len() - 1].is_whitespace();
    (spaced && trimmed).then_some((a, b))
}

/// The path in `text[a..=b]`, if it is one that exists and `i` is on it.
/// `trim`: leave out trailing punctuation (not inside quotes).
fn candidate(
    text: &[char],
    a: usize,
    mut b: usize,
    i: usize,
    trim: bool,
    at: Where,
    exists: &mut impl FnMut(&str) -> bool,
) -> Option<Found> {
    if trim {
        while b > a {
            let c = text[b];
            let open = match c {
                ')' => Some('('),
                ']' => Some('['),
                '}' => Some('{'),
                _ => None,
            };
            let unbalanced = open.is_some_and(|o| {
                let span = &text[a..=b];
                span.iter().filter(|&&x| x == o).count() < span.iter().filter(|&&x| x == c).count()
            });
            if ".,:;!?".contains(c) || unbalanced {
                b -= 1;
            } else {
                break;
            }
        }
    }
    if a > b {
        return None;
    }
    let (path_end, end) = location(text, a, b);
    if i < a || i > end || path_end <= a {
        return None;
    }
    let written: String = text[a..path_end].iter().collect();
    if !looks_like_path(&written) {
        return None;
    }
    let path = resolutions(&written, at).into_iter().find(|p| exists(p))?;
    Some(Found { start: a, end, path })
}

/// The most paths one written path is tried as.
const MAX_TRIES: usize = 16;

/// `written` as the absolute paths it may name, the likeliest first:
/// escaped spaces unescaped, each `$VAR` and `${VAR}` replaced with each
/// of its values, then resolved as [`resolve`] does in the shell's folder
/// and each folder of `places`, and a `git diff` `a/` or `b/` prefix left
/// out last. None when a variable has no value.
fn resolutions(written: &str, at: Where) -> Vec<String> {
    let unescaped: String;
    let written = if written.contains('\\') {
        let mut out = String::with_capacity(written.len());
        let mut chars = written.chars();
        while let Some(c) = chars.next() {
            out.push(if c == '\\' { chars.next().unwrap_or(c) } else { c });
        }
        unescaped = out;
        &unescaped
    } else {
        written
    };
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: String| {
        if out.len() < MAX_TRIES && !out.contains(&p) {
            out.push(p);
        }
    };
    let home = at.places.home.as_deref();
    // A bare name (no `/`) only in the shell's folder: any word of prose
    // names something in one of many folders.
    let cwds = |w: &str| {
        let others = if w.contains('/') { at.places.cwds.as_slice() } else { &[] };
        at.cwd.into_iter().chain(others.iter().map(String::as_str))
    };
    for w in expand(written, at.places) {
        if w.starts_with('/') || w == "~" || w.starts_with("~/") {
            if let Some(p) = resolve(&w, None, home) {
                push(p);
            }
            continue;
        }
        for cwd in cwds(&w) {
            if let Some(p) = resolve(&w, Some(cwd), home) {
                push(p);
            }
        }
        if let Some(rest) = w.strip_prefix("a/").or_else(|| w.strip_prefix("b/")) {
            for cwd in cwds(rest) {
                if let Some(p) = resolve(rest, Some(cwd), home) {
                    push(p);
                }
            }
        }
    }
    out
}

/// `written` with each `$NAME` and `${NAME}` replaced by each value
/// `places` has for it, every combination up to [`MAX_TRIES`]. Itself
/// when it names none; nothing when one has no value.
fn expand(written: &str, places: &Places) -> Vec<String> {
    let mut done = vec![String::new()];
    let mut rest = written;
    while let Some(k) = rest.find('$') {
        let after = &rest[k + 1..];
        let (name, len) = match after.strip_prefix('{') {
            Some(inner) => match inner.find('}') {
                Some(e) => (&inner[..e], e + 2),
                None => ("", 0),
            },
            None => {
                let e = after
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(after.len());
                (&after[..e], e)
            }
        };
        let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        if !valid {
            // A `$` that isn't a variable is kept as written.
            for d in &mut done {
                d.push_str(&rest[..=k]);
            }
            rest = after;
            continue;
        }
        let values: Vec<&str> = places.values(name).collect();
        if values.is_empty() {
            return Vec::new();
        }
        let mut next = Vec::new();
        for d in &done {
            for v in &values {
                if next.len() < MAX_TRIES {
                    next.push(format!("{d}{}{}", &rest[..k], v.trim_end_matches('/')));
                }
            }
        }
        next.dedup();
        done = next;
        rest = &after[len..];
    }
    for d in &mut done {
        d.push_str(rest);
    }
    done
}

/// Where the path in `text[a..=b]` ends (exclusive), before a `:line` or
/// `:line:col`, and where what is found ends (inclusive): after that
/// suffix, which drops what follows it (grep's `:text`).
fn location(text: &[char], a: usize, b: usize) -> (usize, usize) {
    let digits = |from: usize| (from..=b).take_while(|&k| text[k].is_ascii_digit()).count();
    for k in a + 1..b {
        if text[k] != ':' {
            continue;
        }
        let n = digits(k + 1);
        if n == 0 {
            continue;
        }
        let mut end = k + n;
        if end < b && text[end + 1] == ':' {
            let m = digits(end + 2);
            if m > 0 {
                end += 1 + m;
            }
        }
        return (k, end);
    }
    (b + 1, b)
}

/// Whether `s` may be a path: not a URL, nor only dots. A bare name
/// (`crates`, as `ls` lists it) counts: it is only found when the shell's
/// folder holds it.
fn looks_like_path(s: &str) -> bool {
    !s.contains("://") && !s.chars().all(|c| c == '.')
}

/// `written` as an absolute path: `~` and `~/…` in `home`, a relative
/// path in `cwd`. `None` when there is nothing to resolve it against.
pub fn resolve(written: &str, cwd: Option<&str>, home: Option<&str>) -> Option<String> {
    if written == "~" {
        return home.map(str::to_string);
    }
    if let Some(rest) = written.strip_prefix("~/") {
        return home.map(|h| format!("{}/{rest}", h.trim_end_matches('/')));
    }
    if written.starts_with('/') {
        return Some(written.to_string());
    }
    let mut rel = written;
    while let Some(r) = rel.strip_prefix("./") {
        rel = r;
    }
    let cwd = cwd.filter(|c| c.starts_with('/'))?;
    Some(format!("{}/{rel}", cwd.trim_end_matches('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILES: &[&str] = &[
        "/Users/me/src/app/main.rs",
        "/Users/me/src/app/Cargo.toml",
        "/Users/me/src/app",
        "/Users/me/notes.txt",
        "/Users/me",
        "/tmp/a b/c.txt",
        "/etc/hosts",
        "/Applications/Terminal Decline.app",
        "/Users/me/src/app/app/[id]/page.tsx",
        "/Users/me/src/app/crates",
    ];

    fn find(line: &str, col: usize) -> Option<(String, String)> {
        let chars: Vec<char> = line.chars().collect();
        path_at(&chars, col, Some("/Users/me/src/app"), &Places::home(Some("/Users/me")), |p| {
            FILES.contains(&p)
        })
        .map(|f| (chars[f.start..=f.end].iter().collect(), f.path))
    }

    #[test]
    fn paths_that_exist() {
        type Case = (&'static str, usize, Option<(&'static str, &'static str)>);
        let cases: &[Case] = &[
            ("see /etc/hosts now", 6, Some(("/etc/hosts", "/etc/hosts"))),
            ("see /etc/hosts now", 2, None),
            ("see /etc/nope now", 6, None),
            ("~/notes.txt", 3, Some(("~/notes.txt", "/Users/me/notes.txt"))),
            ("cd ~", 3, Some(("~", "/Users/me"))),
            ("main.rs", 0, Some(("main.rs", "/Users/me/src/app/main.rs"))),
            ("./main.rs", 4, Some(("./main.rs", "/Users/me/src/app/main.rs"))),
            ("--> main.rs:12:3", 6, Some(("main.rs:12:3", "/Users/me/src/app/main.rs"))),
            ("main.rs:12:3", 10, Some(("main.rs:12:3", "/Users/me/src/app/main.rs"))),
            ("main.rs:12", 2, Some(("main.rs:12", "/Users/me/src/app/main.rs"))),
            ("main.rs:12:fn main() {", 2, Some(("main.rs:12", "/Users/me/src/app/main.rs"))),
            ("main.rs:12:fn main() {", 12, None),
            ("main.rs:", 2, Some(("main.rs", "/Users/me/src/app/main.rs"))),
            ("(see main.rs)", 6, Some(("main.rs", "/Users/me/src/app/main.rs"))),
            ("(main.rs:4)", 2, Some(("main.rs:4", "/Users/me/src/app/main.rs"))),
            ("main.rs, Cargo.toml.", 10, Some(("Cargo.toml", "/Users/me/src/app/Cargo.toml"))),
            ("main.rs, Cargo.toml.", 1, Some(("main.rs", "/Users/me/src/app/main.rs"))),
            ("\"main.rs\"", 3, Some(("main.rs", "/Users/me/src/app/main.rs"))),
            ("'/etc/hosts'", 2, Some(("/etc/hosts", "/etc/hosts"))),
            ("\"main.rs\"", 0, None),
            ("open \"/tmp/a b/c.txt\" now", 9, Some(("/tmp/a b/c.txt", "/tmp/a b/c.txt"))),
            ("open \"/tmp/a b/c.txt\" now", 13, Some(("/tmp/a b/c.txt", "/tmp/a b/c.txt"))),
            // Without quotes too, when it starts with `/` or `~` and the
            // longer path exists, from any of its words.
            ("open /tmp/a b/c.txt now", 13, Some(("/tmp/a b/c.txt", "/tmp/a b/c.txt"))),
            ("open /tmp/a b/c.txt now", 6, Some(("/tmp/a b/c.txt", "/tmp/a b/c.txt"))),
            (
                "installed /Applications/Terminal Decline.app (0.0.1)",
                12,
                Some(("/Applications/Terminal Decline.app", "/Applications/Terminal Decline.app")),
            ),
            (
                "installed /Applications/Terminal Decline.app: done",
                36,
                Some(("/Applications/Terminal Decline.app", "/Applications/Terminal Decline.app")),
            ),
            ("installed /Applications/Terminal Decline.app (0.0.1)", 47, None),
            // Not from a bare name: prose naming it isn't a path.
            ("see Terminal Decline.app here", 6, None),
            (
                "--manifest-path=Cargo.toml",
                18,
                Some(("Cargo.toml", "/Users/me/src/app/Cargo.toml")),
            ),
            ("[the file](main.rs)", 13, Some(("main.rs", "/Users/me/src/app/main.rs"))),
            (
                "app/[id]/page.tsx",
                5,
                Some(("app/[id]/page.tsx", "/Users/me/src/app/app/[id]/page.tsx")),
            ),
            ("file:///etc/hosts", 9, None),
            ("..", 0, None),
            ("src", 1, None),
            // A bare name in the shell's folder, as `ls` lists it.
            ("Cargo.toml  crates", 13, Some(("crates", "/Users/me/src/app/crates"))),
            // A program's box around it isn't part of it.
            ("│ main.rs │", 3, Some(("main.rs", "/Users/me/src/app/main.rs"))),
            ("│main.rs│", 2, Some(("main.rs", "/Users/me/src/app/main.rs"))),
            ("", 0, None),
        ];
        for &(line, col, want) in cases {
            let got = find(line, col);
            let want = want.map(|(s, p)| (s.to_string(), p.to_string()));
            assert_eq!(got, want, "{line:?} at {col}");
        }
    }

    #[test]
    fn relative_paths_need_a_folder() {
        let chars: Vec<char> = "main.rs".chars().collect();
        let none = Places::default();
        assert_eq!(path_at(&chars, 0, None, &none, |_| true), None);
        let abs: Vec<char> = "/etc/hosts".chars().collect();
        assert!(path_at(&abs, 0, None, &none, |_| true).is_some());
        let tilde: Vec<char> = "~/x.txt".chars().collect();
        assert_eq!(path_at(&tilde, 0, None, &none, |_| true), None, "no home");
    }

    /// A program's own folders and variables, as a TUI running agents in
    /// other folders has them.
    fn places() -> Places {
        Places {
            home: Some("/Users/me".into()),
            cwds: vec!["/work/one".into(), "/work/two".into()],
            vars: vec![
                ("TMPDIR".into(), "/var/folders/T/".into()),
                ("TMPDIR".into(), "/tmp/agent".into()),
                ("PROJ".into(), "/work/two".into()),
            ],
        }
    }

    fn find_in(line: &str, col: usize, files: &[&str]) -> Option<(String, String)> {
        let chars: Vec<char> = line.chars().collect();
        path_at(&chars, col, Some("/shell"), &places(), |p| files.contains(&p)).map(|f| (chars[f.start..=f.end].iter().collect(), f.path))
    }

    #[test]
    fn variables_take_each_value_that_names_a_path() {
        let files = ["/tmp/agent/heavy/rounds", "/work/two/src/a.rs", "/Users/me/x"];
        let found = |line, col| find_in(line, col, &files);
        let want = |s: &str, p: &str| Some((s.to_string(), p.to_string()));
        assert_eq!(
            found("cd $TMPDIR/heavy/rounds", 5),
            want("$TMPDIR/heavy/rounds", "/tmp/agent/heavy/rounds")
        );
        assert_eq!(
            found("cd ${TMPDIR}/heavy/rounds", 5),
            want("${TMPDIR}/heavy/rounds", "/tmp/agent/heavy/rounds")
        );
        assert_eq!(found("\"$PROJ/src/a.rs\"", 3), want("$PROJ/src/a.rs", "/work/two/src/a.rs"));
        assert_eq!(found("$HOME/x", 2), want("$HOME/x", "/Users/me/x"));
        assert_eq!(found("$NOPE/x", 2), None, "no value");
        assert_eq!(found("cd $TMPDIR/gone", 5), None);
    }

    #[test]
    fn relative_paths_are_tried_in_each_folder() {
        let files = ["/work/two/src/a.rs", "/shell/b.rs", "/work/one/b.rs"];
        let found = |line, col| find_in(line, col, &files);
        assert_eq!(found("src/a.rs:3", 1), Some(("src/a.rs:3".into(), "/work/two/src/a.rs".into())));
        assert_eq!(
            found("b.rs", 1),
            Some(("b.rs".into(), "/shell/b.rs".into())),
            "the shell's folder first"
        );
        // As `git diff` prints them.
        assert_eq!(found("--- a/src/a.rs", 6), Some(("a/src/a.rs".into(), "/work/two/src/a.rs".into())));
        assert_eq!(found("+++ b/src/a.rs", 6), Some(("b/src/a.rs".into(), "/work/two/src/a.rs".into())));
    }

    #[test]
    fn escaped_spaces_are_part_of_the_path() {
        let files = ["/tmp/a b/c.txt", "/shell/my file.rs"];
        let found = |line, col| find_in(line, col, &files);
        assert_eq!(
            found("open /tmp/a\\ b/c.txt now", 7),
            Some(("/tmp/a\\ b/c.txt".into(), "/tmp/a b/c.txt".into()))
        );
        assert_eq!(
            found("open /tmp/a\\ b/c.txt now", 12),
            Some(("/tmp/a\\ b/c.txt".into(), "/tmp/a b/c.txt".into()))
        );
        assert_eq!(
            found("vim my\\ file.rs", 10),
            Some(("my\\ file.rs".into(), "/shell/my file.rs".into()))
        );
    }

    #[test]
    fn resolving() {
        assert_eq!(resolve("a/b", Some("/x/"), None).as_deref(), Some("/x/a/b"));
        assert_eq!(resolve("././a", Some("/x"), None).as_deref(), Some("/x/a"));
        assert_eq!(resolve("~/a", None, Some("/h/")).as_deref(), Some("/h/a"));
        assert_eq!(resolve("~user/a", Some("/x"), Some("/h")).as_deref(), Some("/x/~user/a"));
        assert_eq!(resolve("a", Some("relative"), None), None);
    }
}
