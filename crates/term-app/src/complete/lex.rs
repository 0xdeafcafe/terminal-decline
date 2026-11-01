//! Reading a command line into words (docs/design/completion.md §4): a
//! small shell reader, not a parser. It finds the words, quotes,
//! expansions, operators and redirects, which words are commands, and
//! what the cursor is on, for colouring the line and for the completion
//! list. Whether a command or a file exists is for the caller to check.
//!
//! What it doesn't understand (a here-document, `case`) gives no words
//! and [`At::Nothing`], so the line is drawn as it would be without it.

use std::ops::Range;

/// Longer lines aren't read.
pub const MAX_LINE: usize = 4096;

/// Commands that run the command after them: the word after one (and
/// after its options) is a command again.
const PRECOMMANDS: &[&str] = &[
    "sudo",
    "doas",
    "env",
    "time",
    "nohup",
    "exec",
    "command",
    "builtin",
    "noglob",
    "nocorrect",
    "-",
];

/// Reserved words after which a command comes.
const LEADING: &[&str] = &["if", "then", "else", "elif", "while", "until", "do", "!", "{", "coproc"];

/// Reserved words that end a construct.
const CLOSING: &[&str] = &["fi", "done", "}"];

/// What a stretch of the line is, for colouring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Command,
    /// A reserved word: `if`, `for`, `do`, `[[`.
    Keyword,
    Argument,
    Flag,
    /// `NAME=value` before a command.
    Assignment,
    /// Inside quotes.
    String,
    /// `$NAME`, `${…}`, `$(…)`, `` `…` ``: its value isn't known here.
    Variable,
    /// `|`, `&&`, `;`, `(`, a newline.
    Operator,
    /// `>`, `2>&1`, `<<<`.
    Redirect,
    Comment,
}

/// A stretch of the line: byte offsets and what it is. Spans don't
/// overlap and are in order; blanks between them belong to none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: Kind,
}

/// What a word does in its command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Command,
    Keyword,
    Argument,
    Flag,
    Assignment,
    /// The file after a redirect.
    Target,
}

/// A word of the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    /// Byte offsets in the line, quotes included.
    pub range: Range<usize>,
    /// The word with quotes and backslashes taken out; expansions are
    /// kept as typed.
    pub text: String,
    pub role: Role,
    /// The word of the command it belongs to (itself for a command).
    pub command: Option<usize>,
    /// The quote the word is still inside at its end (`"my f`).
    pub open_quote: Option<char>,
    /// It holds an expansion, so its value isn't `text`.
    pub expands: bool,
}

/// What the cursor is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum At {
    /// A command being typed.
    Command { prefix: String },
    /// An argument of `command`: `position` counts the arguments before
    /// it (flags aside), so 0 may be a subcommand (`git ch`).
    Argument {
        command: String,
        position: usize,
        prefix: String,
        quote: Option<char>,
    },
    /// A flag of `command`, after its first argument `subcommand` if any.
    Flag {
        command: String,
        subcommand: Option<String>,
        prefix: String,
    },
    /// A file: after a redirect, after `--flag=` or `NAME=`, in a `for` list
    /// or a `[[ ]]` test.
    Path { prefix: String, quote: Option<char> },
    /// A variable's name, after `$` or `${`.
    Variable { prefix: String },
    /// Nothing to complete: a comment, inside `$(…)`, an operator, or a
    /// line this reader doesn't understand.
    Nothing,
}

/// A command line read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub words: Vec<Word>,
    pub spans: Vec<Span>,
    pub at: At,
}

impl Line {
    fn nothing() -> Line {
        Line {
            words: Vec::new(),
            spans: Vec::new(),
            at: At::Nothing,
        }
    }
}

/// Read `line` with the cursor at byte `cursor` (on a character boundary).
pub fn read(line: &str, cursor: usize) -> Line {
    if line.len() > MAX_LINE {
        return Line::nothing();
    }
    let mut r = Reader {
        line,
        b: line.as_bytes(),
        cursor: cursor.min(line.len()),
        words: Vec::new(),
        spans: Vec::new(),
        state: State::Command,
        target_next: false,
        last_end: 0,
        at: None,
    };
    match r.run() {
        Some(()) => Line {
            at: r.at.unwrap_or(At::Nothing),
            words: r.words,
            spans: r.spans,
        },
        None => Line::nothing(),
    }
}

