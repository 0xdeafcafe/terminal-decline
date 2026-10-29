//! The panes of a window (or tab): a tree of splits, where each pane is
//! laid out, and what moving focus, closing, zooming and resizing do to it.
//!
//! No AppKit here: the window view keeps a [`Panes`], asks it for a
//! [`Layout`] once per size change and places its pane views and divider
//! views from that. Everything is in whole device pixels, y down, so views,
//! scissor rects and origins agree to the pixel.

use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};

use crate::reload::{SavedNode, SavedPane, SavedPanes};

/// A pane's id: unique in the process, never reused, so a session hook for
/// a pane that has closed can't reach another one.
pub type PaneId = u32;

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// A fresh pane id.
pub fn new_id() -> PaneId {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Where a split puts the new pane: to the right of the old one, or below.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Right,
    Down,
}

/// A direction to move focus or a divider in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    /// The splits whose dividers move along this direction.
    fn axis(self) -> Dir {
        match self {
            Direction::Left | Direction::Right => Dir::Right,
            Direction::Up | Direction::Down => Dir::Down,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    /// `a` is left of (or above) `b`; `ratio` is `a`'s share of the space
    /// left after the divider.
    Split {
        dir: Dir,
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

/// A rectangle in whole device pixels, y down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl PxRect {
    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.w)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h)
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// What a layout is for: the window's content in pixels and its limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub bounds: PxRect,
    /// The title bar (and tab bar) over the top of the content, in pixels.
    /// Panes touching the top have it on top of their padding; dividers
    /// can't be grabbed in it, so a drag there still moves the window.
    pub top_inset: i32,
    /// The smallest a pane may be, padding included (10 columns × 3 lines
    /// of the window's cells), not counting the top inset.
    pub min: (i32, i32),
    /// A divider's width: 1 pt in pixels at the backing scale.
    pub divider: i32,
}

/// How far a divider can be grabbed on each side of its line, in dividers
/// (so 3 pt).
pub const GRAB: i32 = 3;

