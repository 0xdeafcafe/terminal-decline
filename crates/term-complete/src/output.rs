//! Plain command output coloured (the `colour_output` setting,
//! docs/design/command-marks.md §3): commit hashes, URLs, arrows,
//! numbers and a few words, found in one row's text. Only cells the
//! program left plain are coloured; [`super::marks`] says which rows are
//! output and keeps the answers per row.

use std::ops::Range;

const RED: u8 = 1;
const GREEN: u8 = 2;
const YELLOW: u8 = 3;
const BLUE: u8 = 4;
const MAGENTA: u8 = 5;
const CYAN: u8 = 6;

/// A word's colour, if it is one of the words coloured: as written,
/// capitalised or in capitals.
fn word_color(w: &str) -> Option<u8> {
    const WORDS: [(&str, u8); 12] = [
        ("error", RED),
        ("fatal", RED),
        ("failed", RED),
        ("panic", RED),
        ("denied", RED),
        ("warning", YELLOW),
        ("warn", YELLOW),
        ("deprecated", YELLOW),
        ("ok", GREEN),
        ("done", GREEN),
        ("passed", GREEN),
        ("success", GREEN),
    ];
    let lower = w.to_ascii_lowercase();
    let (word, color) = WORDS.iter().find(|(word, _)| *word == lower)?;
    let capitalised = w.len() > 1 && w[..1].bytes().all(|b| b.is_ascii_uppercase()) && w[1..] == word[1..];
    (w == *word || w == word.to_ascii_uppercase() || capitalised).then_some(*color)
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// A commit hash: 7 to 40 hex digits, at least one a digit and one a
/// letter.
fn is_hash(w: &[char]) -> bool {
    (7..=40).contains(&w.len())
        && w.iter().all(char::is_ascii_hexdigit)
        && w.iter().any(char::is_ascii_digit)
        && w.iter().any(|c| c.is_ascii_alphabetic())
}

/// Where a URL starting at `i` ends: a scheme from a short list, `://`,
/// then anything but spaces and quotes, without trailing punctuation.
fn url_end(text: &[char], i: usize) -> Option<usize> {
    const SCHEMES: [&str; 5] = ["https://", "http://", "file://", "ftp://", "ssh://"];
    if i > 0 && is_word_char(text[i - 1]) {
        return None;
    }
    let scheme = SCHEMES.iter().find(|s| {
        let n = s.len();
        text.len() >= i + n && text[i..i + n].iter().copied().eq(s.chars())
    })?;
    let mut end = i + scheme.len();
    while end < text.len() && !text[end].is_whitespace() && !matches!(text[end], '"' | '\'' | '<' | '>' | '`' | '\0') {
        end += 1;
    }
    while end > i + scheme.len() && matches!(text[end - 1], '.' | ',' | ';' | ':' | ')' | ']' | '}' | '!' | '?') {
        end -= 1;
    }
    (end > i + scheme.len()).then_some(end)
}

/// Where a number starting at `i` (a digit after a non-word character)
/// ends: digits, `.` and more digits, then unit letters (up to three) or
/// `%`, at the end of a word. A bare number isn't one: sizes, counts,
/// dates and times in listings (`ls -l`) are everywhere, and colouring
/// them all reads as noise. `None` if there's no unit or the word goes on.
fn number_end(text: &[char], i: usize) -> Option<usize> {
    let digits = |mut j: usize| {
        while j < text.len() && text[j].is_ascii_digit() {
            j += 1;
        }
        j
    };
    let mut end = digits(i);
    while end + 1 < text.len() && text[end] == '.' && text[end + 1].is_ascii_digit() {
        end = digits(end + 1);
    }
    let letters = (end..text.len()).take_while(|&j| text[j].is_ascii_alphabetic()).count();
    if letters > 0 && letters <= 3 {
        end += letters;
    } else if letters == 0 && end < text.len() && text[end] == '%' {
        return Some(end + 1);
    } else {
        return None;
    }
    (end == text.len() || !is_word_char(text[end])).then_some(end)
}

/// The colours in one row's text, one char per cell, as cell ranges.
/// `plain` says which cells the program left in the default colour with
/// no attributes: a stretch is coloured only if all its cells are.
pub fn row_colors(text: &[char], plain: &[bool], out: &mut Vec<(Range<u16>, u8)>) {
    let n = text.len();
    let mut push = |a: usize, b: usize, color: u8| {
        if plain.get(a..b).is_some_and(|p| p.iter().all(|&x| x)) {
            out.push((a as u16..b as u16, color));
        }
    };
    let mut i = 0;
    while i < n {
        let c = text[i];
        if let Some(end) = url_end(text, i) {
            push(i, end, BLUE);
            i = end;
            continue;
        }
        if (c == '-' || c == '=') && text.get(i + 1) == Some(&'>') {
            push(i, i + 2, MAGENTA);
            i += 2;
            continue;
        }
        if !is_word_char(c) {
            i += 1;
            continue;
        }
        // A word, from a word boundary.
        let end = (i..n).find(|&j| !is_word_char(text[j])).unwrap_or(n);
        let word = &text[i..end];
        if is_hash(word) {
            push(i, end, YELLOW);
        } else if c.is_ascii_digit()
            && let Some(e) = number_end(text, i)
        {
            push(i, e, CYAN);
            i = e;
            continue;
        } else if word.len() <= 10 {
            let s: String = word.iter().collect();
            if let Some(color) = word_color(&s) {
                push(i, end, color);
            }
        }
        i = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(line: &str) -> Vec<(String, u8)> {
        let text: Vec<char> = line.chars().collect();
        colors_plain(line, &vec![true; text.len()])
    }

    fn colors_plain(line: &str, plain: &[bool]) -> Vec<(String, u8)> {
        let text: Vec<char> = line.chars().collect();
        let mut out = Vec::new();
        row_colors(&text, plain, &mut out);
        out.into_iter()
            .map(|(r, c)| (text[r.start as usize..r.end as usize].iter().collect(), c))
            .collect()
    }

    fn s(w: &str, c: u8) -> (String, u8) {
        (w.to_string(), c)
    }

    #[test]
    fn commit_hashes_but_not_words_or_plain_numbers() {
        assert_eq!(colors("commit 3f2a9c1 on main"), [s("3f2a9c1", YELLOW)]);
        assert_eq!(
            colors("abc1234def5678abc1234def5678abc1234def56"),
            [s("abc1234def5678abc1234def5678abc1234def56", YELLOW)]
        );
        // No digit: a word. No letter: a number. Too short or too long: not a hash.
        assert_eq!(colors("deadbeef cafebabe"), []);
        assert_eq!(colors("1234567"), []);
        assert_eq!(colors("ab12cd"), []);
        assert_eq!(colors(&"a1".repeat(21)), []);
        // Only as a whole word.
        assert_eq!(colors("x3f2a9c1 3f2a9c1x"), []);
    }

    #[test]
    fn numbers_with_units_and_percentages() {
        assert_eq!(
            colors("took 12ms, 3.5GB at 99% (v2 x86)"),
            [s("12ms", CYAN), s("3.5GB", CYAN), s("99%", CYAN)]
        );
        // Bare numbers aren't coloured: an `ls -l` line keeps its
        // sizes, counts, dates and times as printed.
        assert_eq!(colors("1.2.3"), []);
        assert_eq!(colors("drwxr-xr-x@  14 lw  staff  448 Nov  1 20:12 .docker"), []);
        assert_eq!(colors(".zcompdump-AFRs-MacBook-Pro-5.9.local.47316"), []);
        // Inside a word, or with a long tail: not a number.
        assert_eq!(colors("abc123 42things"), []);
    }

    #[test]
    fn urls_and_arrows() {
        assert_eq!(
            colors("see https://example.com/a?b=1. a -> b => c"),
            [s("https://example.com/a?b=1", BLUE), s("->", MAGENTA), s("=>", MAGENTA)]
        );
        assert_eq!(colors("(http://x.y/z)"), [s("http://x.y/z", BLUE)]);
    }

    #[test]
    fn words_only_whole() {
        assert_eq!(
            colors("error: build failed, 2 warnings; ok done"),
            [s("error", RED), s("failed", RED), s("ok", GREEN), s("done", GREEN)]
        );
        assert_eq!(
            colors("Error ERROR FAILED Warning"),
            [s("Error", RED), s("ERROR", RED), s("FAILED", RED), s("Warning", YELLOW)]
        );
        assert_eq!(colors("errors terror error_code okay undone eRRor"), []);
        assert_eq!(colors("non-fatal"), [s("fatal", RED)]);
    }

    #[test]
    fn only_over_plain_cells() {
        let line = "error 3f2a9c1";
        let mut plain = vec![true; line.len()];
        // The program coloured `rr` in `error` itself.
        plain[1] = false;
        plain[2] = false;
        assert_eq!(colors_plain(line, &plain), [s("3f2a9c1", YELLOW)]);
    }
}
