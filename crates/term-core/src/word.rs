//! What a word is, for double-click selection (and, later, whole-word
//! search): a run of characters that are neither whitespace nor one of a
//! few brackets, quotes and separators. Paths, URLs and `file:line:col`
//! stay whole.

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
}
