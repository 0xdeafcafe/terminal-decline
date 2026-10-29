//! iTerm2's inline images: the arguments of `OSC 1337 ; File=<args> :
//! <base64>` and `MultipartFile=<args>` (images.md §7.2).
//!
//! `<args>` is `key=value` pairs separated by `;`. Keys are case-sensitive,
//! as in iTerm2; unknown keys are ignored, and a value that doesn't parse
//! falls back to the key's default, as iTerm2 does.

use crate::base64;

/// `width=` or `height=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Dimension {
    /// `auto` (or absent, or unreadable): from the image.
    #[default]
    Auto,
    /// `N`: cells.
    Cells(u32),
    /// `Npx`: pixels.
    Pixels(u32),
    /// `N%`: percent of the screen's width or height.
    Percent(u32),
}

impl Dimension {
    fn parse(v: &[u8]) -> Dimension {
        let num = |d: &[u8]| -> Option<u32> {
            if d.is_empty() || d.len() > 9 || !d.iter().all(u8::is_ascii_digit) {
                return None;
            }
            std::str::from_utf8(d).ok()?.parse().ok()
        };
        if v.eq_ignore_ascii_case(b"auto") {
            Dimension::Auto
        } else if let Some(n) = v.strip_suffix(b"px").and_then(num) {
            Dimension::Pixels(n)
        } else if let Some(n) = v.strip_suffix(b"%").and_then(num) {
            Dimension::Percent(n)
        } else if let Some(n) = num(v) {
            Dimension::Cells(n)
        } else {
            Dimension::Auto
        }
    }
}

/// The arguments of one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    /// `inline=1`: show it. `inline=0` (the default) is a download, which
    /// the terminal drops.
    pub inline: bool,
    /// `name=`, base64-decoded; `None` if absent or not base64. At most
    /// [`MAX_NAME`] bytes are kept.
    pub name: Option<Vec<u8>>,
    /// `size=`: the file's size in bytes, if given.
    pub size: Option<u64>,
    pub width: Dimension,
    pub height: Dimension,
    /// `preserveAspectRatio=0` stretches; the default (1) fits inside the box.
    pub preserve_aspect_ratio: bool,
    /// `doNotMoveCursor=1` (iTerm2 3.5).
    pub do_not_move_cursor: bool,
}

impl Default for Args {
    fn default() -> Args {
        Args {
            inline: false,
            name: None,
            size: None,
            width: Dimension::Auto,
            height: Dimension::Auto,
            preserve_aspect_ratio: true,
            do_not_move_cursor: false,
        }
    }
}

/// Longest file name kept, decoded.
pub const MAX_NAME: usize = 1024;

/// Longest argument string read; the rest is ignored.
pub const MAX_ARGS_LEN: usize = 4096;

impl Args {
    /// Parse `<args>` (between `File=` and `:`).
    pub fn parse(args: &[u8]) -> Args {
        let args = &args[..args.len().min(MAX_ARGS_LEN)];
        let mut out = Args::default();
        for pair in args.split(|&b| b == b';') {
            let Some(eq) = pair.iter().position(|&b| b == b'=') else { continue };
            let (key, v) = (&pair[..eq], &pair[eq + 1..]);
            let flag = |v: &[u8]| v.first().is_some_and(|&b| b != b'0');
            match key {
                b"inline" => out.inline = flag(v),
                b"name" => {
                    out.name = base64::decode(v).ok().map(|mut n| {
                        n.truncate(MAX_NAME);
                        n
                    })
                }
                b"size" => {
                    out.size = (!v.is_empty() && v.len() <= 19 && v.iter().all(u8::is_ascii_digit))
                        .then(|| std::str::from_utf8(v).ok()?.parse().ok())
                        .flatten()
                }
                b"width" => out.width = Dimension::parse(v),
                b"height" => out.height = Dimension::parse(v),
                b"preserveAspectRatio" => out.preserve_aspect_ratio = v != b"0",
                b"doNotMoveCursor" => out.do_not_move_cursor = flag(v),
                _ => {}
            }
        }
        out
    }
}

