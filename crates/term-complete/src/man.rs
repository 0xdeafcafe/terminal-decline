//! A command's flags and what each does, and its one-line description,
//! read from its man page (docs/design/completion.md §6). Two layouts
//! cover what macOS and Homebrew install: mdoc (BSD, macOS's own: `.It
//! Fl x Ar arg` items) and man(7) (GNU, Homebrew, git's asciidoc: a line
//! of flags after `.TP`, or before an `.RS` block). Pure: the page comes
//! in as text; [`super::sources`] finds and reads it on the worker.

use unicode_width::UnicodeWidthChar;

/// The longest description kept, in cells; longer ones end in `…`.
pub const ABOUT_CELLS: usize = 52;
/// At most this many flags are kept from a page.
const MAX_FLAGS: usize = 1000;

/// One flag of a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    pub name: String,
    /// The first sentence of what it does.
    pub about: String,
}

/// What a man page says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    /// Its NAME line's description (`git-fetch - Download objects…`).
    pub about: String,
    /// In the page's order, one per name.
    pub flags: Vec<Flag>,
}

/// Read a man page's flags and description.
pub fn read(src: &str) -> Page {
    let mdoc = src.lines().any(|l| l.starts_with(".Dd") || l.starts_with(".Sh "));
    let mut r = Reader::default();
    for line in src.lines() {
        if mdoc {
            r.mdoc_line(line);
        } else {
            r.man_line(line);
        }
        if r.page.flags.len() >= MAX_FLAGS {
            break;
        }
    }
    r.finish_item();
    r.page
}

/// Only the NAME line's description: stops at the section after it.
pub fn about(src: &str) -> String {
    let mdoc = src.lines().any(|l| l.starts_with(".Dd") || l.starts_with(".Sh "));
    let mut r = Reader::default();
    for line in src.lines() {
        if mdoc {
            r.mdoc_line(line);
        } else {
            r.man_line(line);
        }
        if r.past_name {
            break;
        }
    }
    r.page.about
}

#[derive(Default)]
struct Reader {
    page: Page,
    /// In the NAME section; and past it.
    in_name: bool,
    past_name: bool,
    name_text: String,
    /// The flag item being read: its names and description so far.
    item: Option<(Vec<String>, String)>,
    /// man(7): the line after `.TP` is the tag.
    tag_next: bool,
    /// man(7): text since the last paragraph, a tag if an `.RS` follows.
    pending: Vec<String>,
}

impl Reader {
    fn finish_item(&mut self) {
        let Some((names, text)) = self.item.take() else { return };
        let about = sentence(&text);
        for name in names {
            if !self.page.flags.iter().any(|f| f.name == name) {
                self.page.flags.push(Flag {
                    name,
                    about: about.clone(),
                });
            }
        }
    }

    fn add_text(&mut self, text: &str) {
        if self.in_name {
            push_words(&mut self.name_text, text);
        } else if let Some((_, desc)) = &mut self.item {
            push_words(desc, text);
        }
    }

    /// A section starts: NAME's description is read at the end of it.
    fn section(&mut self, title: &str) {
        self.finish_item();
        if self.in_name {
            self.in_name = false;
            self.past_name = true;
            let text = std::mem::take(&mut self.name_text);
            // `name, other \- what it does`.
            let about = text
                .strip_prefix("- ")
                .map(|d| ("", d))
                .or_else(|| text.split_once(" - "))
                .or_else(|| text.split_once(" — "))
                .map_or(text.as_str(), |(_, d)| d);
            self.page.about = sentence(about);
        }
        let title = title.trim().trim_matches('"');
        if title.eq_ignore_ascii_case("NAME") && !self.past_name {
            self.in_name = true;
        }
    }

    fn mdoc_line(&mut self, line: &str) {
        let Some(rest) = line.strip_prefix('.').or_else(|| line.strip_prefix('\'')) else {
            self.add_text(&plain(line));
            return;
        };
        let (mac, args) = rest.split_once([' ', '\t']).unwrap_or((rest, ""));
        match mac {
            "Sh" | "SH" => self.section(args),
            "Ss" | "El" => self.finish_item(),
            "It" => {
                self.finish_item();
                let names = mdoc_flags(args);
                if !names.is_empty() {
                    self.item = Some((names, String::new()));
                }
            }
            "Nd" => {
                if self.in_name {
                    self.name_text.clear();
                    push_words(&mut self.name_text, &format!("- {}", mdoc_text(args)));
                }
            }
            // The first paragraph of a flag's description is enough.
            "Pp" | "Bl" | "Bd" => {
                if self.item.as_ref().is_some_and(|(_, d)| !d.is_empty()) {
                    self.finish_item();
                }
            }
            "Nm" if self.in_name => push_words(&mut self.name_text, &mdoc_text(args)),
            _ if mac.starts_with('\\') => {}
            _ => {
                let text = mdoc_text(&format!("{mac} {args}"));
                self.add_text(&text);
            }
        }
    }

