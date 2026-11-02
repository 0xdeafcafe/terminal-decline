//! Links in the terminal (roadmap E.2): what is under the pointer (an
//! OSC 8 hyperlink, else a plain URL in the text, else a file path that
//! exists), and which of them Cmd-click opens. A plain click on one
//! selects it and opens a small menu (Open, Copy; for a path also Reveal
//! in Finder); see the pane view.
//!
//! Only `http`, `https` and `mailto` links open as they are, and `file`
//! links naming this Mac (no host, `localhost` or its own name). Anything
//! else a program could make a link of (`x-apple…`, `ssh:`, app URL
//! schemes, other machines' files) is refused with a notice: clicking text
//! a program printed shouldn't start arbitrary apps.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use term_core::{Point, Terminal};

/// The link at `p`: its OSC 8 URI, else the plain URL written there
/// ([`Terminal::url_at`]).
pub fn under(t: &Terminal, p: Point) -> Option<String> {
    t.link_at(p).map(|l| l.uri).or_else(|| t.url_at(p).map(|(_, _, url)| url))
}

/// What is under the pointer, for the hover underline and a click.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// An OSC 8 link or a plain URL: what it opens, and its cells.
    Url { uri: String, start: Point, end: Point },
    /// A file path that exists: as written (with its `:line:col`), and
    /// the absolute path it names (without).
    Path {
        shown: String,
        path: String,
        start: Point,
        end: Point,
        /// Its cells row by row ([`term_core::PathAt::pieces`]).
        pieces: Vec<(Point, Point)>,
    },
    /// Something written as a path (with a `/`, or starting with `~`)
    /// that doesn't exist: found only for a click, whose menu offers to
    /// copy it or reveal the closest folder that does exist.
    Missing {
        shown: String,
        path: String,
        start: Point,
        end: Point,
        pieces: Vec<(Point, Point)>,
    },
}

impl Target {
    /// Its first and last cell.
    pub fn span(&self) -> (Point, Point) {
        match self {
            Target::Url { start, end, .. } | Target::Path { start, end, .. } | Target::Missing { start, end, .. } => (*start, *end),
        }
    }

    /// Its cells row by row, for the hover underline: a path's pieces,
    /// or a link's first and last cell.
    pub fn pieces(&self) -> Vec<(Point, Point)> {
        match self {
            Target::Url { start, end, .. } => vec![(*start, *end)],
            Target::Path { pieces, .. } | Target::Missing { pieces, .. } => pieces.clone(),
        }
    }

    /// What Copy (Copy Path) puts on the clipboard: the URL, or the path
    /// as shown, `:line:col` and all.
    pub fn copy_text(&self) -> &str {
        match self {
            Target::Url { uri, .. } => uri,
            Target::Path { shown, .. } | Target::Missing { shown, .. } => shown,
        }
    }
}

/// What a click at `p` acts on: [`target_at`], else a path that doesn't
/// exist ([`Target::Missing`]).
pub fn click_target_at(t: &Terminal, p: Point, cache: &mut ExistsCache) -> Option<Target> {
    target_at(t, p, cache).or_else(|| missing_at(t, p))
}

/// Something written as a path at `p` that isn't there: the run of path
/// characters around it, if it holds a `/` or starts with `~` (a bare
/// word would be any prose).
fn missing_at(t: &Terminal, p: Point) -> Option<Target> {
    if !t.cwd_host().is_none_or(|h| is_this_mac(h, &host_name())) {
        return None;
    }
    let home = std::env::var("HOME").ok();
    let found = t.path_at(p, home.as_deref(), |_| true)?;
    let written = found.shown.split(':').next().unwrap_or_default();
    if !(written.contains('/') || written.starts_with('~')) || written.contains("://") {
        return None;
    }
    Some(Target::Missing {
        shown: found.shown,
        path: found.path,
        start: found.start,
        end: found.end,
        pieces: found.pieces,
    })
}

/// The closest folder above `path` that exists (asked afresh).
pub fn nearest_existing(path: &str) -> Option<std::path::PathBuf> {
    std::path::Path::new(path)
        .ancestors()
        .skip(1)
        .find(|a| !a.as_os_str().is_empty() && a.exists())
        .map(Into::into)
}

/// The link or path at `p`: an OSC 8 link, else a plain URL, else a path
/// that exists ([`Terminal::path_at`], asking `cache`). Paths are only
/// looked for while the shell is on this Mac (its OSC 7 host is ours or
/// none): another machine's paths can't be checked or opened here.
pub fn target_at(t: &Terminal, p: Point, cache: &mut ExistsCache) -> Option<Target> {
    if let Some(link) = t.link_at(p) {
        let (start, end) = t.link_span(p).unwrap_or((p, p));
        return Some(Target::Url { uri: link.uri, start, end });
    }
    if let Some((start, end, uri)) = t.url_at(p) {
        return Some(Target::Url { uri, start, end });
    }
    if !t.cwd_host().is_none_or(|h| is_this_mac(h, &host_name())) {
        return None;
    }
    let home = std::env::var("HOME").ok();
    let found = t.path_at(p, home.as_deref(), |path| cache.exists(path))?;
    Some(Target::Path {
        shown: found.shown,
        path: found.path,
        start: found.start,
        end: found.end,
        pieces: found.pieces,
    })
}