/// Split the text after `File=` into its arguments and its base64 data at
/// the first `:`. `None` if there is no `:` yet.
pub fn split(body: &[u8]) -> Option<(&[u8], &[u8])> {
    let colon = body.iter().position(|&b| b == b':')?;
    Some((&body[..colon], &body[colon + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imgcat() {
        // What iTerm2's imgcat sends.
        let a = Args::parse(b"name=cGhvdG8uanBn;size=1234567;inline=1");
        assert!(a.inline);
        assert_eq!(a.name.as_deref(), Some(&b"photo.jpg"[..]));
        assert_eq!(a.size, Some(1_234_567));
        assert_eq!((a.width, a.height), (Dimension::Auto, Dimension::Auto));
        assert!(a.preserve_aspect_ratio);
        assert!(!a.do_not_move_cursor);
    }

    #[test]
    fn every_key_and_size_form() {
        let a = Args::parse(b"inline=1;width=40;height=50%;preserveAspectRatio=0;doNotMoveCursor=1");
        assert_eq!(a.width, Dimension::Cells(40));
        assert_eq!(a.height, Dimension::Percent(50));
        assert!(!a.preserve_aspect_ratio);
        assert!(a.do_not_move_cursor);
        let a = Args::parse(b"width=800px;height=auto");
        assert_eq!((a.width, a.height), (Dimension::Pixels(800), Dimension::Auto));
        assert!(!a.inline);
        let a = Args::parse(b"width=AUTO;height=0");
        assert_eq!((a.width, a.height), (Dimension::Auto, Dimension::Cells(0)));
        // Unreadable values fall back to the default.
        for v in [
            &b"width=-3"[..],
            b"width=3em",
            b"width=",
            b"width=px",
            b"width=%",
            b"width=99999999999",
        ] {
            assert_eq!(Args::parse(v).width, Dimension::Auto, "{:?}", v.escape_ascii());
        }
        assert_eq!(Args::parse(b"size=12a").size, None);
        assert_eq!(Args::parse(b"size=99999999999999999999").size, None);
        assert!(!Args::parse(b"inline=0").inline);
        assert!(!Args::parse(b"inline=").inline);
        // Keys are case-sensitive.
        assert!(!Args::parse(b"Inline=1").inline);
        // Repeats: the last one wins.
        assert_eq!(Args::parse(b"width=1;width=2").width, Dimension::Cells(2));
    }

    #[test]
    fn names() {
        assert_eq!(Args::parse(b"name=").name.as_deref(), Some(&b""[..]));
        assert_eq!(Args::parse(b"name=!!!").name, None);
        // UTF-8 names, unpadded.
        assert_eq!(Args::parse(b"name=w6kuZ2lm").name.as_deref(), Some("é.gif".as_bytes()));
        let big = format!("name={}", "QUFB".repeat(1000));
        assert_eq!(Args::parse(big.as_bytes()).name.unwrap().len(), MAX_NAME);
    }

    #[test]
    fn garbage() {
        assert_eq!(Args::parse(b""), Args::default());
        assert_eq!(Args::parse(b";;;==;=;inline"), Args::default());
        assert_eq!(Args::parse(b"\xff\xfe=\x00;width=\xff"), Args::default());
        let huge = b"x=1;".repeat(5000);
        assert_eq!(Args::parse(&huge), Args::default());
    }

    #[test]
    fn split_at_the_colon() {
        assert_eq!(split(b"inline=1:AAAA"), Some((&b"inline=1"[..], &b"AAAA"[..])));
        assert_eq!(split(b"inline=1;width=2:"), Some((&b"inline=1;width=2"[..], &b""[..])));
        assert_eq!(split(b"inline=1"), None);
    }
}