/// What the next word is.
#[derive(Clone, Copy, Debug)]
enum State {
    Command,
    /// After a precommand (`sudo`): its options, then a command.
    Pre {
        pre: usize,
        value_next: bool,
    },
    Args {
        cmd: usize,
        options_ended: bool,
    },
    /// `for x in …`: words that are neither commands nor arguments.
    List,
    /// Inside `[[ … ]]`.
    Test,
    /// The name after `function`.
    Name,
}

struct Reader<'a> {
    line: &'a str,
    b: &'a [u8],
    cursor: usize,
    words: Vec<Word>,
    spans: Vec<Span>,
    state: State,
    /// The last token was a redirect wanting a file.
    target_next: bool,
    /// Where the last token ended.
    last_end: usize,
    at: Option<At>,
}

/// A word as read, before it is given a role.
#[derive(Default)]
struct Raw {
    text: String,
    /// Quoted and expanded stretches, in order.
    segments: Vec<Span>,
    /// No quotes, escapes or expansions: may be a reserved word.
    plain: bool,
    open_quote: Option<char>,
    expands: bool,
    /// `text`'s length where the cursor is, when it is in the word.
    prefix_len: Option<usize>,
    /// The quote the cursor is inside.
    quote_at_cursor: Option<char>,
    /// The cursor is in a `$name` or `${name`: the name before it.
    variable: Option<String>,
    /// The cursor is inside `$(…)`, `${…}` or `` `…` ``.
    in_expansion: bool,
}