/// Whether a host named in a `file://` link or OSC 7 (`name`) is this
/// Mac (`host`): none, `localhost`, or its own name with or without
/// `.local`.
pub fn is_this_mac(name: &str, host: &str) -> bool {
    let ours = |h: &str| h.eq_ignore_ascii_case(name) || h.strip_suffix(".local").is_some_and(|h| h.eq_ignore_ascii_case(name));
    name.is_empty() || name.eq_ignore_ascii_case("localhost") || ours(host)
}

/// Whether paths exist, remembered for a while, so the hover underline
/// doesn't ask the filesystem on every mouse move.
#[derive(Default)]
pub struct ExistsCache {
    entries: HashMap<String, (bool, Instant)>,
}

/// How long an answer is kept: a file made or removed shows within this.
const EXISTS_FOR: Duration = Duration::from_secs(3);
/// Answers kept at most; past it they are all dropped.
const EXISTS_MAX: usize = 512;

impl ExistsCache {
    pub fn exists(&mut self, path: &str) -> bool {
        self.exists_with(path, Instant::now(), |p| std::fs::metadata(p).is_ok())
    }

    fn exists_with(&mut self, path: &str, now: Instant, check: impl FnOnce(&str) -> bool) -> bool {
        if let Some(&(there, at)) = self.entries.get(path)
            && now.duration_since(at) < EXISTS_FOR
        {
            return there;
        }
        if self.entries.len() >= EXISTS_MAX {
            self.entries.clear();
        }
        let there = check(path);
        self.entries.insert(path.to_string(), (there, now));
        there
    }

    /// Forget every answer, so the next ones are asked afresh.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Why a link isn't opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A scheme other than http, https, mailto and file.
    Scheme,
    /// A file on another machine.
    RemoteFile,
}

impl Refused {
    /// The notice shown in the terminal.
    pub fn notice(&self, uri: &str) -> String {
        let shown: String = if uri.chars().count() > 80 {
            uri.chars().take(79).chain(['…']).collect()
        } else {
            uri.into()
        };
        match self {
            Refused::Scheme => format!("not opening {shown}: only http, https, mailto and file links open"),
            Refused::RemoteFile => format!("not opening {shown}: it is a file on another machine"),
        }
    }
}

/// What opening `uri` opens: http, https and mailto links as they are; a
/// file link on this Mac (`host` is its name) as `file:///path`, since
/// macOS opens only those. Schemes are matched in any case.
pub fn to_open(uri: &str, host: &str) -> Result<String, Refused> {
    let (scheme, rest) = uri.split_once(':').ok_or(Refused::Scheme)?;
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" | "mailto" => Ok(uri.to_string()),
        "file" => {
            let Some(after) = rest.strip_prefix("//") else {
                // `file:/path`: no host.
                return Ok(format!("file://{rest}"));
            };
            let (name, path) = after.split_at(after.find('/').unwrap_or(after.len()));
            if !is_this_mac(name, host) {
                return Err(Refused::RemoteFile);
            }
            Ok(format!("file://{}", if path.is_empty() { "/" } else { path }))
        }
        _ => Err(Refused::Scheme),
    }
}

