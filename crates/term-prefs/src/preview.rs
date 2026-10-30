//! The live theme preview: while a theme list is open in Settings, the
//! preferences ask the app to show the theme under the selection in every
//! window, through `OSC 7771 ; theme-preview ; … ST` (honoured only from
//! the Settings tab, like `record-chord`). Nothing is written to the
//! config until Enter.
//!
//! Anything that writes to the Settings tab's terminal could send these:
//! the editor `e` opens, or a file it shows. So the app gives the
//! preferences a token in [`TOKEN_ENV`] and hears only requests that
//! start with it ([`sign`], [`verify`]); the preferences keep it from the
//! programs they run.

/// The environment variable the app passes the token in.
pub const TOKEN_ENV: &str = "DECLINE_PREFS_TOKEN";

/// `osc` (an `OSC 7771 ; … ST` request) with `token` first:
/// `OSC 7771 ; <token> ; … ST`. Unchanged without a token.
pub fn sign(osc: &str, token: Option<&str>) -> String {
    match (token, osc.strip_prefix("\x1b]7771;")) {
        (Some(t), Some(rest)) => format!("\x1b]7771;{t};{rest}"),
        _ => osc.to_string(),
    }
}

/// What follows `OSC 7771 ;` with the token taken off: `None` unless it
/// starts with `token`.
pub fn verify<'a>(what: &'a str, token: &str) -> Option<&'a str> {
    let rest = what.strip_prefix(token)?.strip_prefix(';')?;
    (!token.is_empty()).then_some(rest)
}

/// What the preferences ask of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    /// Show theme `name` in every window, light and dark alike ("" for
    /// the built-in colours), in place of the config's theme.
    Show(String),
    /// Put back the config's theme now (Esc in the list).
    Cancel,
    /// The previewed theme was just written to the config: keep showing it
    /// until the reload that follows, so windows don't flash back to the
    /// old theme in between. The app drops it at that reload, or soon
    /// after if none comes (the file didn't change).
    Keep,
}

impl Preview {
    /// The sequence the preferences send.
    pub fn osc(&self) -> String {
        let what = match self {
            Preview::Show(name) => format!("show;{name}"),
            Preview::Cancel => "cancel".into(),
            Preview::Keep => "keep".into(),
        };
        format!("\x1b]7771;theme-preview;{what}\x1b\\")
    }

    /// Reads what follows `OSC 7771 ;`.
    pub fn parse(what: &str) -> Option<Preview> {
        let rest = what.strip_prefix("theme-preview;")?;
        match rest {
            "cancel" => Some(Preview::Cancel),
            "keep" => Some(Preview::Keep),
            _ => {
                let name = rest.strip_prefix("show;")?;
                // A theme name is a file name: no controls.
                (!name.chars().any(char::is_control)).then(|| Preview::Show(name.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        for p in [
            Preview::Show("rose-pine-dawn".into()),
            Preview::Show(String::new()),
            Preview::Cancel,
            Preview::Keep,
        ] {
            let osc = p.osc();
            let what = osc.strip_prefix("\x1b]7771;").unwrap().strip_suffix("\x1b\\").unwrap();
            assert_eq!(Preview::parse(what), Some(p));
        }
        assert_eq!(Preview::parse("record-chord"), None);
        assert_eq!(Preview::parse("theme-preview;bogus"), None);
        assert_eq!(Preview::parse("theme-preview;show;a\x07b"), None);
    }

    /// Requests carry the app's token; one without it, or with another,
    /// isn't heard.
    #[test]
    fn requests_are_signed_with_the_token() {
        let osc = sign(&Preview::Cancel.osc(), Some("4f2a"));
        assert_eq!(osc, "\x1b]7771;4f2a;theme-preview;cancel\x1b\\");
        let what = osc.strip_prefix("\x1b]7771;").unwrap().strip_suffix("\x1b\\").unwrap();
        assert_eq!(verify(what, "4f2a").and_then(Preview::parse), Some(Preview::Cancel));
        assert_eq!(verify(what, "4f2b"), None, "another token");
        assert_eq!(verify("theme-preview;cancel", "4f2a"), None, "no token");
        assert_eq!(verify("4f2atheme-preview;cancel", "4f2a"), None);
        assert_eq!(verify(";theme-preview;cancel", ""), None, "an empty token is none");
        assert_eq!(sign("\x1b]7771;record-chord\x1b\\", None), "\x1b]7771;record-chord\x1b\\");
        assert_eq!(sign("plain", Some("t")), "plain");
    }
}