    fn man_line(&mut self, line: &str) {
        let Some(rest) = line.strip_prefix('.').or_else(|| line.strip_prefix('\'')) else {
            self.man_text(&man_plain(line));
            return;
        };
        let (mac, args) = rest.split_once([' ', '\t']).unwrap_or((rest, ""));
        match mac {
            "SH" | "Sh" => {
                self.pending.clear();
                self.section(args);
            }
            "SS" => {
                self.finish_item();
                self.pending.clear();
            }
            "TP" => {
                self.finish_item();
                self.pending.clear();
                self.tag_next = true;
            }
            "IP" => {
                self.finish_item();
                self.pending.clear();
                let tag = plain(first_arg(args));
                let names = flag_names(&tag);
                if !names.is_empty() {
                    self.item = Some((names, String::new()));
                }
            }
            "RS" => {
                if self.item.is_none() && !self.pending.is_empty() {
                    let tag = self.pending.join(" ");
                    let names = flag_names(&tag);
                    if !names.is_empty() {
                        self.item = Some((names, String::new()));
                    }
                }
                self.pending.clear();
            }
            // A description ends with its first paragraph.
            "PP" | "P" | "LP" | "sp" | "HP" | "RE" => {
                if self.item.as_ref().is_some_and(|(_, d)| !d.is_empty()) {
                    self.finish_item();
                }
                self.pending.clear();
            }
            // Font macros carry text: `.B \-x`, `.BR \-\-color " WHEN"`.
            "B" | "I" | "R" | "SM" | "SB" => self.man_text(&man_plain(&words(args).join(" "))),
            "BR" | "BI" | "IB" | "IR" | "RB" | "RI" => self.man_text(&man_plain(&words(args).concat())),
            _ => {}
        }
    }

    fn man_text(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        if self.tag_next {
            self.tag_next = false;
            let names = flag_names(text);
            if !names.is_empty() {
                self.item = Some((names, String::new()));
            }
            return;
        }
        if self.in_name || self.item.is_some() {
            self.add_text(text);
        } else {
            self.pending.push(text.to_string());
        }
    }
}

/// The first argument of a macro line, quoted or not.
fn first_arg(args: &str) -> &str {
    let args = args.trim_start();
    if let Some(rest) = args.strip_prefix('"') {
        rest.split('"').next().unwrap_or("")
    } else {
        args.split_whitespace().next().unwrap_or("")
    }
}

/// A macro line's arguments, `"quoted ones"` kept whole.
fn words(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = args.trim_start();
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('"') {
            let end = r.find('"').unwrap_or(r.len());
            out.push(&r[..end]);
            rest = r.get(end + 1..).unwrap_or("").trim_start();
        } else {
            let end = rest.find([' ', '\t']).unwrap_or(rest.len());
            out.push(&rest[..end]);
            rest = rest[end..].trim_start();
        }
    }
    out
}

/// Whether `w` is an mdoc macro that can be called on another's line.
fn is_mdoc_macro(w: &str) -> bool {
    matches!(
        w,
        "Ad" | "An"
            | "Ap"
            | "Ar"
            | "Bc"
            | "Bo"
            | "Bq"
            | "Brc"
            | "Bro"
            | "Brq"
            | "Bsx"
            | "Bx"
            | "Cd"
            | "Cm"
            | "Dc"
            | "Do"
            | "Dq"
            | "Dv"
            | "Dx"
            | "Ec"
            | "Em"
            | "Eo"
            | "Er"
            | "Ev"
            | "Fa"
            | "Fc"
            | "Fl"
            | "Fn"
            | "Fo"
            | "Ft"
            | "Fx"
            | "Ic"
            | "Li"
            | "Lk"
            | "Ms"
            | "Mt"
            | "Nm"
            | "No"
            | "Ns"
            | "Nx"
            | "Oc"
            | "Oo"
            | "Op"
            | "Ox"
            | "Pa"
            | "Pc"
            | "Pf"
            | "Po"
            | "Pq"
            | "Qc"
            | "Ql"
            | "Qo"
            | "Qq"
            | "Sc"
            | "So"
            | "Sq"
            | "St"
            | "Sx"
            | "Sy"
            | "Ta"
            | "Tn"
            | "Ux"
            | "Va"
            | "Vt"
            | "Xc"
            | "Xo"
            | "Xr"
    )
}

