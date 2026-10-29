//! What a word is, for double-click selection (and, later, whole-word
//! search): a run of characters that are neither whitespace nor one of a
//! few brackets, quotes and separators. Paths, URLs and `file:line:col`
//! stay whole. Also what a plain URL is, for links without OSC 8
//! ([`url_at`]).

/// Characters that end a word (besides whitespace).
const WORD_BREAKS: &str = "()[]{}<>'\"`|,;";

pub fn is_word_char(c: char) -> bool {
    !c.is_whitespace() && !WORD_BREAKS.contains(c)
}

/// The indices (inclusive) of the word around `i` in `text`, or `None` when
/// `text[i]` isn't part of a word.
pub fn word_at(text: &[char], i: usize) -> Option<(usize, usize)> {
    let is_word = |c: usize| text.get(c).copied().is_some_and(is_word_char);
    if !is_word(i) {
        return None;
    }
    let mut a = i;
    while a > 0 && is_word(a - 1) {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < text.len() && is_word(b + 1) {
        b += 1;
    }
    Some((a, b))
}

/// The schemes a plain URL is found by. Others aren't looked for: they
/// would be refused when opened anyway.
const URL_SCHEMES: [&str; 4] = ["https://", "http://", "file://", "mailto:"];

/// Characters that can't be part of a plain URL (besides whitespace and
/// controls): what RFC 3986 leaves out and people put around URLs.
const URL_BREAKS: &str = "<>\"'`{}|\\^";

fn is_url_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !URL_BREAKS.contains(c)
}

/// The indices (inclusive) of the plain URL around `i` in `text`, or
/// `None`. A URL starts with one of [`URL_SCHEMES`] (not in the middle of
/// a word) and runs to the next space or character no URL holds; a
/// trailing `.`, `,`, `:`, `;`, `!` or `?`, and closing brackets with no
/// opening one in the URL, are left out, as is usual in prose and
/// Markdown: `(see https://x.y/a_(b))` gives `https://x.y/a_(b)`.
pub fn url_at(text: &[char], i: usize) -> Option<(usize, usize)> {
    if !text.get(i).copied().is_some_and(is_url_char) {
        return None;
    }
    let mut a = i;
    while a > 0 && is_url_char(text[a - 1]) {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < text.len() && is_url_char(text[b + 1]) {
        b += 1;
    }
    // The first scheme that starts at or before `i`, not after a letter
    // or digit (`xhttp://` isn't one), so a URL holding another is whole.
    let starts = |s: usize| {
        URL_SCHEMES.iter().find(|scheme| {
            let n = scheme.chars().count();
            s + n <= b + 1 && text[s..s + n].iter().zip(scheme.chars()).all(|(&c, k)| c.to_ascii_lowercase() == k)
        })
    };
    let (start, scheme) = (a..=i)
        .filter(|&s| s == a || !text[s - 1].is_alphanumeric())
        .find_map(|s| starts(s).map(|k| (s, k.chars().count())))?;
    let mut end = b;
    loop {
        let c = text[end];
        let open = match c {
            ')' => Some('('),
            ']' => Some('['),
            _ => None,
        };
        let unbalanced = open.is_some_and(|o| {
            let span = &text[start..=end];
            span.iter().filter(|&&x| x == o).count() < span.iter().filter(|&&x| x == c).count()
        });
        if end > start && (".,:;!?".contains(c) || unbalanced) {
            end -= 1;
        } else {
            break;
        }
    }
    // Something must follow the scheme, and `i` must still be in it.
    (end >= start + scheme && i <= end).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_click_words() {
        type Row = (&'static str, usize, Option<(usize, usize)>);
        let rows: &[Row] = &[
            ("hello world", 0, Some((0, 4))),
            ("hello world", 4, Some((0, 4))),
            ("hello world", 5, None),
            ("hello world", 6, Some((6, 10))),
            ("hello world", 10, Some((6, 10))),
            ("hello world", 30, None),
            ("(foo)", 0, None),
            ("(foo)", 2, Some((1, 3))),
            ("a,b;c", 2, Some((2, 2))),
            ("say \"hi\" `x` 'y' <z> [w] {v} |u|", 6, Some((5, 6))),
            ("src/main.rs:12:5", 3, Some((0, 15))),
            ("https://x.y/?a=b&c", 0, Some((0, 17))),
            ("tab\tsep", 4, Some((4, 6))),
            ("", 0, None),
        ];
        for &(line, col, want) in rows {
            let chars: Vec<char> = line.chars().collect();
            assert_eq!(word_at(&chars, col), want, "{line:?} at {col}");
        }
    }

    #[test]
    fn plain_urls() {
        type Case = (&'static str, usize, Option<&'static str>);
        let cases: &[Case] = &[
            ("see https://example.com/a?b=c#d now", 4, Some("https://example.com/a?b=c#d")),
            ("see https://example.com/a?b=c#d now", 20, Some("https://example.com/a?b=c#d")),
            ("see https://example.com now", 0, None),
            ("see https://example.com now", 3, None),
            ("http://x.y.", 0, Some("http://x.y")),
            ("go to http://x.y/z, then", 8, Some("http://x.y/z")),
            ("(see https://x.y/a_(b))", 6, Some("https://x.y/a_(b)")),
            ("[link](https://x.y/p)", 10, Some("https://x.y/p")),
            ("[link](https://x.y/p)", 2, None),
            ("<https://x.y/>", 3, Some("https://x.y/")),
            ("\"file:///tmp/a b\"", 3, Some("file:///tmp/a")),
            ("mailto:me@x.y?subject=hi", 9, Some("mailto:me@x.y?subject=hi")),
            ("HTTPS://X.Y", 2, Some("HTTPS://X.Y")),
            ("xhttp://x.y", 3, None),
            ("ftp://x.y", 2, None),
            ("https://", 2, None),
            ("https://.", 2, None),
            ("a=https://x.y", 5, Some("https://x.y")),
            ("a=https://x.y", 0, None),
            ("https://a.b/https://c.d", 15, Some("https://a.b/https://c.d")),
            ("", 0, None),
        ];
        for &(line, col, want) in cases {
            let chars: Vec<char> = line.chars().collect();
            let got = url_at(&chars, col).map(|(a, b)| chars[a..=b].iter().collect::<String>());
            assert_eq!(got.as_deref(), want, "{line:?} at {col}");
        }
    }
}