/// Which split a divider belongs to: the way down from the root, `true`
/// for the second child (`b`).
#[derive(Clone, Debug, PartialEq, Eq, Default, Hash)]
pub struct SplitPath(pub Vec<bool>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divider {
    /// The 1 pt line, drawn.
    pub line: PxRect,
    /// Where it can be grabbed and dragged: the line and 3 pt either side,
    /// within its split, below the top inset.
    pub grab: PxRect,
    /// `Right`: a vertical line between left and right panes.
    pub dir: Dir,
    pub path: SplitPath,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Layout {
    /// In tree order.
    pub panes: Vec<(PaneId, PxRect)>,
    pub dividers: Vec<Divider>,
}

impl Layout {
    pub fn rect(&self, id: PaneId) -> Option<PxRect> {
        self.panes.iter().find(|(p, _)| *p == id).map(|(_, r)| *r)
    }
}

/// A split refused: the new panes wouldn't fit at their minimum size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooSmall;

/// A window's panes: the tree, the pane with focus, and the zoomed one.
#[derive(Clone, Debug, PartialEq)]
pub struct Panes {
    root: Node,
    focused: PaneId,
    zoomed: Option<PaneId>,
}

impl Default for Panes {
    fn default() -> Panes {
        Panes::new()
    }
}

/// A finite ratio within 0–1; anything else is an even split.
fn clean_ratio(r: f32) -> f32 {
    if r.is_finite() { r.clamp(0.0, 1.0) } else { 0.5 }
}

/// How a length `len` is shared by a split: `a`'s length and the
/// divider's. `a` gets `ratio` of what the divider leaves, clamped so both
/// sides keep their minimum; when they can't both, the space is shared in
/// proportion to the minimums.
fn share(len: i32, divider: i32, ratio: f32, min_a: i32, min_b: i32) -> (i32, i32) {
    let len = len.max(0);
    let d = divider.clamp(0, len);
    let avail = len - d;
    let want = (avail as f32 * clean_ratio(ratio)).round() as i32;
    let a = if min_a.saturating_add(min_b) <= avail {
        want.clamp(min_a, avail - min_b)
    } else if min_a.saturating_add(min_b) > 0 {
        (avail as i64 * min_a.max(0) as i64 / (min_a.max(0) as i64 + min_b.max(0) as i64).max(1)) as i32
    } else {
        want.clamp(0, avail)
    };
    (a.clamp(0, avail), d)
}

impl Node {
    fn leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.leaves(out);
                b.leaves(out);
            }
        }
    }

    fn first_leaf(&self) -> PaneId {
        match self {
            Node::Leaf(id) => *id,
            Node::Split { a, .. } => a.first_leaf(),
        }
    }

    fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(p) => *p == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    /// The smallest this subtree can be; `top`: it touches the window's
    /// top, so its top panes carry the inset.
    fn min_size(&self, g: &Geometry, top: bool) -> (i32, i32) {
        match self {
            Node::Leaf(_) => (g.min.0, g.min.1.saturating_add(if top { g.top_inset } else { 0 })),
            Node::Split { dir: Dir::Right, a, b, .. } => {
                let (aw, ah) = a.min_size(g, top);
                let (bw, bh) = b.min_size(g, top);
                (aw.saturating_add(g.divider).saturating_add(bw), ah.max(bh))
            }
            Node::Split { dir: Dir::Down, a, b, .. } => {
                let (aw, ah) = a.min_size(g, top);
                let (bw, bh) = b.min_size(g, false);
                (aw.max(bw), ah.saturating_add(g.divider).saturating_add(bh))
            }
        }
    }

    /// `a`'s rect, the divider's and `b`'s, for a split laid out in `r`.
    fn split_rects(&self, r: PxRect, g: &Geometry, top: bool) -> Option<(PxRect, PxRect, PxRect)> {
        let Node::Split { dir, ratio, a, b } = self else { return None };
        Some(match dir {
            Dir::Right => {
                let (aw, d) = share(r.w, g.divider, *ratio, a.min_size(g, top).0, b.min_size(g, top).0);
                let ra = PxRect { w: aw, ..r };
                let line = PxRect { x: r.x + aw, w: d, ..r };
                let rb = PxRect {
                    x: line.right(),
                    w: r.w.max(0) - aw - d,
                    ..r
                };
                (ra, line, rb)
            }
            Dir::Down => {
                let (ah, d) = share(r.h, g.divider, *ratio, a.min_size(g, top).1, b.min_size(g, false).1);
                let ra = PxRect { h: ah, ..r };
                let line = PxRect { y: r.y + ah, h: d, ..r };
                let rb = PxRect {
                    y: line.bottom(),
                    h: r.h.max(0) - ah - d,
                    ..r
                };
                (ra, line, rb)
            }
        })
    }

    fn layout(&self, r: PxRect, g: &Geometry, top: bool, path: &mut Vec<bool>, out: &mut Layout) {
        let Some((ra, line, rb)) = self.split_rects(r, g, top) else {
            if let Node::Leaf(id) = self {
                out.panes.push((*id, r));
            }
            return;
        };
        let Node::Split { dir, a, b, .. } = self else { return };
        let pad = g.divider.saturating_mul(GRAB);
        let below_inset = g.bounds.y.saturating_add(g.top_inset);
        let grab = match dir {
            Dir::Right => {
                let x0 = (line.x - pad).max(r.x);
                let x1 = (line.right() + pad).min(r.right());
                let y0 = r.y.max(below_inset).min(r.bottom());
                PxRect {
                    x: x0,
                    y: y0,
                    w: (x1 - x0).max(0),
                    h: (r.bottom() - y0).max(0),
                }
            }
            Dir::Down => {
                let y0 = (line.y - pad).max(r.y).max(below_inset).min(r.bottom());
                let y1 = (line.bottom() + pad).min(r.bottom());
                PxRect {
                    x: r.x,
                    y: y0,
                    w: r.w,
                    h: (y1 - y0).max(0),
                }
            }
        };
        path.push(false);
        a.layout(ra, g, top, path, out);
        path.pop();
        out.dividers.push(Divider {
            line,
            grab,
            dir: *dir,
            path: SplitPath(path.clone()),
        });
        path.push(true);
        b.layout(rb, g, top && *dir == Dir::Right, path, out);
        path.pop();
    }

    fn replace_leaf(&mut self, id: PaneId, with: Node) -> bool {
        match self {
            Node::Leaf(p) if *p == id => {
                *self = with;
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => {
                if a.contains(id) {
                    a.replace_leaf(id, with)
                } else {
                    b.replace_leaf(id, with)
                }
            }
        }
    }

    /// Takes leaf `id` out of the tree: its parent split becomes its
    /// sibling. Returns the sibling's first pane. `None` if `id` isn't
    /// below this split (or this is a leaf).
    fn remove(&mut self, id: PaneId) -> Option<PaneId> {
        let keep = match self {
            Node::Leaf(_) => return None,
            Node::Split { a, b, .. } => {
                if **a == Node::Leaf(id) {
                    std::mem::replace(&mut **b, Node::Leaf(0))
                } else if **b == Node::Leaf(id) {
                    std::mem::replace(&mut **a, Node::Leaf(0))
                } else {
                    return a.remove(id).or_else(|| b.remove(id));
                }
            }
        };
        let first = keep.first_leaf();
        *self = keep;
        Some(first)
    }

    /// Equal shares for each run of same-direction splits: returns how
    /// many panes side by side (`dir`) this subtree counts as.
    fn equalize(&mut self, parent: Option<Dir>) -> u32 {
        match self {
            Node::Leaf(_) => 1,
            Node::Split { dir, ratio, a, b } => {
                let na = a.equalize(Some(*dir));
                let nb = b.equalize(Some(*dir));
                *ratio = na as f32 / (na + nb) as f32;
                if parent == Some(*dir) { na + nb } else { 1 }
            }
        }
    }

    fn node_at(&mut self, path: &[bool]) -> Option<&mut Node> {
        match (path.split_first(), self) {
            (None, n) => Some(n),
            (Some((second, rest)), Node::Split { a, b, .. }) => if *second { b } else { a }.node_at(rest),
            (Some(_), Node::Leaf(_)) => None,
        }
    }

    /// The way down to leaf `id`.
    fn path_to(&self, id: PaneId, path: &mut Vec<bool>) -> bool {
        match self {
            Node::Leaf(p) => *p == id,
            Node::Split { a, b, .. } => {
                path.push(false);
                if a.path_to(id, path) {
                    return true;
                }
                path.pop();
                path.push(true);
                if b.path_to(id, path) {
                    return true;
                }
                path.pop();
                false
            }
        }
    }

    fn save(&self, pane: &mut impl FnMut(PaneId) -> SavedPane) -> SavedNode {
        match self {
            Node::Leaf(id) => SavedNode::Pane(pane(*id)),
            Node::Split { dir, ratio, a, b } => SavedNode::Split {
                dir: *dir,
                ratio: *ratio,
                a: Box::new(a.save(pane)),
                b: Box::new(b.save(pane)),
            },
        }
    }

    fn restore(saved: &SavedNode, out: &mut Vec<(PaneId, SavedPane)>) -> Node {
        match saved {
            SavedNode::Pane(pane) => {
                let id = new_id();
                out.push((id, pane.clone()));
                Node::Leaf(id)
            }
            SavedNode::Split { dir, ratio, a, b } => Node::Split {
                dir: *dir,
                ratio: clean_ratio(*ratio),
                a: Box::new(Node::restore(a, out)),
                b: Box::new(Node::restore(b, out)),
            },
        }
    }
}

impl Panes {
    /// One pane, focused.
    pub fn new() -> Panes {
        let id = new_id();
        Panes {
            root: Node::Leaf(id),
            focused: id,
            zoomed: None,
        }
    }