/// The flags named on an mdoc `.It` line: each `Fl x` is `-x`.
fn mdoc_flags(args: &str) -> Vec<String> {
    let ws = words(args);
    let mut names = Vec::new();
    let mut i = 0;
    while i < ws.len() {
        if ws[i] == "Fl" {
            // `Fl Fl text` is `--text`.
            let dashes = ws[i..].iter().take_while(|w| **w == "Fl").count();
            i += dashes - 1;
            if let Some(next) = ws.get(i + 1).filter(|w| !is_mdoc_macro(w)) {
                let name = format!("{}{}", "-".repeat(dashes), plain(next));
                if valid_name(&name) {
                    names.push(name);
                }
                i += 1;
            }
        }
        i += 1;
    }
    names
}

/// An mdoc macro line's text: macro names dropped, `Fl x` as `-x`,
/// `Xr ls 1` as `ls(1)`, punctuation kept against the word before it.
fn mdoc_text(args: &str) -> String {
    let ws = words(args);
    let mut out = String::new();
    let mut glue = false;
    let mut i = 0;
    while i < ws.len() {
        let w = ws[i];
        let piece = match w {
            "Ns" => {
                glue = true;
                i += 1;
                continue;
            }
            "Fl" => {
                let next = ws.get(i + 1).filter(|n| !is_mdoc_macro(n));
                if next.is_some() {
                    i += 1;
                }
                format!("-{}", next.map(|n| plain(n)).unwrap_or_default())
            }
            "Xr" => {
                let (name, sec) = (ws.get(i + 1).copied().unwrap_or(""), ws.get(i + 2).copied().unwrap_or(""));
                i += 2;
                format!("{}({})", plain(name), plain(sec))
            }
            _ if is_mdoc_macro(w) => {
                i += 1;
                continue;
            }
            _ => plain(w),
        };
        let punct = matches!(piece.as_str(), "," | "." | ";" | ":" | ")" | "?" | "!");
        if !out.is_empty() && !glue && !punct {
            out.push(' ');
        }
        out.push_str(&piece);
        glue = piece == "(";
        i += 1;
    }
    out
}

/// The flag names in a tag line: `-r, --release`, `--color=WHEN`,
/// `--[no-]tags` (both), `-j <n>`. Empty unless it starts with a flag.
pub(super) fn flag_names(tag: &str) -> Vec<String> {
    let tag = tag.trim();
    if !tag.starts_with('-') {
        return Vec::new();
    }
    let mut names = Vec::new();
    for piece in tag.split([',', '|']) {
        let piece = piece.trim();
        if !piece.starts_with('-') {
            continue;
        }
        let (dashes, rest) = piece.split_at(piece.len() - piece.trim_start_matches('-').len());
        if let Some(rest) = rest.strip_prefix("[no-]") {
            let name: String = rest.chars().take_while(|c| name_char(*c)).collect();
            for n in [format!("{dashes}{name}"), format!("{dashes}no-{name}")] {
                if valid_name(&n) {
                    names.push(n);
                }
            }
            continue;
        }
        let name: String = piece
            .chars()
            .take_while(|c| *c == '-')
            .chain(rest.chars().take_while(|c| name_char(*c)))
            .collect();
        if valid_name(&name) {
            names.push(name);
        }
    }
    names
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '?' | '#' | '@' | '+')
}

fn valid_name(n: &str) -> bool {
    let body = n.trim_start_matches('-');
    !body.is_empty() && n.len() - body.len() <= 2 && body.chars().all(name_char) && !body.starts_with('-')
}

/// Words added to `out` with one space between.
fn push_words(out: &mut String, text: &str) {
    for w in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
}

