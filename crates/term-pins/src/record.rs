//! A pin's record, `pins/<id>.pin.toml` (docs/design/pins.md §5.4):
//! what made it and how, and the icon it was given, written only by
//! Terminal Decline, so `--edit` doesn't show it and the config loader
//! never reads it.

use std::path::{Path, PathBuf};

use term_icon::PinIcon;

/// How the pin runs Terminal Decline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// The app's stub, loading the installed library.
    Stub,
    /// A copy of a bare `terminal-decline` binary (§5.6).
    Clone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// The interface version of the stub inside (0 for clones).
    pub stub: u32,
    pub method: Method,
    /// The app version that made it.
    pub made_by: String,
    /// `dev` or `release` (stubs), empty for clones.
    pub requirement: String,
    pub pin_app: PathBuf,
    /// The generated icon's plan; `None` with `--icon` (or none).
    pub icon: Option<PinIcon>,
}

/// The record's file for pin `id`.
pub fn path(pins_dir: &Path, id: &str) -> PathBuf {
    pins_dir.join(format!("{id}.pin.toml"))
}

/// Whether `p` is a record rather than a pin's settings.
pub fn is_record(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".pin.toml"))
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

impl Record {
    pub fn to_toml(&self) -> String {
        use toml::Value;
        let mut t = toml::Table::new();
        t.insert("stub".into(), Value::Integer(self.stub.into()));
        let method = match self.method {
            Method::Stub => "stub",
            Method::Clone => "clone",
        };
        t.insert("method".into(), Value::String(method.into()));
        t.insert("made_by".into(), Value::String(self.made_by.clone()));
        t.insert("requirement".into(), Value::String(self.requirement.clone()));
        t.insert("pin_app".into(), Value::String(self.pin_app.to_string_lossy().into_owned()));
        if let Some(i) = &self.icon {
            let mut icon = toml::Table::new();
            icon.insert("generator".into(), Value::String(i.generator.into()));
            icon.insert("canonical".into(), Value::String(i.canonical.clone()));
            icon.insert("hash".into(), Value::Integer(i.hash.into()));
            icon.insert("index".into(), Value::Integer(i.index.into()));
            icon.insert("top".into(), Value::String(hex(i.top)));
            icon.insert("bottom".into(), Value::String(hex(i.bottom)));
            icon.insert("initial".into(), Value::String(i.initial.clone()));
            icon.insert("outline".into(), Value::String(i.outline.clone()));
            t.insert("icon".into(), Value::Table(icon));
        }
        format!(
            "# Written by Terminal Decline; not settings (those are in the .toml beside it).\n{}",
            toml::to_string(&t).expect("a table of strings and numbers")
        )
    }

    pub fn parse(text: &str) -> Option<Record> {
        let t: toml::Table = text.parse().ok()?;
        let s = |k: &str| t.get(k).and_then(|v| v.as_str()).map(String::from);
        let method = match s("method")?.as_str() {
            "stub" => Method::Stub,
            "clone" => Method::Clone,
            _ => return None,
        };
        let icon = t.get("icon").and_then(|v| v.as_table()).and_then(|i| {
            let s = |k: &str| i.get(k).and_then(|v| v.as_str());
            let n = |k: &str| i.get(k).and_then(|v| v.as_integer());
            // Only a generator this build draws.
            (s("generator")? == term_icon::GENERATOR).then_some(())?;
            Some(PinIcon {
                generator: term_icon::GENERATOR,
                canonical: s("canonical")?.into(),
                hash: u32::try_from(n("hash")?).ok()?,
                index: u8::try_from(n("index")?).ok()?,
                top: parse_hex(s("top")?)?,
                bottom: parse_hex(s("bottom")?)?,
                initial: s("initial")?.into(),
                outline: s("outline")?.into(),
            })
        });
        Some(Record {
            stub: t
                .get("stub")
                .and_then(|v| v.as_integer())
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0),
            method,
            made_by: s("made_by").unwrap_or_default(),
            requirement: s("requirement").unwrap_or_default(),
            pin_app: PathBuf::from(s("pin_app")?),
            icon,
        })
    }

    pub fn read(pins_dir: &Path, id: &str) -> Option<Record> {
        Record::parse(&std::fs::read_to_string(path(pins_dir, id)).ok()?)
    }

    pub fn write(&self, pins_dir: &Path, id: &str) -> Result<(), String> {
        let p = path(pins_dir, id);
        std::fs::create_dir_all(pins_dir).map_err(|e| format!("{}: {e}", pins_dir.display()))?;
        std::fs::write(&p, self.to_toml()).map_err(|e| format!("{}: {e}", p.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut icon = term_icon::plan("Rush");
        icon.outline = "M1 2L3 4Z".into();
        let r = Record {
            stub: 1,
            method: Method::Stub,
            made_by: "0.4.0 (1234)".into(),
            requirement: "dev".into(),
            pin_app: "/Users/me/Applications/Rush.app".into(),
            icon: Some(icon.clone()),
        };
        let text = r.to_toml();
        assert!(text.contains("method = \"stub\"") && text.contains("[icon]"), "{text}");
        assert!(text.contains("top = \"#205B51\""), "{text}");
        assert_eq!(Record::parse(&text), Some(r.clone()));
        let clone = Record {
            method: Method::Clone,
            stub: 0,
            icon: None,
            ..r
        };
        assert_eq!(Record::parse(&clone.to_toml()), Some(clone));
        assert_eq!(Record::parse("method = 'other'\npin_app = '/x'"), None);
        assert!(is_record(Path::new("/p/rush.pin.toml")) && !is_record(Path::new("/p/rush.toml")));
    }
}