impl Reader<'_> {
    fn run(&mut self) -> Option<()> {
        let b = self.b;
        let mut i = 0;
        while i < b.len() {
            if matches!(b[i], b' ' | b'\t') {
                i += 1;
                continue;
            }
            self.gap(i);
            if b[i] == b'#' {
                let end = memchr(b'\n', &b[i..]).map_or(b.len(), |n| i + n);
                self.spans.push(Span {
                    range: i..end,
                    kind: Kind::Comment,
                });
                if self.cursor > i && self.cursor <= end {
                    self.at.get_or_insert(At::Nothing);
                }
                self.last_end = end;
                i = end;
                continue;
            }
            if let Some((n, wants_file)) = redirect(&b[i..]) {
                let op = &self.line[i..i + n];
                if (op.ends_with("<<") && !op.ends_with("<<<")) || op.ends_with("<<-") {
                    // A here-document: its body follows on later lines.
                    return None;
                }
                self.token(i..i + n, Kind::Redirect);
                self.target_next = wants_file;
                i += n;
                continue;
            }
            if let Some(n) = operator(&b[i..]) {
                self.token(i..i + n, Kind::Operator);
                self.state = State::Command;
                self.target_next = false;
                i += n;
                continue;
            }
            let end = self.word(i)?;
            // Every byte that starts no token above starts a word; never
            // stand still on one that ends it at once.
            i = end.max(i + 1);
        }
        self.gap(usize::MAX);
        Some(())
    }

    /// An operator or redirect at `range`; the cursor strictly inside it
    /// is on nothing.
    fn token(&mut self, range: Range<usize>, kind: Kind) {
        if self.cursor > range.start && self.cursor < range.end {
            self.at.get_or_insert(At::Nothing);
        }
        self.last_end = range.end;
        self.spans.push(Span { range, kind });
    }

    /// The cursor in the blank before a token at `next`: on a word not
    /// typed yet.
    fn gap(&mut self, next: usize) {
        if self.at.is_some() || self.cursor < self.last_end || self.cursor >= next {
            return;
        }
        let empty = String::new;
        let at = if self.target_next {
            At::Path {
                prefix: empty(),
                quote: None,
            }
        } else {
            match self.state {
                State::Command | State::Pre { value_next: false, .. } => At::Command { prefix: empty() },
                State::Pre { pre, value_next: true } => self.argument(pre, empty(), None),
                State::Args { cmd, .. } => self.argument(cmd, empty(), None),
                State::List | State::Test => At::Path {
                    prefix: empty(),
                    quote: None,
                },
                State::Name => At::Nothing,
            }
        };
        self.at = Some(at);
    }

    /// The cursor on an argument of the command at word `cmd`.
    fn argument(&self, cmd: usize, prefix: String, quote: Option<char>) -> At {
        At::Argument {
            command: self.words[cmd].text.clone(),
            position: self.arguments_of(cmd).count(),
            prefix,
            quote,
        }
    }

    fn arguments_of(&self, cmd: usize) -> impl Iterator<Item = &Word> {
        self.words
            .iter()
            .filter(move |w| w.role == Role::Argument && w.command == Some(cmd))
    }

    /// Read the word at `start`, give it its role and spans; where it ends.
    fn word(&mut self, start: usize) -> Option<usize> {
        let (end, raw) = self.read_word(start);
        let (role, command) = self.classify(&raw)?;
        let index = self.words.len();
        let kind = match role {
            Role::Command => Kind::Command,
            Role::Keyword => Kind::Keyword,
            Role::Argument | Role::Target => Kind::Argument,
            Role::Flag => Kind::Flag,
            Role::Assignment => Kind::Assignment,
        };
        let mut from = start;
        for seg in &raw.segments {
            if seg.range.start > from {
                self.spans.push(Span {
                    range: from..seg.range.start,
                    kind,
                });
            }
            self.spans.push(seg.clone());
            from = seg.range.end;
        }
        if end > from {
            self.spans.push(Span { range: from..end, kind });
        }
        if self.at.is_none() && self.cursor >= start && self.cursor <= end {
            self.at = Some(self.at_word(&raw, role, command.unwrap_or(index)));
        }
        self.words.push(Word {
            range: start..end,
            text: raw.text,
            role,
            command: command.or((role == Role::Command).then_some(index)),
            open_quote: raw.open_quote,
            expands: raw.expands,
        });
        self.last_end = end;
        Some(end)
    }

    /// What the cursor on this word is on.
    fn at_word(&self, raw: &Raw, role: Role, cmd: usize) -> At {
        if let Some(prefix) = &raw.variable {
            return At::Variable { prefix: prefix.clone() };
        }
        if raw.in_expansion {
            return At::Nothing;
        }
        let prefix = raw.text[..raw.prefix_len.unwrap_or(raw.text.len())].to_string();
        let quote = raw.quote_at_cursor;
        // A lone `-` is stdin to the command, but being typed it starts a flag.
        let role = if role == Role::Argument && prefix == "-" && raw.plain {
            Role::Flag
        } else {
            role
        };
        match role {
            Role::Command | Role::Keyword => At::Command { prefix },
            Role::Argument if cmd < self.words.len() && self.words[cmd].role == Role::Command => self.argument(cmd, prefix, quote),
            Role::Argument | Role::Target => At::Path { prefix, quote },
            Role::Flag => match prefix.split_once('=') {
                // `--file=pa`: the value is a file.
                Some((flag, value)) if flag.starts_with("--") => At::Path {
                    prefix: value.to_string(),
                    quote,
                },
                _ => At::Flag {
                    command: self.words.get(cmd).map_or_else(String::new, |w| w.text.clone()),
                    subcommand: self.arguments_of(cmd).next().map(|w| w.text.clone()),
                    prefix,
                },
            },
            Role::Assignment => match prefix.split_once('=') {
                Some((_, value)) => At::Path {
                    prefix: value.to_string(),
                    quote,
                },
                None => At::Nothing,
            },
        }
    }

    /// The word's role and its command's word, moving the state on; `None`
    /// for a construct this reader doesn't handle.
    fn classify(&mut self, raw: &Raw) -> Option<(Role, Option<usize>)> {
        let index = self.words.len();
        let text = raw.text.as_str();
        let reserved = |words: &[&str]| raw.plain && words.contains(&text);
        if std::mem::take(&mut self.target_next) {
            let cmd = match self.state {
                State::Args { cmd, .. } => Some(cmd),
                _ => None,
            };
            return Some((Role::Target, cmd));
        }
        match self.state {
            State::Pre { pre, value_next: true } => {
                self.state = State::Pre { pre, value_next: false };
                Some((Role::Argument, Some(pre)))
            }
            State::Pre { pre, .. } if self.words[pre].text != "-" && text.starts_with('-') => {
                let value_next = takes_value(&self.words[pre].text, text);
                self.state = State::Pre { pre, value_next };
                Some((Role::Flag, Some(pre)))
            }
            State::Pre { pre, .. } if self.words[pre].text == "env" && is_assignment(text) => Some((Role::Assignment, None)),
            State::Command | State::Pre { .. } => {
                if reserved(&["case"]) {
                    return None;
                }
                if reserved(LEADING) || reserved(CLOSING) {
                    self.state = State::Command;
                    return Some((Role::Keyword, None));
                }
                if reserved(&["for", "select", "foreach", "repeat"]) {
                    self.state = State::List;
                    return Some((Role::Keyword, None));
                }
                if reserved(&["[["]) {
                    self.state = State::Test;
                    return Some((Role::Keyword, None));
                }
                if reserved(&["function"]) {
                    self.state = State::Name;
                    return Some((Role::Keyword, None));
                }
                if is_assignment(text) {
                    self.state = State::Command;
                    return Some((Role::Assignment, None));
                }
                self.state = if raw.plain && PRECOMMANDS.contains(&text) {
                    State::Pre {
                        pre: index,
                        value_next: false,
                    }
                } else {
                    State::Args {
                        cmd: index,
                        options_ended: false,
                    }
                };
                Some((Role::Command, None))
            }
            State::Args { cmd, options_ended } => {
                if !options_ended && raw.plain && text == "--" {
                    self.state = State::Args { cmd, options_ended: true };
                    Some((Role::Flag, Some(cmd)))
                } else if !options_ended && text.len() > 1 && text.starts_with('-') {
                    Some((Role::Flag, Some(cmd)))
                } else {
                    Some((Role::Argument, Some(cmd)))
                }
            }
            State::List => {
                if reserved(&["do"]) {
                    self.state = State::Command;
                    Some((Role::Keyword, None))
                } else if reserved(&["in"]) {
                    Some((Role::Keyword, None))
                } else {
                    Some((Role::Argument, None))
                }
            }
            State::Test => {
                if reserved(&["]]"]) {
                    self.state = State::List;
                    Some((Role::Keyword, None))
                } else if text.len() > 1 && text.starts_with('-') {
                    Some((Role::Flag, None))
                } else {
                    Some((Role::Argument, None))
                }
            }
            State::Name => {
                self.state = State::Command;
                Some((Role::Argument, None))
            }
        }
    }

    /// Read the word starting at `start`: where it ends, and what it holds.
    fn read_word(&self, start: usize) -> (usize, Raw) {
        let (b, line, cursor) = (self.b, self.line, self.cursor);
        let mut raw = Raw {
            plain: true,
            ..Raw::default()
        };
        let mut i = start;
        loop {
            if i == cursor && raw.prefix_len.is_none() {
                raw.prefix_len = Some(raw.text.len());
            }
            let Some(&c) = b.get(i) else { break };
            match c {
                b'<' | b'>' if b.get(i + 1) == Some(&b'(') => {
                    // `<(…)`, `>(…)`: process substitution, a file name
                    // whose value isn't known here.
                    let (end, closed) = balanced(b, i + 1, b'(', b')');
                    if cursor > i && (cursor < end || !closed) {
                        raw.in_expansion = true;
                    }
                    raw.plain = false;
                    raw.expands = true;
                    raw.text.push_str(&line[i..end]);
                    raw.segments.push(Span {
                        range: i..end,
                        kind: Kind::Variable,
                    });
                    i = end;
                }
                b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b'(' | b')' | b'<' | b'>' => break,
                b'\\' => {
                    raw.plain = false;
                    match line[i + 1..].chars().next() {
                        Some(ch) => {
                            if ch != '\n' {
                                raw.text.push(ch);
                            }
                            i += 1 + ch.len_utf8();
                        }
                        None => i += 1,
                    }
                }
                b'\'' => {
                    raw.plain = false;
                    let from = i;
                    i += 1;
                    loop {
                        if i == cursor && raw.prefix_len.is_none() {
                            raw.prefix_len = Some(raw.text.len());
                            raw.quote_at_cursor = Some('\'');
                        }
                        let Some(ch) = line[i..].chars().next() else {
                            raw.open_quote = Some('\'');
                            break;
                        };
                        i += ch.len_utf8();
                        if ch == '\'' {
                            break;
                        }
                        raw.text.push(ch);
                    }
                    raw.segments.push(Span {
                        range: from..i,
                        kind: Kind::String,
                    });
                }
                b'"' => {
                    raw.plain = false;
                    i = self.double_quoted(i, &mut raw);
                }
                b'$' | b'`' => {
                    let from = i;
                    i = self.expansion(i, &mut raw);
                    if i == from + 1 && c == b'$' {
                        // A `$` on its own is just a dollar sign.
                        raw.text.push('$');
                    }
                }
                _ => {
                    let ch = line[i..].chars().next().unwrap_or(char::REPLACEMENT_CHARACTER);
                    raw.text.push(ch);
                    i += ch.len_utf8();
                }
            }
        }
        if i == cursor && raw.prefix_len.is_none() {
            raw.prefix_len = Some(raw.text.len());
        }
        (i, raw)
    }

    /// `"…"` from `start`: where it ends. Expansions inside are their own
    /// spans; `\` escapes `"`, `$`, `` ` ``, `\` and a newline.
    fn double_quoted(&self, start: usize, raw: &mut Raw) -> usize {
        let (b, line, cursor) = (self.b, self.line, self.cursor);
        let mut from = start;
        let mut i = start + 1;
        loop {
            if i == cursor && raw.prefix_len.is_none() {
                raw.prefix_len = Some(raw.text.len());
                raw.quote_at_cursor = Some('"');
            }
            let Some(&c) = b.get(i) else {
                raw.open_quote = Some('"');
                break;
            };
            match c {
                b'"' => {
                    i += 1;
                    break;
                }
                b'\\' if matches!(b.get(i + 1), Some(b'"' | b'$' | b'`' | b'\\' | b'\n')) => {
                    if b[i + 1] != b'\n' {
                        raw.text.push(b[i + 1] as char);
                    }
                    i += 2;
                }
                b'$' | b'`' => {
                    if i > from {
                        raw.segments.push(Span {
                            range: from..i,
                            kind: Kind::String,
                        });
                    }
                    let at = i;
                    i = self.expansion(i, raw);
                    if i == at + 1 && c == b'$' {
                        raw.text.push('$');
                        // The lone `$` stays part of the string.
                        from = at;
                    } else {
                        from = i;
                    }
                }
                _ => {
                    let ch = line[i..].chars().next().unwrap_or(char::REPLACEMENT_CHARACTER);
                    raw.text.push(ch);
                    i += ch.len_utf8();
                }
            }
        }
        if i > from {
            raw.segments.push(Span {
                range: from..i,
                kind: Kind::String,
            });
        }
        i
    }

    /// An expansion at `start` (`$`, or `` ` ``): where it ends. Adds its
    /// span and its text as typed. A `$` followed by nothing it can start
    /// ends at once, and is the caller's to treat as a dollar sign.
    fn expansion(&self, start: usize, raw: &mut Raw) -> usize {
        let (b, cursor) = (self.b, self.cursor);
        let (end, closed, kind) = match (b[start], b.get(start + 1)) {
            (b'`', _) => {
                let mut i = start + 1;
                while i < b.len() && b[i] != b'`' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                let closed = i < b.len();
                (if closed { i + 1 } else { b.len() }, closed, Kind::Variable)
            }
            (_, Some(b'\'')) => {
                // `$'…'`: a string with C escapes.
                let mut i = start + 2;
                while i < b.len() && b[i] != b'\'' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                let closed = i < b.len();
                let end = if closed { i + 1 } else { b.len() };
                if !closed {
                    raw.open_quote = Some('\'');
                }
                if cursor > start + 1 && (cursor < end || !closed) && raw.prefix_len.is_none() {
                    raw.prefix_len = Some(raw.text.len() + self.line[start + 2..cursor.min(b.len())].len());
                    raw.quote_at_cursor = Some('\'');
                }
                raw.text.push_str(&self.line[start + 2..if closed { end - 1 } else { end }]);
                raw.plain = false;
                raw.segments.push(Span {
                    range: start..end.min(b.len()),
                    kind: Kind::String,
                });
                return end.min(b.len());
            }
            (_, Some(&open @ (b'(' | b'{'))) => {
                let close = if open == b'(' { b')' } else { b'}' };
                let (end, closed) = balanced(b, start + 1, open, close);
                if open == b'{' && cursor > start + 1 && (cursor < end || !closed) {
                    // `${na`: a variable's name, if that's all there is so far.
                    let name = &self.line[start + 2..cursor.min(end)];
                    if name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
                        raw.variable = Some(name.to_string());
                    }
                }
                (end, closed, Kind::Variable)
            }
            (_, Some(c)) if c.is_ascii_alphabetic() || *c == b'_' => {
                let mut i = start + 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                if cursor > start && cursor <= i {
                    raw.variable = Some(self.line[start + 1..cursor].to_string());
                }
                (i, true, Kind::Variable)
            }
            (_, Some(b'?' | b'$' | b'#' | b'!' | b'@' | b'*' | b'-' | b'0'..=b'9')) => (start + 2, true, Kind::Variable),
            _ => return start + 1,
        };
        if cursor > start && (cursor < end || !closed) && raw.variable.is_none() {
            raw.in_expansion = true;
        }
        raw.plain = false;
        raw.expands = true;
        raw.text.push_str(&self.line[start..end]);
        raw.segments.push(Span { range: start..end, kind });
        end
    }
}

