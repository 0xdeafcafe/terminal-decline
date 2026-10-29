//! The link table: the OSC 8 hyperlinks one screen's styles refer to, each
//! stored once (§5.5 of the cells design). A style's `link` is an id here,
//! 0 for none.
//!
//! Links are freed by the style table's sweep: after it frees styles, it
//! frees every link that no live style and no pen refers to. Ids are never
//! renumbered (a style's hash includes its link id); a freed id is reused
//! for the next new link.

use std::collections::HashMap;
use std::sync::Arc;

use crate::style::LinkId;

/// URIs longer than this are ignored (§5.5).
pub const MAX_URI: usize = 4096;
/// `id=` parameters longer than this are ignored.
pub const MAX_ID: usize = 256;

/// An OSC 8 hyperlink, by value: links are compared by URI and id, never
/// by table id, since screen and history number them differently.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LinkValue {
    pub uri: String,
    /// The `id=` parameter, if the program gave one (an empty one is none).
    pub id: Option<String>,
}

/// The hyperlinks of one screen.
#[derive(Clone, Debug, Default)]
pub(crate) struct LinkTable {
    /// By id; id 0 = no link, always `None`, and freed ids are `None`.
    links: Vec<Option<Arc<LinkValue>>>,
    index: HashMap<Arc<LinkValue>, LinkId>,
    /// Freed ids, lowest last, so they're used first.
    free: Vec<LinkId>,
    /// Scratch for the sweep: one bit per id.
    marks: Vec<u64>,
}

impl LinkTable {
    pub fn new() -> LinkTable {
        LinkTable {
            links: vec![None],
            ..LinkTable::default()
        }
    }

    /// The id of `link`, adding it if it is new.
    pub fn intern(&mut self, link: &Arc<LinkValue>) -> LinkId {
        if let Some(&id) = self.index.get(&**link) {
            return id;
        }
        let id = match self.free.pop() {
            Some(id) => {
                self.links[id as usize] = Some(link.clone());
                id
            }
            None => {
                self.links.push(Some(link.clone()));
                (self.links.len() - 1) as LinkId
            }
        };
        self.index.insert(link.clone(), id);
        id
    }

    /// The id `link` has, without adding it.
    pub fn find(&self, link: &LinkValue) -> Option<LinkId> {
        self.index.get(link).copied()
    }

    /// The links by id, `None` for 0 and freed ids.
    #[inline]
    pub fn as_slice(&self) -> &[Option<Arc<LinkValue>>] {
        &self.links
    }

    /// The link with id `id`; `None` for 0 and freed ids.
    #[inline]
    pub fn get(&self, id: LinkId) -> Option<&Arc<LinkValue>> {
        self.links.get(id as usize)?.as_ref()
    }

    /// Links held, the empty id 0 not counted.
    pub fn live(&self) -> usize {
        self.index.len()
    }

    /// Start a sweep: nothing is marked yet.
    pub fn begin_sweep(&mut self) {
        self.marks.clear();
        self.marks.resize(self.links.len().div_ceil(64), 0);
    }

    /// Something live refers to `id`. Ids past the table are ignored.
    #[inline]
    pub fn mark(&mut self, id: LinkId) {
        if let Some(w) = self.marks.get_mut(id as usize >> 6) {
            *w |= 1 << (id & 63);
        }
    }

    /// Free every link not marked since [`LinkTable::begin_sweep`].
    pub fn end_sweep(&mut self) {
        if self.index.is_empty() {
            return;
        }
        self.free.clear();
        for id in (1..self.links.len()).rev() {
            let marked = self.marks[id >> 6] >> (id & 63) & 1 != 0;
            if !marked && let Some(link) = self.links[id].take() {
                self.index.remove(&*link);
            }
            if self.links[id].is_none() {
                self.free.push(id as LinkId);
            }
        }
        // Freed ids at the end of the table are dropped, not kept free.
        while self.links.len() > 1 && self.links.last().is_some_and(|l| l.is_none()) {
            self.links.pop();
        }
        let len = self.links.len() as LinkId;
        self.free.retain(|&id| id < len);
        if self.index.is_empty() {
            *self = LinkTable::new();
        }
    }

    /// Bytes of heap held, about (for measuring).
    pub fn bytes(&self) -> usize {
        let strings: usize = self
            .index
            .keys()
            .map(|l| std::mem::size_of::<LinkValue>() + 16 + l.uri.capacity() + l.id.as_ref().map_or(0, |s| s.capacity()))
            .sum();
        strings + self.links.capacity() * 8 + self.index.capacity() * 16 + self.free.capacity() * 4 + self.marks.capacity() * 8
    }

    /// The table's own rules: the index and `links` agree, id 0 is empty,
    /// and `free` holds exactly the empty ids.
    pub fn check(&self) -> Result<(), String> {
        if self.links.first().is_none_or(|l| l.is_some()) {
            return Err("link id 0 isn't the empty link".into());
        }
        for (link, &id) in &self.index {
            if self.get(id).map(|l| &**l) != Some(&**link) {
                return Err(format!("link {link:?} indexed as {id}, which holds {:?}", self.get(id)));
            }
        }
        let held = self.links.iter().filter(|l| l.is_some()).count();
        if held != self.index.len() {
            return Err(format!("{held} links held, {} indexed", self.index.len()));
        }
        let mut free = self.free.clone();
        free.sort_unstable();
        let empty: Vec<LinkId> = (1..self.links.len() as LinkId).filter(|&id| self.get(id).is_none()).collect();
        if free != empty {
            return Err(format!("free link ids {free:?}, empty ids {empty:?}"));
        }
        Ok(())
    }
}

