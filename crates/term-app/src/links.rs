//! Links in the terminal (roadmap E.2): what is under the pointer while
//! Cmd is held (an OSC 8 hyperlink, else a plain URL in the text), and
//! which of them Cmd-click opens.
//!
//! Only `http`, `https` and `mailto` links open as they are, and `file`
//! links naming this Mac (no host, `localhost` or its own name). Anything
//! else a program could make a link of (`x-apple…`, `ssh:`, app URL
//! schemes, other machines' files) is refused with a notice: clicking text
//! a program printed shouldn't start arbitrary apps.

use term_core::{Point, Terminal};

/// The link at `p`: its OSC 8 URI, else the plain URL written there
/// ([`Terminal::url_at`]).
pub fn under(t: &Terminal, p: Point) -> Option<String> {
    t.link_at(p).map(|l| l.uri).or_else(|| t.url_at(p).map(|(_, _, url)| url))
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
            let ours = |h: &str| h.eq_ignore_ascii_case(name) || h.strip_suffix(".local").is_some_and(|h| h.eq_ignore_ascii_case(name));
            let local = name.is_empty() || name.eq_ignore_ascii_case("localhost") || ours(host);
            if !local {
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

    #[test]
    fn host_name_is_known() {
        assert!(!host_name().is_empty());
    }
}