/// From the opening bracket at `open_at`, where the matching close ends
/// (past it), and whether it was found; quotes inside are skipped.
fn balanced(b: &[u8], open_at: usize, open: u8, close: u8) -> (usize, bool) {
    let mut depth = 0usize;
    let mut i = open_at;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'\'' => i = memchr(b'\'', &b[i + 1..]).map_or(b.len(), |n| i + 1 + n),
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return (i + 1, true);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (b.len(), false)
}

/// The length of the operator at the start of `b`, if one is there.
fn operator(b: &[u8]) -> Option<usize> {
    let two = b.get(..2);
    if matches!(two, Some(b"||" | b"|&" | b"&&" | b"&!" | b"&|" | b";;" | b";&" | b";|")) {
        return Some(2);
    }
    matches!(b.first(), Some(b'|' | b'&' | b';' | b'(' | b')' | b'\n')).then_some(1)
}

/// The length of the redirect at the start of `b`, if one is there, and
/// whether a file comes after it (`2>&1` and `>&-` need none).
fn redirect(b: &[u8]) -> Option<(usize, bool)> {
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 && b.first() == Some(&b'&') && b.get(1) == Some(&b'>') {
        // `&>`, `&>>`: both outputs.
        i = 1;
    }
    let op: &[u8] = match &b[i..] {
        [b'<', b'<', b'<', ..] => b"<<<",
        [b'<', b'<', b'-', ..] => b"<<-",
        [b'<', b'<', ..] => b"<<",
        [b'<', b'>', ..] => b"<>",
        [b'<', b'&', ..] => b"<&",
        [b'>', b'>', b'|', ..] | [b'>', b'>', b'!', ..] => b">>|",
        [b'>', b'>', ..] => b">>",
        [b'>', b'|', ..] | [b'>', b'!', ..] => b">|",
        [b'>', b'&', ..] => b">&",
        [b'<', ..] => b"<",
        [b'>', ..] => b">",
        _ => return None,
    };
    if b.get(i + 1) == Some(&b'(') && op.len() == 1 {
        // `<(…)` and `>(…)`: process substitution, a word.
        return None;
    }
    i += op.len();
    if op.ends_with(b"&") {
        // `2>&1`, `>&-`: duplicated or closed, no file.
        let n = b[i..].iter().take_while(|c| c.is_ascii_digit() || **c == b'-').count();
        if n > 0 {
            return Some((i + n, false));
        }
    }
    Some((i, true))
}