    pub fn root(&self) -> &Node {
        &self.root
    }

    pub fn focused(&self) -> PaneId {
        self.focused
    }

    pub fn zoomed(&self) -> Option<PaneId> {
        self.zoomed
    }

    /// Every pane, in tree order (left to right, top to bottom).
    pub fn ids(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.root.leaves(&mut out);
        out
    }

    pub fn len(&self) -> usize {
        self.ids().len()
    }

    /// Never: a window has at least one pane. (For clippy's `len` rule.)
    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn contains(&self, id: PaneId) -> bool {
        self.root.contains(id)
    }

    /// Splits the focused pane: it keeps the left (or top) half and a new
    /// pane, which gets focus, takes the other. Refused if the two halves
    /// wouldn't each fit at the minimum size. Ends a zoom.
    pub fn split(&mut self, dir: Dir, g: &Geometry) -> Result<PaneId, TooSmall> {
        let Some(r) = self.full_layout(g).rect(self.focused) else {
            return Err(TooSmall);
        };
        let new = new_id();
        let node = Node::Split {
            dir,
            ratio: 0.5,
            a: Box::new(Node::Leaf(self.focused)),
            b: Box::new(Node::Leaf(new)),
        };
        let (w, h) = node.min_size(g, r.y == g.bounds.y);
        if w > r.w || h > r.h {
            return Err(TooSmall);
        }
        let mut root = self.root.clone();
        root.replace_leaf(self.focused, node);
        let (w, h) = root.min_size(g, true);
        if w > g.bounds.w || h > g.bounds.h {
            return Err(TooSmall);
        }
        self.root = root;
        self.focused = new;
        self.zoomed = None;
        Ok(new)
    }

    /// Closes pane `id`: its sibling takes the space. Returns the pane with
    /// focus after it (the sibling's first pane if `id` had focus), or
    /// `None` when `id` was the last pane, which stays: the window (or
    /// tab) closes instead. Closing the zoomed pane ends the zoom.
    pub fn close(&mut self, id: PaneId) -> Option<PaneId> {
        if self.root == Node::Leaf(id) {
            return None;
        }
        let Some(sibling) = self.root.remove(id) else {
            return Some(self.focused);
        };
        if self.focused == id {
            self.focused = sibling;
        }
        if self.zoomed == Some(id) {
            self.zoomed = None;
        }
        Some(self.focused)
    }

    /// Where each pane goes and the dividers between them. Zoomed: the
    /// zoomed pane alone, filling the bounds.
    pub fn layout(&self, g: &Geometry) -> Layout {
        match self.zoomed {
            Some(z) => Layout {
                panes: vec![(z, g.bounds)],
                dividers: Vec::new(),
            },
            None => self.full_layout(g),
        }
    }

    /// The layout as if not zoomed.
    fn full_layout(&self, g: &Geometry) -> Layout {
        let mut out = Layout::default();
        self.root.layout(g.bounds, g, true, &mut Vec::new(), &mut out);
        out
    }

    /// The smallest the content can be with every pane at its minimum: the
    /// window's `contentMinSize`, in pixels.
    pub fn min_size(&self, g: &Geometry) -> (i32, i32) {
        self.root.min_size(g, true)
    }

    /// Moves the divider of the split at `path` so its line starts at `to`
    /// (x for side-by-side panes, y for stacked), clamped so both sides
    /// keep their minimum. Returns whether it moved.
    pub fn drag(&mut self, path: &SplitPath, to: i32, g: &Geometry) -> bool {
        // The split's rect: walk down from the bounds.
        let mut r = g.bounds;
        let mut top = true;
        let mut node = &self.root;
        for &second in &path.0 {
            let Some((ra, _, rb)) = node.split_rects(r, g, top) else {
                return false;
            };
            let Node::Split { dir, a, b, .. } = node else { return false };
            if second {
                top = top && *dir == Dir::Right;
                r = rb;
                node = b;
            } else {
                r = ra;
                node = a;
            }
        }
        let Node::Split { dir, a, b, .. } = node else { return false };
        let (start, len, min_a, min_b) = match dir {
            Dir::Right => (r.x, r.w, a.min_size(g, top).0, b.min_size(g, top).0),
            Dir::Down => (r.y, r.h, a.min_size(g, top).1, b.min_size(g, false).1),
        };
        let Some((ra, _, _)) = node.split_rects(r, g, top) else {
            return false;
        };
        let before = if *dir == Dir::Right { ra.w } else { ra.h };
        let d = g.divider.clamp(0, len.max(0));
        let avail = len.max(0) - d;
        if avail <= 0 {
            return false;
        }
        let mut want = to.saturating_sub(start).clamp(0, avail);
        if min_a.saturating_add(min_b) <= avail {
            want = want.clamp(min_a, avail - min_b);
        }
        if want == before {
            return false;
        }
        let ratio = want as f32 / avail as f32;
        if let Some(Node::Split { ratio: r, .. }) = self.root.node_at(&path.0) {
            *r = ratio;
        }
        true
    }

    /// Moves the divider nearest the focused pane that runs across `dir`
    /// (the innermost split of that kind around it) by `by` pixels that
    /// way: one cell, for the keyboard. Returns false (beep) when there is
    /// no such divider or it can't move further.
    pub fn resize(&mut self, dir: Direction, by: i32, g: &Geometry) -> bool {
        let mut path = Vec::new();
        if !self.root.path_to(self.focused, &mut path) {
            return false;
        }
        // The innermost split of the right kind on the way down.
        let mut found = None;
        let mut node = &self.root;
        for (i, &second) in path.iter().enumerate() {
            let Node::Split { dir: d, a, b, .. } = node else { break };
            if *d == dir.axis() {
                found = Some(i);
            }
            node = if second { b } else { a };
        }
        let Some(depth) = found else { return false };
        let split = SplitPath(path[..depth].to_vec());
        let Some(line) = self.full_layout(g).dividers.iter().find(|d| d.path == split).map(|d| d.line) else {
            return false;
        };
        let to = match dir {
            Direction::Left => line.x.saturating_sub(by),
            Direction::Right => line.x.saturating_add(by),
            Direction::Up => line.y.saturating_sub(by),
            Direction::Down => line.y.saturating_add(by),
        };
        self.drag(&split, to, g)
    }