/// The first sentence of `text`, at most [`ABOUT_CELLS`] wide.
pub(super) fn sentence(text: &str) -> String {
    let text = text.trim();
    let mut end = text.len();
    for (i, _) in text.match_indices(". ") {
        // Not after `e.g` or `i.e` or a single letter.
        let before = &text[..i];
        let word = before.rsplit(' ').next().unwrap_or("");
        if word.len() > 1 && !word.contains('.') {
            end = i;
            break;
        }
    }
    let s = text[..end].trim_end_matches(['.', ':', ',']);
    let mut out = String::new();
    let mut cells = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if cells + w > ABOUT_CELLS - 1 {
            // At a word's end.
            if let Some(i) = out.rfind(' ') {
                out.truncate(i);
            }
            out.truncate(out.trim_end_matches([' ', ',', ';', ':']).len());
            out.push('…');
            return out;
        }
        cells += w;
        out.push(c);
    }
    out
}

/// Text with roff's escapes read: fonts and sizes dropped, `\-` as `-`,
/// named characters (`\(em`, `\[aq]`) as themselves, comments removed.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(e) = chars.next() else { break };
        match e {
            '-' => out.push('-'),
            'e' | '\\' => out.push('\\'),
            ' ' | '~' | '0' => out.push(' '),
            '"' | '#' => break,
            '(' => {
                let name: String = chars.by_ref().take(2).collect();
                out.push_str(named(&name));
            }
            '[' => {
                let name: String = chars.by_ref().take_while(|c| *c != ']').collect();
                out.push_str(named(&name));
            }
            // A font, size, string, register or colour: its name dropped.
            'f' | '*' | 'n' | 'm' | 'k' | 'F' | 'g' | 'Y' | 's' => {
                if e == 's' {
                    while chars.peek().is_some_and(|c| matches!(c, '+' | '-')) {
                        chars.next();
                    }
                }
                match chars.next() {
                    Some('(') => {
                        chars.next();
                        chars.next();
                    }
                    Some('[') => while chars.next().is_some_and(|c| c != ']') {},
                    Some(d) if e == 's' && d.is_ascii_digit() && chars.peek().is_some_and(|c| c.is_ascii_digit()) => {
                        chars.next();
                    }
                    _ => {}
                }
            }
            // Escapes with a quoted argument: motions, widths, boxes.
            'h' | 'v' | 'w' | 'o' | 'l' | 'L' | 'X' | 'Z' | 'D' | 'b' | 'x' | 'N' | 'R' | 'A' | 'B' | 'C' | 'H' | 'S' | 'V' => {
                if let Some(delim) = chars.next() {
                    while chars.next().is_some_and(|c| c != delim) {}
                }
            }
            _ => {}
        }
    }
    out
}

/// man(7) text read: an italic argument is set apart from the flag
/// before it (`\-b\fIn\fR` is `-b n`, not a flag `-bn`).
fn man_plain(text: &str) -> String {
    plain(&text.replace("\\fI", " \\fI"))
}