/// `NAME=value` or `NAME+=value`.
fn is_assignment(text: &str) -> bool {
    let Some(eq) = text.find('=') else { return false };
    let name = text[..eq].strip_suffix('+').unwrap_or(&text[..eq]);
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A precommand's options that take a value: `sudo -u root make`.
fn takes_value(pre: &str, flag: &str) -> bool {
    match pre {
        "sudo" => matches!(flag, "-u" | "-g" | "-C" | "-D" | "-h" | "-p" | "-R" | "-T" | "-U"),
        "doas" => matches!(flag, "-u" | "-C"),
        "env" => matches!(flag, "-u" | "-C" | "-S" | "-P"),
        _ => false,
    }
}

fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|&c| c == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `line` with `▏` where the cursor is.
    fn at(line: &str) -> At {
        let cursor = line.find('▏').expect("a cursor");
        let line = line.replacen('▏', "", 1);
        read(&line, cursor).at
    }

    fn command(prefix: &str) -> At {
        At::Command { prefix: prefix.into() }
    }

    fn arg(command: &str, position: usize, prefix: &str) -> At {
        At::Argument {
            command: command.into(),
            position,
            prefix: prefix.into(),
            quote: None,
        }
    }

    fn flag(command: &str, subcommand: Option<&str>, prefix: &str) -> At {
        At::Flag {
            command: command.into(),
            subcommand: subcommand.map(Into::into),
            prefix: prefix.into(),
        }
    }

    fn path(prefix: &str) -> At {
        At::Path {
            prefix: prefix.into(),
            quote: None,
        }
    }

    /// The line's spans as `kind:text` pairs.
    fn spans(line: &str) -> Vec<String> {
        let l = read(line, line.len());
        l.spans.iter().map(|s| format!("{:?}:{}", s.kind, &line[s.range.clone()])).collect()
    }

    #[test]
    fn what_the_cursor_is_on() {
        for (line, want) in [
            ("▏", command("")),
            ("gi▏", command("gi")),
            ("g▏it", command("g")),
            ("git ▏", arg("git", 0, "")),
            ("git ch▏", arg("git", 0, "ch")),
            ("git checkout ma▏", arg("git", 1, "ma")),
            ("git checkout -▏", flag("git", Some("checkout"), "-")),
            ("git checkout --det▏", flag("git", Some("checkout"), "--det")),
            ("ls -l▏", flag("ls", None, "-l")),
            ("ls -l ▏", arg("ls", 0, "")),
            ("ls -- -l▏", arg("ls", 0, "-l")),
            ("ls - ▏", arg("ls", 1, "")),
            ("ls -▏", flag("ls", None, "-")),
            ("diff <(ls -▏", At::Nothing),
            ("ls | gr▏", command("gr")),
            ("ls |gr▏", command("gr")),
            ("ls |▏", command("")),
            ("ls && ▏", command("")),
            ("a; b▏", command("b")),
            ("(cd x▏", arg("cd", 0, "x")),
            ("echo hi > ou▏", path("ou")),
            ("echo hi >▏", path("")),
            ("echo hi 2>&1 ▏", arg("echo", 1, "")),
            ("cat < in▏", path("in")),
            ("make --file=Ma▏", path("Ma")),
            ("CC=clang ma▏", command("ma")),
            ("CC=cl▏", path("cl")),
            ("sudo ma▏", command("ma")),
            ("sudo -u root ma▏", command("ma")),
            ("sudo -u ro▏", arg("sudo", 0, "ro")),
            ("sudo -▏", flag("sudo", None, "-")),
            ("env FOO=1 BAR=2 py▏", command("py")),
            ("time nohup ca▏", command("ca")),
            ("if tru▏", command("tru")),
            ("if true; then ech▏", command("ech")),
            ("for f in *.t▏", path("*.t")),
            ("for f in a; do ec▏", command("ec")),
            ("[[ -f ./x▏", path("./x")),
            ("echo $HO▏", At::Variable { prefix: "HO".into() }),
            ("echo ${HO▏", At::Variable { prefix: "HO".into() }),
            ("echo \"$HO▏\"", At::Variable { prefix: "HO".into() }),
            ("echo $(ls -▏)", At::Nothing),
            ("echo `ls▏`", At::Nothing),
            ("ls # a comm▏", At::Nothing),
            ("ls |▏| x", At::Nothing),
            ("ls &▏& x", At::Nothing),
        ] {
            assert_eq!(at(line), want, "{line}");
        }
    }

    #[test]
    fn quotes_and_escapes_in_prefixes() {
        assert_eq!(
            at("cat \"my f▏"),
            At::Argument {
                command: "cat".into(),
                position: 0,
                prefix: "my f".into(),
                quote: Some('"'),
            }
        );
        assert_eq!(
            at("cat 'it''s ▏"),
            At::Argument {
                command: "cat".into(),
                position: 0,
                prefix: "its ".into(),
                quote: Some('\''),
            }
        );
        assert_eq!(at("cat my\\ f▏"), arg("cat", 0, "my f"));
        assert_eq!(at("cat ~/Doc▏"), arg("cat", 0, "~/Doc"));
        assert_eq!(at("cat $HOME/Doc▏"), arg("cat", 0, "$HOME/Doc"));
        assert_eq!(at("cat \"a b\"/c▏"), arg("cat", 0, "a b/c"));
        assert_eq!(at("cat é/ü▏"), arg("cat", 0, "é/ü"));
        // The cursor in the middle of a word: the part before it.
        assert_eq!(at("cat abc▏def"), arg("cat", 0, "abc"));
    }

    #[test]
    fn words_and_roles() {
        let l = read("sudo -u root FOO=1 make -j8 all >log 2>&1 && git push", 0);
        let roles: Vec<(&str, Role)> = l.words.iter().map(|w| (w.text.as_str(), w.role)).collect();
        assert_eq!(
            roles,
            [
                ("sudo", Role::Command),
                ("-u", Role::Flag),
                ("root", Role::Argument),
                ("FOO=1", Role::Assignment),
                ("make", Role::Command),
                ("-j8", Role::Flag),
                ("all", Role::Argument),
                ("log", Role::Target),
                ("git", Role::Command),
                ("push", Role::Argument),
            ]
        );
        assert_eq!(l.words[6].command, Some(4));
        assert_eq!(l.words[9].command, Some(8));
        let l = read("cat \"$HOME\"/x 'open", 0);
        assert!(l.words[1].expands && l.words[1].open_quote.is_none());
        assert_eq!(l.words[2].open_quote, Some('\''));
        assert_eq!(l.words[2].text, "open");
    }

    #[test]
    fn spans_for_colouring() {
        assert_eq!(
            spans("git commit -m \"fix; it\" | grep -v $HOME > ./out.txt"),
            [
                "Command:git",
                "Argument:commit",
                "Flag:-m",
                "String:\"fix; it\"",
                "Operator:|",
                "Command:grep",
                "Flag:-v",
                "Variable:$HOME",
                "Redirect:>",
                "Argument:./out.txt",
            ]
        );
        assert_eq!(
            spans("echo \"a $b c\" $(ls) `id` $'\\n' x\\ y # done"),
            [
                "Command:echo",
                "String:\"a ",
                "Variable:$b",
                "String: c\"",
                "Variable:$(ls)",
                "Variable:`id`",
                "String:$'\\n'",
                "Argument:x\\ y",
                "Comment:# done",
            ]
        );
        assert_eq!(
            spans("if [[ -n $x ]]; then FOO=1 ls; fi"),
            [
                "Keyword:if",
                "Keyword:[[",
                "Flag:-n",
                "Variable:$x",
                "Keyword:]]",
                "Operator:;",
                "Keyword:then",
                "Assignment:FOO=1",
                "Command:ls",
                "Operator:;",
                "Keyword:fi",
            ]
        );
        // A reserved word quoted is a command name like any other.
        assert_eq!(spans("'if' x"), ["String:'if'", "Argument:x"]);
        assert_eq!(read("'if' x", 0).words[0].role, Role::Command);
        // Spans are in order and don't overlap.
        let line = "a\"b$c\"d'e'$(f) g>h";
        let l = read(line, 0);
        assert!(l.spans.windows(2).all(|w| w[0].range.end <= w[1].range.start), "{:?}", l.spans);
        assert!(l.spans.iter().all(|s| s.range.start < s.range.end));
    }

    #[test]
    fn redirects() {
        for (line, want) in [
            ("a 2>&1", vec!["Command:a", "Redirect:2>&1"]),
            ("a &>log", vec!["Command:a", "Redirect:&>", "Argument:log"]),
            ("a >>log", vec!["Command:a", "Redirect:>>", "Argument:log"]),
            ("a <<<word", vec!["Command:a", "Redirect:<<<", "Argument:word"]),
            ("a >&-", vec!["Command:a", "Redirect:>&-"]),
            (
                "diff <(a) >(b) c",
                vec!["Command:diff", "Variable:<(a)", "Variable:>(b)", "Argument:c"],
            ),
        ] {
            assert_eq!(spans(line), want, "{line}");
        }
    }

    #[test]
    fn what_it_doesnt_understand_gives_nothing() {
        for line in ["cat <<EOF", "cat <<-EOF", "case $x in", "if x; then case y in a) b;; esac; fi"] {
            let l = read(line, line.len());
            assert_eq!((l.words.len(), l.spans.len(), &l.at), (0, 0, &At::Nothing), "{line}");
        }
        let long = "x".repeat(MAX_LINE + 1);
        assert_eq!(read(&long, 3).at, At::Nothing);
    }

    #[test]
    fn broken_input_does_not_panic() {
        for line in [
            "\"", "'", "$", "${", "$(", "`", "\\", "$'", "a\\", "\"$", "\"$(", ">", "2>&", "&", ";;", "((", "))", "é\\", "\"é", "$é",
            "${é}", "[[", "]]", "for", "sudo -u", "env -u",
        ] {
            for cursor in 0..=line.len() {
                if line.is_char_boundary(cursor) {
                    let _ = read(line, cursor);
                }
            }
        }
    }

    proptest::proptest! {
        /// Any line, any cursor: no panic, spans in order inside the line,
        /// words in order.
        #[test]
        fn any_line_reads(line in "[ a-z0-9\\-\"'$`(){}<>|&;#=~/\\\\\n\té]{0,40}", cursor in 0usize..48) {
            let mut cursor = cursor.min(line.len());
            while !line.is_char_boundary(cursor) {
                cursor -= 1;
            }
            let l = read(&line, cursor);
            for s in &l.spans {
                proptest::prop_assert!(s.range.start < s.range.end && s.range.end <= line.len(), "{:?}", s);
            }
            proptest::prop_assert!(l.spans.windows(2).all(|w| w[0].range.end <= w[1].range.start), "{:?}", l.spans);
            proptest::prop_assert!(l.words.windows(2).all(|w| w[0].range.end <= w[1].range.start));
        }
    }
}