/// The URI and `id=` parameter of an `OSC 8 ; params ; URI` string, from
/// vte's parts (`parts[0]` is `8`). The URI may contain `;`, which vte
/// splits on, so it is the remaining parts joined with `;` (§14).
///
/// `None` ends the current link: an empty URI, a malformed string, a URI
/// or id past the limits, or 16 parts (vte's most, so the string may have
/// been cut short).
pub(crate) fn parse_osc8(parts: &[&[u8]]) -> Option<LinkValue> {
    if parts.len() < 3 || parts.len() >= 16 {
        return None;
    }
    let mut uri = Vec::new();
    for (i, p) in parts[2..].iter().enumerate() {
        if i > 0 {
            uri.push(b';');
        }
        uri.extend_from_slice(p);
    }
    if uri.is_empty() || uri.len() > MAX_URI {
        return None;
    }
    // Parameters are `key=value` pairs separated by `:`; only `id` is used.
    let mut id = None;
    for kv in parts[1].split(|&b| b == b':') {
        if let Some(v) = kv.strip_prefix(b"id=") {
            if v.len() > MAX_ID {
                return None;
            }
            id = (!v.is_empty()).then(|| String::from_utf8_lossy(v).into_owned());
        }
    }
    let uri = String::from_utf8(uri).ok()?;
    if uri.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(LinkValue { uri, id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(uri: &str, id: Option<&str>) -> Arc<LinkValue> {
        Arc::new(LinkValue {
            uri: uri.into(),
            id: id.map(Into::into),
        })
    }

    #[test]
    fn same_value_same_id() {
        let mut t = LinkTable::new();
        let a = t.intern(&link("https://a", None));
        let b = t.intern(&link("https://b", None));
        let a2 = t.intern(&link("https://a", Some("x")));
        assert_eq!((a, b), (1, 2));
        assert_ne!(a2, a, "the id parameter is part of the link");
        assert_eq!(t.intern(&link("https://a", None)), a);
        assert_eq!(t.find(&link("https://a", Some("x"))), Some(a2));
        assert_eq!(t.find(&link("https://c", None)), None);
        assert_eq!(t.get(0), None);
        assert_eq!(t.get(b).map(|l| l.uri.as_str()), Some("https://b"));
        t.check().unwrap();
    }

    #[test]
    fn sweep_frees_unmarked_and_reuses_ids() {
        let mut t = LinkTable::new();
        for i in 0..5 {
            t.intern(&link(&format!("u{i}"), None));
        }
        t.begin_sweep();
        t.mark(2);
        t.mark(4);
        t.mark(999);
        t.end_sweep();
        t.check().unwrap();
        assert_eq!(t.live(), 2);
        // Id 5 was at the end, so it went; 1 and 3 are free, 1 first.
        assert_eq!(t.intern(&link("new", None)), 1);
        assert_eq!(t.intern(&link("newer", None)), 3);
        assert_eq!(t.intern(&link("newest", None)), 5);
        assert_eq!(t.find(&link("u1", None)), Some(2));
        t.check().unwrap();
        t.begin_sweep();
        t.end_sweep();
        assert_eq!(t.live(), 0);
        assert_eq!(t.intern(&link("again", None)), 1);
        t.check().unwrap();
    }

    fn parse(s: &str) -> Option<LinkValue> {
        let parts: Vec<&[u8]> = s.as_bytes().split(|&b| b == b';').collect();
        parse_osc8(&parts)
    }

    #[test]
    fn osc8_strings() {
        assert_eq!(
            parse("8;;https://x"),
            Some(LinkValue {
                uri: "https://x".into(),
                id: None
            })
        );
        assert_eq!(
            parse("8;id=a;https://x?q=1;2"),
            Some(LinkValue {
                uri: "https://x?q=1;2".into(),
                id: Some("a".into())
            })
        );
        assert_eq!(parse("8;foo=bar:id=7:baz;file:///tmp").and_then(|l| l.id), Some("7".into()));
        assert_eq!(parse("8;id=;u").and_then(|l| l.id), None);
        assert_eq!(parse("8;;"), None, "an empty URI ends the link");
        assert_eq!(parse("8;id=a;"), None);
        assert_eq!(parse("8"), None);
        assert_eq!(parse(&format!("8;;{}", "a".repeat(MAX_URI))).map(|l| l.uri.len()), Some(MAX_URI));
        assert_eq!(parse(&format!("8;;{}", "a".repeat(MAX_URI + 1))), None);
        assert_eq!(parse(&format!("8;id={};u", "i".repeat(MAX_ID + 1))), None);
        assert_eq!(parse(&format!("8;;u{}", ";".repeat(12))).map(|l| l.uri.len()), Some(13));
        assert_eq!(parse(&format!("8;;u{}", ";".repeat(13))), None, "16 parts may be cut short");
        assert_eq!(parse("8;;a\u{7}b"), None);
    }
}