/// This Mac's host name, as programs put it in `file://` links (what
/// `hostname` prints, often `name.local`).
pub fn host_name() -> String {
    let mut buf = [0u8; 256];
    let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if r != 0 {
        return String::new();
    }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..len]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_mail_and_local_files_open() {
        let host = "mac.local";
        for (uri, want) in [
            ("https://example.com/a?b", Ok("https://example.com/a?b")),
            ("HTTP://example.com", Ok("HTTP://example.com")),
            ("mailto:me@example.com", Ok("mailto:me@example.com")),
            ("file:///tmp/a%20b", Ok("file:///tmp/a%20b")),
            ("file://localhost/tmp/x", Ok("file:///tmp/x")),
            ("file://mac.local/tmp/x", Ok("file:///tmp/x")),
            ("file://MAC/tmp/x", Ok("file:///tmp/x")),
            ("file://mac.local", Ok("file:///")),
            ("file:/tmp/x", Ok("file:///tmp/x")),
            ("file://other/tmp/x", Err(Refused::RemoteFile)),
            ("file://mac.example.com/x", Err(Refused::RemoteFile)),
            ("ssh://host", Err(Refused::Scheme)),
            ("x-apple.systempreferences:com.apple", Err(Refused::Scheme)),
            ("javascript:alert(1)", Err(Refused::Scheme)),
            ("no scheme", Err(Refused::Scheme)),
            ("", Err(Refused::Scheme)),
        ] {
            assert_eq!(to_open(uri, host), want.map(String::from), "{uri}");
        }
        assert_eq!(to_open("file://h/x", "h"), Ok("file:///x".into()), "a name without .local");
    }

    #[test]
    fn notices_name_the_link() {
        assert_eq!(
            Refused::Scheme.notice("ssh://h"),
            "not opening ssh://h: only http, https, mailto and file links open"
        );
        let long = format!("ssh://{}", "a".repeat(100));
        let n = Refused::RemoteFile.notice(&long);
        assert!(n.starts_with("not opening ssh://aaa") && n.contains("…: it is a file"), "{n}");
    }

    #[test]
    fn under_prefers_the_osc8_link() {
        let mut t = term_core::Term::new(40, 2, 0, term_core::ImagePool::process());
        t.advance(b"\x1b]8;;https://real\x1b\\https://shown\x1b]8;;\x1b\\ http://plain.example x");
        let at = |col| under(&t.state, t.state.point_at(0, col));
        assert_eq!(at(3).as_deref(), Some("https://real"));
        assert_eq!(at(16).as_deref(), Some("http://plain.example"));
        assert_eq!(at(35), None);
    }

    /// A click on something written as a path that isn't there still
    /// finds it, with the closest folder above it that exists; a bare
    /// word or a URL doesn't count.
    #[test]
    fn clicks_find_paths_that_dont_exist() {
        let mut t = term_core::Term::new(60, 2, 0, term_core::ImagePool::process());
        t.advance(b"\x1b]7;file:///tmp\x1b\\cat /tmp/zq9-gone/a.txt:3 nope-zq9 ok");
        let mut cache = ExistsCache::default();
        let mut at = |col| click_target_at(&t.state, t.state.point_at(0, col), &mut cache);
        match at(8) {
            Some(Target::Missing { shown, path, .. }) => {
                assert_eq!(shown, "/tmp/zq9-gone/a.txt:3");
                assert_eq!(path, "/tmp/zq9-gone/a.txt");
                assert_eq!(nearest_existing(&path), Some("/tmp".into()));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(at(28), None, "a bare word");
        assert_eq!(at(1), None, "the command");
        assert_eq!(nearest_existing("/"), None);
    }

    #[test]
    fn targets_are_links_then_urls_then_paths() {
        let mut t = term_core::Term::new(60, 2, 0, term_core::ImagePool::process());
        t.advance(
            b"\x1b]7;file:///tmp\x1b\\\x1b]8;;https://real\x1b\\shown\x1b]8;;\x1b\\ http://plain.example /etc/hosts:3 nope-zq9-missing",
        );
        let mut cache = ExistsCache::default();
        let at = |t: &term_core::Term, cache: &mut ExistsCache, col| target_at(&t.state, t.state.point_at(0, col), cache);
        let p = |col| t.state.point_at(0, col);
        let url = |uri: &str, a, b| Target::Url {
            uri: uri.into(),
            start: p(a),
            end: p(b),
        };
        assert_eq!(at(&t, &mut cache, 2), Some(url("https://real", 0, 4)));
        assert_eq!(at(&t, &mut cache, 8), Some(url("http://plain.example", 6, 25)));
        let path = at(&t, &mut cache, 30).expect("/etc/hosts is on every Mac");
        assert_eq!(path.copy_text(), "/etc/hosts:3");
        assert!(matches!(&path, Target::Path { path, .. } if path == "/etc/hosts"));
        assert_eq!(path.span(), (p(27), p(38)));
        assert_eq!(at(&t, &mut cache, 40), None);

        // Over ssh, paths name another machine's files.
        t.advance(b"\x1b]7;file://elsewhere.example/tmp\x1b\\");
        assert_eq!(at(&t, &mut cache, 30), None);
    }

    #[test]
    fn exists_answers_are_kept_a_while() {
        let mut cache = ExistsCache::default();
        let t0 = Instant::now();
        let mut asked = 0;
        let mut ask = |cache: &mut ExistsCache, now| {
            cache.exists_with("/x", now, |_| {
                asked += 1;
                true
            })
        };
        assert!(ask(&mut cache, t0));
        assert!(ask(&mut cache, t0 + Duration::from_secs(1)));
        assert!(ask(&mut cache, t0 + EXISTS_FOR));
        assert_eq!(asked, 2, "asked again once the answer is old");
    }

    #[test]
    fn host_name_is_known() {
        assert!(!host_name().is_empty());
    }
}