    /// Equal shares: each run of side-by-side (or stacked) panes gets the
    /// same width (or height), to within a divider.
    pub fn equalize(&mut self) {
        self.root.equalize(None);
    }

    /// The pane next to the focused one in `dir`: of those across a
    /// divider from it, the one level with its centre, else the one
    /// sharing the most of its edge (as Ghostty and iTerm2 do).
    pub fn neighbour(&self, dir: Direction, g: &Geometry) -> Option<PaneId> {
        let layout = self.full_layout(g);
        let f = layout.rect(self.focused)?;
        let d = g.divider.max(0);
        let overlap = |a0: i32, a1: i32, b0: i32, b1: i32| a1.min(b1) - a0.max(b0);
        let mut best: Option<(bool, i32, PaneId)> = None;
        for &(id, r) in &layout.panes {
            if id == self.focused {
                continue;
            }
            let (gap, over, centred) = match dir {
                Direction::Left => (
                    f.x - r.right(),
                    overlap(f.y, f.bottom(), r.y, r.bottom()),
                    (r.y..r.bottom()).contains(&(f.y + f.h / 2)),
                ),
                Direction::Right => (
                    r.x - f.right(),
                    overlap(f.y, f.bottom(), r.y, r.bottom()),
                    (r.y..r.bottom()).contains(&(f.y + f.h / 2)),
                ),
                Direction::Up => (
                    f.y - r.bottom(),
                    overlap(f.x, f.right(), r.x, r.right()),
                    (r.x..r.right()).contains(&(f.x + f.w / 2)),
                ),
                Direction::Down => (
                    r.y - f.bottom(),
                    overlap(f.x, f.right(), r.x, r.right()),
                    (r.x..r.right()).contains(&(f.x + f.w / 2)),
                ),
            };
            if !(0..=d).contains(&gap) || over <= 0 {
                continue;
            }
            if best.is_none_or(|(c, o, _)| (centred, over) > (c, o)) {
                best = Some((centred, over, id));
            }
        }
        best.map(|(_, _, id)| id)
    }

    /// The pane after the focused one in tree order, wrapping.
    pub fn next(&self) -> PaneId {
        let ids = self.ids();
        let i = ids.iter().position(|&p| p == self.focused).unwrap_or(0);
        ids[(i + 1) % ids.len()]
    }

    /// The pane before the focused one in tree order, wrapping.
    pub fn previous(&self) -> PaneId {
        let ids = self.ids();
        let i = ids.iter().position(|&p| p == self.focused).unwrap_or(0);
        ids[(i + ids.len() - 1) % ids.len()]
    }

    /// Gives `id` focus; focusing another pane ends a zoom. False if `id`
    /// isn't one of these panes.
    pub fn focus(&mut self, id: PaneId) -> bool {
        if !self.contains(id) {
            return false;
        }
        if self.zoomed.is_some_and(|z| z != id) {
            self.zoomed = None;
        }
        self.focused = id;
        true
    }

    /// Zooms the focused pane to fill the window, or ends the zoom. False
    /// (beep) with only one pane.
    pub fn toggle_zoom(&mut self) -> bool {
        if self.zoomed.take().is_some() {
            return true;
        }
        if matches!(self.root, Node::Leaf(_)) {
            return false;
        }
        self.zoomed = Some(self.focused);
        true
    }

    /// The layout to save for a restart, with each pane's working
    /// directory and command.
    pub fn save(&self, mut pane: impl FnMut(PaneId) -> SavedPane) -> SavedPanes {
        let ids = self.ids();
        let index = |id: PaneId| ids.iter().position(|&p| p == id);
        SavedPanes {
            root: self.root.save(&mut pane),
            focused: index(self.focused).unwrap_or(0),
            zoomed: self.zoomed.and_then(index),
        }
    }

