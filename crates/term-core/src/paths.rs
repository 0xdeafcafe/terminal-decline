//! File paths written in the text, for a click on one: absolute ones,
//! `~/…`, and relative ones resolved against the shell's folder (OSC 7).
//! Only a path that exists counts; whether it does is asked of the
//! caller (`exists`), so this stays free of the filesystem.
//!
//! A `:line` or `:line:col` after the path (compilers, grep, test runners)
//! is part of what is found and shown, but not of the path checked and
//! opened. Trailing punctuation from prose (`.`, `,`, `:`, a closing
//! bracket with no opening one) is left out, and a path may follow `=`,
//! `(`, `[` or `{` (`--path=src/a.rs`, `[a](src/a.rs)`). A path holding
//! spaces is found only when written in quotes.

/// Characters that end a path written without quotes (besides whitespace
/// and controls).
const PATH_BREAKS: &str = "\"'`<>|;";

/// What a path may follow inside a run of text.
const PATH_STARTS: &str = "=([{";

/// The longest run of text looked at, in characters.
const MAX_LEN: usize = 4096;

fn is_path_char(c: char) -> bool {
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

/// The path around `i` in `text` that `exists` says is there, or `None`.
/// `cwd` is the folder a relative path is in (none: relative paths aren't
/// found), `home` what `~` stands for.
pub fn path_at(text: &[char], i: usize, cwd: Option<&str>, home: Option<&str>, mut exists: impl FnMut(&str) -> bool) -> Option<Found> {
    let c = *text.get(i)?;
    if let Some((a, b)) = quoted(text, i)
        && let Some(f) = candidate(text, a, b, i, false, cwd, home, &mut exists)
    {
        return Some(f);
    }
    if !is_path_char(c) {
        return None;
    }
    let mut a = i;
    while a > 0 && i - a < MAX_LEN && is_path_char(text[a - 1]) {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < text.len() && b - i < MAX_LEN && is_path_char(text[b + 1]) {
        b += 1;
    }
    // The whole run first, then from after each `=`, `(`, … before `i`.
    let starts = std::iter::once(a).chain((a..i).filter(|&k| PATH_STARTS.contains(text[k])).map(|k| k + 1));
    for s in starts.take(8) {
        if let Some(f) = candidate(text, s, b, i, true, cwd, home, &mut exists) {
            return Some(f);
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
#[allow(clippy::too_many_arguments)]
fn candidate(
    text: &[char],
    a: usize,
    mut b: usize,
    i: usize,
    trim: bool,
    cwd: Option<&str>,
    home: Option<&str>,
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
    let path = resolve(&written, cwd, home)?;
    exists(&path).then_some(Found { start: a, end, path })
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
        "/Users/me/src/app/app/[id]/page.tsx",
        "/Users/me/src/app/crates",
    ];

    fn find(line: &str, col: usize) -> Option<(String, String)> {
        let chars: Vec<char> = line.chars().collect();
        path_at(&chars, col, Some("/Users/me/src/app"), Some("/Users/me"), |p| FILES.contains(&p))
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
            ("open /tmp/a b/c.txt now", 13, None),
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
        assert_eq!(path_at(&chars, 0, None, None, |_| true), None);
        let abs: Vec<char> = "/etc/hosts".chars().collect();
        assert!(path_at(&abs, 0, None, None, |_| true).is_some());
        let tilde: Vec<char> = "~/x.txt".chars().collect();
        assert_eq!(path_at(&tilde, 0, None, None, |_| true), None, "no home");
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