/// A named character.
fn named(name: &str) -> &'static str {
    match name {
        "em" => "—",
        "en" => "–",
        "bu" => "•",
        "aq" | "oq" | "cq" | "fm" => "'",
        "dq" | "lq" | "rq" | "Lq" | "Rq" => "\"",
        "mi" | "hy" | "-" | "-D" => "-",
        "co" => "©",
        "rg" => "®",
        "tm" => "™",
        "ga" => "`",
        "ti" | "ap" => "~",
        "ha" | "a^" => "^",
        "rs" => "\\",
        "lB" => "[",
        "rB" => "]",
        "lC" => "{",
        "rC" => "}",
        "la" => "<",
        "ra" => ">",
        "<=" => "≤",
        ">=" => "≥",
        "->" => "→",
        "<-" => "←",
        "u2014" => "—",
        "u2013" => "–",
        "u2019" | "u2018" => "'",
        "u201C" | "u201D" => "\"",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: &Page) -> Vec<&str> {
        p.flags.iter().map(|f| f.name.as_str()).collect()
    }

    fn about_of<'a>(p: &'a Page, name: &str) -> &'a str {
        &p.flags.iter().find(|f| f.name == name).unwrap().about
    }

    #[test]
    fn mdoc_items() {
        let src = r#".Dd March 3, 2024
.Dt LS 1
.Sh NAME
.Nm ls
.Nd list directory contents
.Sh SYNOPSIS
.Nm
.Op Fl ABC
.Sh DESCRIPTION
.Bl -tag -width indent
.It Fl -color Ns = Ns Ar when
Output colored escape sequences based on
.Ar when ,
which may be set to either
.Cm always .
.It Fl a
Include directory entries whose names begin with a
dot
.Pq Sq Pa \&. .
.It Fl l
(The lowercase letter
.Dq ell . )
List files in the long format, as described in the
.Sx The Long Format
subsection below.
.It Fl B Ar num , Fl Fl bytes Ar num
Print
.Ar num
bytes. Then more.
.El
"#;
        let p = read(src);
        assert_eq!(p.about, "list directory contents");
        assert_eq!(names(&p), ["--color", "-a", "-l", "-B", "--bytes"]);
        assert_eq!(about_of(&p, "--color"), "Output colored escape sequences based on when…");
        assert_eq!(about_of(&p, "-B"), "Print num bytes");
        assert_eq!(about(src), "list directory contents");
    }

    #[test]
    fn man_tags_before_rs_blocks() {
        // git's asciidoc and cargo's pages.
        let src = r#".TH "GIT\-FETCH" "1"
.SH "NAME"
git-fetch \- Download objects and refs from another repository
.SH "OPTIONS"
.PP
\fB\-\-all\fR, \fB\-\-no\-all\fR
.RS 4
Fetch all remotes, except for the ones that have the
\fBremote\&.\fR\fI<name>\fR\fB\&.skipFetchAll\fR
configuration variable set\&. This overrides it\&.
.RE
.PP
\fB\-j\fR, \fB\-\-jobs=\fR\fI<n>\fR
.RS 4
Number of parallel children to be used\&.
.RE
.PP
\fB\-\-[no\-]write\-fetch\-head\fR
.RS 4
Write the list of remote refs fetched\&.
.RE
.sp
\fB\-r\fR,
\fB\-\-release\fR
.RS 4
Build optimized artifacts with the \fBrelease\fR profile\&.
.RE
"#;
        let p = read(src);
        assert_eq!(p.about, "Download objects and refs from another repository");
        assert_eq!(
            names(&p),
            [
                "--all",
                "--no-all",
                "-j",
                "--jobs",
                "--write-fetch-head",
                "--no-write-fetch-head",
                "-r",
                "--release"
            ]
        );
        assert_eq!(about_of(&p, "--jobs"), "Number of parallel children to be used");
        assert_eq!(about_of(&p, "--release"), "Build optimized artifacts with the release profile");
        assert_eq!(about(src), "Download objects and refs from another repository");
    }

    #[test]
    fn man_tp_items() {
        assert_eq!(names(&read(".TH LESS 1\n.TP\n\\-b\\fIn\\fR\nBuffers.\n")), ["-b"]);
        let src = r#".TH GREP 1
.SH NAME
grep, egrep \- print lines that match patterns
.SH OPTIONS
.TP
.BR \-i ", " \-\-ignore\-case
Ignore case distinctions in patterns and data.
.TP
\fB\-v\fP, \fB\-\-invert\-match\fP
Invert the sense of matching. More.
.IP "\-x" 4
Select only whole lines.
.TP
.B FILES
Not a flag.
"#;
        let p = read(src);
        assert_eq!(p.about, "print lines that match patterns");
        assert_eq!(names(&p), ["-i", "--ignore-case", "-v", "--invert-match", "-x"]);
        assert_eq!(about_of(&p, "-v"), "Invert the sense of matching");
    }

    #[test]
    fn escapes() {
        assert_eq!(plain(r"\fB\-\-all\fR"), "--all");
        assert_eq!(plain(r"cargo\-build \[em] Compile"), "cargo-build — Compile");
        assert_eq!(plain(r"it\(aqs \f(CWcode\fP \s-1small\s0 \*(lqq\(rq"), "it's code small q\"");
        assert_eq!(plain(r#"text \" a comment"#), "text ");
        assert_eq!(plain(r"a\h'1n'b\&."), "ab.");
    }

    #[test]
    fn long_descriptions_are_cut_short() {
        let s = sentence(&"word ".repeat(30));
        assert!(s.ends_with('…'));
        assert!(s.chars().count() <= ABOUT_CELLS);
        assert_eq!(sentence("Use e.g. this. Then that."), "Use e.g. this");
    }

    #[test]
    fn broken_pages_do_not_panic() {
        for src in [
            "",
            ".",
            "\\",
            ".It Fl",
            ".TP\n",
            ".RS\n.RE",
            "\\f",
            "\\(",
            "\\s+",
            ".IP \"",
            ".SH NAME\n.SH",
        ] {
            read(src);
            about(src);
        }
    }
}