    /// Panes laid out as saved, with fresh ids, and each pane's working
    /// directory and command in tree order. A focused or zoomed index out
    /// of range is the first pane, or no zoom.
    pub fn restore(saved: &SavedPanes) -> (Panes, Vec<(PaneId, SavedPane)>) {
        let mut panes = Vec::new();
        let root = Node::restore(&saved.root, &mut panes);
        let focused = panes.get(saved.focused).unwrap_or(&panes[0]).0;
        let zoomed = saved.zoomed.and_then(|z| panes.get(z)).map(|p| p.0).filter(|_| panes.len() > 1);
        (Panes { root, focused, zoomed }, panes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn geometry(w: i32, h: i32) -> Geometry {
        Geometry {
            bounds: PxRect { x: 0, y: 0, w, h },
            top_inset: 0,
            min: (20, 10),
            divider: 1,
        }
    }

    fn leaf(id: PaneId) -> Box<Node> {
        Box::new(Node::Leaf(id))
    }

    fn split(dir: Dir, a: Box<Node>, b: Box<Node>) -> Box<Node> {
        Box::new(Node::Split { dir, ratio: 0.5, a, b })
    }

    #[allow(clippy::boxed_local)]
    fn panes(root: Box<Node>, focused: PaneId) -> Panes {
        Panes {
            root: *root,
            focused,
            zoomed: None,
        }
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> PxRect {
        PxRect { x, y, w, h }
    }

    #[test]
    fn one_pane_fills_the_bounds() {
        let p = Panes::new();
        let g = geometry(800, 600);
        let l = p.layout(&g);
        assert_eq!(l.panes, [(p.focused(), g.bounds)]);
        assert!(l.dividers.is_empty());
        assert_eq!(p.min_size(&g), (20, 10));
        assert_eq!(p.next(), p.focused());
        assert_eq!(p.previous(), p.focused());
    }

    #[test]
    fn splits_share_the_space_around_a_divider() {
        let mut p = Panes::new();
        let first = p.focused();
        let g = Geometry {
            top_inset: 28,
            divider: 2,
            ..geometry(801, 600)
        };
        let second = p.split(Dir::Right, &g).unwrap();
        assert_eq!(p.focused(), second, "the new pane has focus");
        let l = p.layout(&g);
        assert_eq!(l.panes, [(first, rect(0, 0, 400, 600)), (second, rect(402, 0, 399, 600))]);
        assert_eq!(l.dividers.len(), 1);
        let d = &l.dividers[0];
        assert_eq!(d.line, rect(400, 0, 2, 600));
        assert_eq!(d.grab, rect(394, 28, 14, 572), "3 pt each side, below the title bar");
        assert_eq!(d.path, SplitPath(vec![]));
        let third = p.split(Dir::Down, &g).unwrap();
        let l = p.layout(&g);
        assert_eq!(l.rect(second), Some(rect(402, 0, 399, 299)));
        assert_eq!(l.rect(third), Some(rect(402, 301, 399, 299)));
        assert_eq!(l.dividers[1].path, SplitPath(vec![true]));
        assert_eq!(l.dividers[1].line, rect(402, 299, 399, 2));
        // Two side by side, the right one stacked: min width 20 + 2 + 20,
        // min height (10 + inset) + 2 + 10.
        assert_eq!(p.min_size(&g), (42, 50));
    }

    #[test]
    fn split_refuses_rather_than_break_the_minimum() {
        let g = geometry(40, 100);
        let mut p = Panes::new();
        assert_eq!(p.split(Dir::Right, &g), Err(TooSmall));
        assert_eq!(p.len(), 1);
        let g = geometry(41, 100);
        let first = p.focused();
        assert!(p.split(Dir::Right, &g).is_ok());
        assert_eq!(p.split(Dir::Right, &g), Err(TooSmall));
        // The top inset counts for panes at the top only.
        let g = Geometry {
            top_inset: 30,
            ..geometry(41, 50)
        };
        assert!(p.focus(first));
        assert_eq!(p.split(Dir::Down, &g), Err(TooSmall), "40 + 1 + 10 > 50");
        let g = Geometry {
            top_inset: 30,
            ..geometry(41, 51)
        };
        assert!(p.split(Dir::Down, &g).is_ok());
    }

    #[test]
    fn close_gives_the_sibling_the_space() {
        let g = geometry(900, 600);
        // [1 | [2 / 3]]
        let mut p = panes(split(Dir::Right, leaf(1), split(Dir::Down, leaf(2), leaf(3))), 2);
        assert_eq!(p.close(2), Some(3), "focus goes to the sibling");
        assert_eq!(p.root, *split(Dir::Right, leaf(1), leaf(3)));
        assert_eq!(p.layout(&g).rect(3), Some(rect(451, 0, 449, 600)));
        assert_eq!(p.close(1), Some(3));
        assert_eq!(p.close(3), None, "the last pane closes the window");
        assert_eq!(p.ids(), [3]);
        // Closing a pane without focus keeps the focus where it is; the
        // sibling's first pane gets it when it had it.
        let mut p = panes(split(Dir::Right, leaf(1), split(Dir::Down, leaf(2), leaf(3))), 3);
        assert_eq!(p.close(1), Some(3));
        let mut p = panes(split(Dir::Right, leaf(1), split(Dir::Down, leaf(2), leaf(3))), 1);
        assert_eq!(p.close(1), Some(2));
        assert_eq!(p.close(42), Some(2), "not a pane: nothing changes");
    }

    #[test]
    fn zoom_shows_one_pane_until_split_close_or_focus() {
        let g = geometry(900, 600);
        let mut p = panes(split(Dir::Right, leaf(1), leaf(2)), 1);
        assert!(p.toggle_zoom());
        assert_eq!(p.layout(&g).panes, [(1, g.bounds)]);
        assert!(p.layout(&g).dividers.is_empty());
        assert!(p.toggle_zoom(), "again restores");
        assert_eq!(p.layout(&g).panes.len(), 2);
        // Focusing another pane ends it; focusing the zoomed one doesn't.
        p.toggle_zoom();
        assert!(p.focus(1));
        assert_eq!(p.zoomed(), Some(1));
        assert!(p.focus(2));
        assert_eq!(p.zoomed(), None);
        // A split ends it.
        p.toggle_zoom();
        p.split(Dir::Down, &g).unwrap();
        assert_eq!(p.zoomed(), None);
        // Closing the zoomed pane ends it; closing another doesn't.
        let mut p = panes(split(Dir::Right, leaf(1), split(Dir::Down, leaf(2), leaf(3))), 1);
        p.toggle_zoom();
        p.close(3);
        assert_eq!(p.zoomed(), Some(1));
        p.close(1);
        assert_eq!(p.zoomed(), None);
        // One pane can't zoom.
        assert!(!Panes::new().toggle_zoom());
        assert!(!p.focus(99));
    }

    /// 1 2
    /// 3 4
    fn two_by_two() -> Panes {
        panes(
            split(Dir::Right, split(Dir::Down, leaf(1), leaf(3)), split(Dir::Down, leaf(2), leaf(4))),
            1,
        )
    }

    #[test]
    fn neighbours_by_geometry() {
        use Direction::*;
        let g = geometry(1001, 601);
        // Each row: focused, direction, neighbour.
        type Rows = &'static [(PaneId, Direction, Option<PaneId>)];
        let table: &[(&str, Panes, Rows)] = &[
            (
                "2x2",
                two_by_two(),
                &[
                    (1, Right, Some(2)),
                    (1, Down, Some(3)),
                    (1, Left, None),
                    (1, Up, None),
                    (4, Left, Some(3)),
                    (4, Up, Some(2)),
                    (3, Right, Some(4)),
                    (2, Down, Some(4)),
                ],
            ),
            (
                // 1 2
                // 1 3
                "L-shaped",
                panes(split(Dir::Right, leaf(1), split(Dir::Down, leaf(2), leaf(3))), 1),
                &[
                    (1, Right, Some(2)),
                    (3, Left, Some(1)),
                    (2, Left, Some(1)),
                    (2, Down, Some(3)),
                    (3, Up, Some(2)),
                ],
            ),
            (
                // 1 2
                // 3 3   (2 is split again: 2 | 4 on the right)
                "nested",
                panes(
                    split(Dir::Down, split(Dir::Right, leaf(1), split(Dir::Right, leaf(2), leaf(4))), leaf(3)),
                    1,
                ),
                &[
                    (1, Right, Some(2)),
                    (2, Right, Some(4)),
                    (4, Left, Some(2)),
                    (4, Down, Some(3)),
                    (2, Down, Some(3)),
                    // 3 spans the width; its centre (x 500) is on the
                    // divider, so the pane sharing the most edge: 1.
                    (3, Up, Some(1)),
                    (4, Right, None),
                ],
            ),
        ];
        for (name, p, rows) in table {
            for &(from, dir, want) in rows.iter() {
                let mut p = p.clone();
                assert!(p.focus(from));
                assert_eq!(p.neighbour(dir, &g), want, "{name}: from {from} {dir:?}");
            }
        }
        // With no pane level with the centre, the one sharing the most edge.
        let mut p = panes(
            split(
                Dir::Right,
                Box::new(Node::Split {
                    dir: Dir::Down,
                    ratio: 0.3,
                    a: leaf(1),
                    b: leaf(2),
                }),
                Box::new(Node::Split {
                    dir: Dir::Down,
                    ratio: 0.6,
                    a: leaf(3),
                    b: leaf(4),
                }),
            ),
            2,
        );
        // 2 is y 181..601, centre 391: 4 (361..601) holds it.
        assert_eq!(p.neighbour(Right, &g), Some(4));
        p.focus(3);
        // 3 is y 0..360, centre 180: on the divider between 1 and 2; 1
        // shares 180 px of edge, 2 179.
        assert_eq!(p.neighbour(Left, &g), Some(1));
    }

    #[test]
    fn next_and_previous_wrap_in_tree_order() {
        let mut p = two_by_two();
        assert_eq!(p.ids(), [1, 3, 2, 4]);
        let mut seen = vec![];
        for _ in 0..4 {
            let n = p.next();
            seen.push(n);
            p.focus(n);
        }
        assert_eq!(seen, [3, 2, 4, 1]);
        assert_eq!(p.previous(), 4);
    }

    #[test]
    fn equalize_gives_each_run_equal_shares() {
        let g = geometry(902, 600);
        // [1 | [2 | 3]] (a run of three) beside nothing else.
        let mut p = panes(split(Dir::Right, leaf(1), split(Dir::Right, leaf(2), leaf(3))), 1);
        p.equalize();
        let widths: Vec<i32> = p.layout(&g).panes.iter().map(|(_, r)| r.w).collect();
        assert!(widths.iter().all(|w| (w - 300).abs() <= 1), "{widths:?}");
        // A stacked split inside a run counts once.
        let mut p = panes(split(Dir::Right, leaf(1), split(Dir::Down, leaf(2), leaf(3))), 1);
        p.equalize();
        let l = p.layout(&g);
        assert_eq!(l.rect(1).unwrap().w, 451, "half of 901, rounded");
        assert_eq!(l.rect(2).unwrap().h, 300);
    }

    #[test]
    fn drag_and_resize_move_dividers_within_the_minimum() {
        let g = geometry(901, 600);
        let mut p = panes(split(Dir::Right, leaf(1), split(Dir::Down, leaf(2), leaf(3))), 2);
        assert!(p.drag(&SplitPath(vec![]), 300, &g));
        assert_eq!(p.layout(&g).rect(1), Some(rect(0, 0, 300, 600)));
        assert!(!p.drag(&SplitPath(vec![]), 300, &g), "no move");
        // Clamped to the minimum on either side.
        assert!(p.drag(&SplitPath(vec![]), 0, &g));
        assert_eq!(p.layout(&g).rect(1).unwrap().w, 20);
        assert!(p.drag(&SplitPath(vec![]), 5000, &g));
        assert_eq!(p.layout(&g).rect(2).unwrap().w, 20);
        assert!(!p.drag(&SplitPath(vec![false]), 10, &g), "a pane, not a split");
        assert!(!p.drag(&SplitPath(vec![true, true, true]), 10, &g));
        // The keyboard: the nearest divider across the direction, by `by`.
        p.drag(&SplitPath(vec![]), 450, &g);
        assert!(p.resize(Direction::Left, 10, &g));
        assert_eq!(p.layout(&g).rect(1).unwrap().w, 440);
        assert!(p.resize(Direction::Down, 10, &g), "2 / 3's divider");
        assert_eq!(p.layout(&g).rect(2).unwrap().h, 310);
        assert!(p.focus(1));
        assert!(!p.resize(Direction::Up, 10, &g), "nothing stacked around 1");
        assert!(p.resize(Direction::Right, 10, &g));
        assert_eq!(p.layout(&g).rect(1).unwrap().w, 450);
        // Window resizes keep the shares.
        let wide = geometry(1801, 600);
        assert_eq!(p.layout(&wide).rect(1).unwrap().w, 900);
    }

    #[test]
    fn grab_zones_stay_out_of_the_title_bar() {
        let g = Geometry {
            top_inset: 56,
            divider: 2,
            min: (40, 20),
            ..geometry(1000, 600)
        };
        let p = two_by_two();
        let l = p.layout(&g);
        for d in &l.dividers {
            assert!(d.grab.y >= 56, "{d:?}");
            assert!(d.grab.contains(d.line.x, d.line.y.max(56)), "{d:?}");
        }
        // Top panes are taller by the inset's share of the minimum.
        assert_eq!(p.min_size(&g), (82, 56 + 20 + 2 + 20));
    }

    #[test]
    fn save_and_restore() {
        let g = geometry(1001, 601);
        let mut p = two_by_two();
        p.drag(&SplitPath(vec![]), 300, &g);
        p.focus(4);
        p.toggle_zoom();
        let saved = p.save(|id| SavedPane {
            cwd: Some(format!("/p{id}")),
            command: (id == 2).then(|| "nvim 'a b.rs'".into()),
        });
        assert_eq!((saved.focused, saved.zoomed), (3, Some(3)));
        let json = serde_json::to_string(&saved).unwrap();
        let back: SavedPanes = serde_json::from_str(&json).unwrap();
        assert_eq!(back, saved);
        // The fixture's hand-picked ids 1–4 didn't come from the counter;
        // move it past them, as it would be in the app (and when this test
        // runs alone in its own process).
        NEXT_ID.fetch_max(5, Ordering::Relaxed);
        let (r, cwds) = Panes::restore(&back);
        let dirs: Vec<&str> = cwds.iter().map(|(_, c)| c.cwd.as_deref().unwrap()).collect();
        assert_eq!(dirs, ["/p1", "/p3", "/p2", "/p4"]);
        let commands: Vec<Option<&str>> = cwds.iter().map(|(_, c)| c.command.as_deref()).collect();
        assert_eq!(commands, [None, None, Some("nvim 'a b.rs'"), None]);
        assert_eq!(r.focused(), cwds[3].0);
        assert_eq!(r.zoomed(), Some(cwds[3].0));
        assert!(cwds.iter().all(|(id, _)| !p.contains(*id)), "fresh ids");
        // Same shape and shares.
        let unzoomed = |mut p: Panes| {
            p.zoomed = None;
            p.layout(&g).panes.into_iter().map(|(_, r)| r).collect::<Vec<_>>()
        };
        assert_eq!(unzoomed(r), unzoomed(p));
        // Out-of-range indexes and bad ratios are made safe.
        let bad = SavedPanes {
            root: SavedNode::Split {
                dir: Dir::Down,
                ratio: f32::NAN,
                a: Box::new(SavedNode::Pane(SavedPane { cwd: None, command: None })),
                b: Box::new(SavedNode::Pane(SavedPane { cwd: None, command: None })),
            },
            focused: 7,
            zoomed: Some(9),
        };
        let (r, cwds) = Panes::restore(&bad);
        assert_eq!(r.focused(), cwds[0].0);
        assert_eq!(r.zoomed(), None);
        assert_eq!(r.layout(&g).rect(cwds[0].0).unwrap().h, 300);
    }

    // Property tests.

    #[derive(Clone, Debug)]
    enum Op {
        Split(Dir),
        Close(usize),
        Focus(usize),
        Drag(usize, i32),
        Resize(Direction, i32),
        Equalize,
        Zoom,
        Neighbour(Direction),
        Window(i32, i32),
    }

    fn direction() -> impl Strategy<Value = Direction> {
        prop_oneof![
            Just(Direction::Left),
            Just(Direction::Right),
            Just(Direction::Up),
            Just(Direction::Down)
        ]
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            4 => prop_oneof![Just(Dir::Right), Just(Dir::Down)].prop_map(Op::Split),
            2 => any::<usize>().prop_map(Op::Close),
            2 => any::<usize>().prop_map(Op::Focus),
            2 => (any::<usize>(), -100..3000i32).prop_map(|(i, to)| Op::Drag(i, to)),
            1 => (direction(), 1..80i32).prop_map(|(d, by)| Op::Resize(d, by)),
            1 => Just(Op::Equalize),
            1 => Just(Op::Zoom),
            1 => direction().prop_map(Op::Neighbour),
            1 => (0..3000i32, 0..2000i32).prop_map(|(w, h)| Op::Window(w, h)),
        ]
    }

    /// A window: its size and scale, the cells' minimum and the title bar.
    fn window() -> impl Strategy<Value = Geometry> {
        (1..=3i32, 100..3000i32, 100..2000i32, 10..60i32, 6..30i32, 0..40i32).prop_map(|(scale, w, h, cw, ch, inset)| Geometry {
            bounds: PxRect { x: 0, y: 0, w, h },
            top_inset: inset * scale,
            min: (cw * scale, ch * scale),
            divider: scale,
        })
    }

    /// What every layout must be: panes and divider lines tile the bounds
    /// exactly, with no overlap; no pane under the minimum unless the
    /// window is; the tree's panes are the layout's; grab zones below the
    /// title bar and within the bounds.
    fn check(p: &Panes, g: &Geometry) -> Result<(), TestCaseError> {
        let mut full = p.clone();
        full.zoomed = None;
        let l = full.layout(g);
        let ids = p.ids();
        prop_assert_eq!(l.panes.iter().map(|(id, _)| *id).collect::<Vec<_>>(), ids.clone());
        prop_assert!(ids.contains(&p.focused()));
        prop_assert_eq!(l.dividers.len(), ids.len() - 1);
        let mut rects: Vec<PxRect> = l.panes.iter().map(|(_, r)| *r).collect();
        rects.extend(l.dividers.iter().map(|d| d.line));
        let b = g.bounds;
        let mut area = 0i64;
        for (i, r) in rects.iter().enumerate() {
            prop_assert!(r.w >= 0 && r.h >= 0, "{:?}", r);
            prop_assert!(
                r.x >= b.x && r.y >= b.y && r.right() <= b.right() && r.bottom() <= b.bottom(),
                "{:?}",
                r
            );
            area += r.w as i64 * r.h as i64;
            for s in &rects[i + 1..] {
                let ox = r.right().min(s.right()) - r.x.max(s.x);
                let oy = r.bottom().min(s.bottom()) - r.y.max(s.y);
                prop_assert!(ox <= 0 || oy <= 0, "{:?} overlaps {:?}", r, s);
            }
        }
        prop_assert_eq!(area, b.w as i64 * b.h as i64, "tiles the bounds");
        let (mw, mh) = p.min_size(g);
        if mw <= b.w && mh <= b.h {
            for (id, r) in &l.panes {
                let inset = if r.y == b.y { g.top_inset } else { 0 };
                prop_assert!(r.w >= g.min.0 && r.h >= g.min.1 + inset, "pane {} {:?} under {:?}", id, r, g.min);
            }
        }
        for d in &l.dividers {
            prop_assert!(d.grab.h == 0 || d.grab.y >= b.y + g.top_inset, "{:?}", d);
            prop_assert!(
                d.grab.x >= b.x && d.grab.right() <= b.right() && d.grab.bottom() <= b.bottom(),
                "{:?}",
                d
            );
        }
        // `next` visits every pane once.
        let mut walk = p.clone();
        let mut seen = vec![walk.focused()];
        for _ in 1..ids.len() {
            let n = walk.next();
            walk.focus(n);
            seen.push(n);
        }
        seen.sort();
        let mut sorted = ids;
        sorted.sort();
        prop_assert_eq!(seen, sorted);
        prop_assert_eq!(walk.next(), p.focused());
        // Zoomed: the zoomed pane alone, filling the window.
        if let Some(z) = p.zoomed() {
            prop_assert!(p.contains(z));
            prop_assert_eq!(p.layout(g).panes, vec![(z, b)]);
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

        #[test]
        fn layouts_hold_under_random_changes(g0 in window(), ops in prop::collection::vec(op(), 1..60)) {
            let mut g = g0;
            let mut p = Panes::new();
            check(&p, &g)?;
            for op in ops {
                let before = p.clone();
                match op {
                    Op::Split(dir) => match p.split(dir, &g) {
                        Ok(new) => {
                            prop_assert_eq!(p.focused(), new);
                            prop_assert_eq!(p.zoomed(), None);
                            prop_assert_eq!(p.len(), before.len() + 1);
                        }
                        Err(TooSmall) => prop_assert_eq!(&p, &before),
                    },
                    Op::Close(i) => {
                        let ids = p.ids();
                        let id = ids[i % ids.len()];
                        match p.close(id) {
                            None => prop_assert_eq!(ids.len(), 1),
                            Some(f) => {
                                prop_assert_eq!(f, p.focused());
                                prop_assert!(!p.contains(id));
                                prop_assert!(p.zoomed() != Some(id));
                            }
                        }
                    }
                    Op::Focus(i) => {
                        let ids = p.ids();
                        let id = ids[i % ids.len()];
                        prop_assert!(p.focus(id));
                        if before.zoomed().is_some_and(|z| z != id) {
                            prop_assert_eq!(p.zoomed(), None);
                        }
                    }
                    Op::Drag(i, to) => {
                        let l = p.full_layout(&g);
                        if !l.dividers.is_empty() {
                            let d = &l.dividers[i % l.dividers.len()];
                            p.drag(&d.path.clone(), to, &g);
                        }
                    }
                    Op::Resize(dir, by) => {
                        p.resize(dir, by, &g);
                    }
                    Op::Equalize => p.equalize(),
                    Op::Zoom => {
                        p.toggle_zoom();
                    }
                    Op::Neighbour(dir) => {
                        if let Some(n) = p.neighbour(dir, &g) {
                            prop_assert!(n != p.focused() && p.contains(n));
                            prop_assert!(p.focus(n));
                        }
                    }
                    Op::Window(w, h) => g.bounds = PxRect { w, h, ..g.bounds },
                }
                check(&p, &g)?;
            }
        }

        #[test]
        fn closing_every_pane_in_any_order_ends_empty(
            dirs in prop::collection::vec(prop_oneof![Just(Dir::Right), Just(Dir::Down)], 0..16),
            focus in prop::collection::vec(any::<usize>(), 16),
            order in any::<u64>(),
        ) {
            let g = geometry(4000, 4000);
            let mut p = Panes::new();
            for (dir, f) in dirs.iter().zip(&focus) {
                let ids = p.ids();
                p.focus(ids[f % ids.len()]);
                let _ = p.split(*dir, &g);
            }
            let mut ids = p.ids();
            // A shuffle from the seed.
            let mut s = order;
            for i in (1..ids.len()).rev() {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ids.swap(i, (s >> 33) as usize % (i + 1));
            }
            let last = ids.pop().unwrap();
            for id in ids {
                prop_assert!(p.close(id).is_some());
                check(&p, &g)?;
            }
            prop_assert_eq!(p.ids(), vec![last]);
            prop_assert_eq!(p.close(last), None);
        }

        #[test]
        fn restore_gives_back_the_same_layout(g in window(), dirs in prop::collection::vec(any::<(bool, usize, i32)>(), 0..10)) {
            let mut p = Panes::new();
            for (down, f, to) in dirs {
                let ids = p.ids();
                p.focus(ids[f % ids.len()]);
                let _ = p.split(if down { Dir::Down } else { Dir::Right }, &g);
                let l = p.layout(&g);
                if let Some(d) = l.dividers.get(f % l.dividers.len().max(1)) {
                    p.drag(&d.path.clone(), to.rem_euclid(3000), &g);
                }
            }
            let json = serde_json::to_string(&p.save(|_| SavedPane::default())).unwrap();
            let (r, _) = Panes::restore(&serde_json::from_str(&json).unwrap());
            let rects = |p: &Panes| p.layout(&g).panes.into_iter().map(|(_, r)| r).collect::<Vec<_>>();
            prop_assert_eq!(rects(&r), rects(&p));
            prop_assert_eq!(r.ids().iter().position(|&i| i == r.focused()), p.ids().iter().position(|&i| i == p.focused()));
        }
    }
}
