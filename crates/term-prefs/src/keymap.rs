//! The Keys page's shortcut list: every menu action with the shortcuts
//! it has, grouped by menu; what a recorded shortcut means; and the
//! bindings a change makes (replace, add, swap, clear, reset), worked
//! out from the bindings in effect so the clash is found before
//! anything is written. Pure: the app turns the result into edits.
//!
//! Chords are in the app's canonical spelling (`ctrl+alt+shift+cmd+k`,
//! [`crate::Host::parse_chord`]).

use crate::host::Action;
use crate::layers::{Bind, Bound, Origin};

/// The menus, in the menu bar's order, and the name shown for each: the
/// groups of the shortcut list. `App` is the app's own menu.
pub const MENUS: [(&str, &str); 5] = [
    ("Shell", "Shell"),
    ("Edit", "Edit"),
    ("View", "View"),
    ("Window", "Window"),
    ("App", "Terminal Decline"),
];

/// A canonical chord's modifiers (ctrl, alt, shift, cmd) and key.
fn split(chord: &str) -> ([bool; 4], &str) {
    let mut mods = [false; 4];
    let mut rest = chord;
    for (i, m) in ["ctrl+", "alt+", "shift+", "cmd+"].iter().enumerate() {
        // `ctrl++` can't happen: `+` is spelled `plus`.
        if rest.len() > m.len()
            && let Some(r) = rest.strip_prefix(m)
        {
            mods[i] = true;
            rest = r;
        }
    }
    (mods, rest)
}

/// A canonical chord as macOS menus write it: `⌃⌥⇧⌘` then the key
/// (`⌘T`, `⇧⌘↩`, `⌃⇥`, `⌥⌘←`, `F5`), as the menus show it.
pub fn symbols(chord: &str) -> String {
    let (mods, key) = split(chord);
    let mut out: String = mods
        .iter()
        .zip(['⌃', '⌥', '⇧', '⌘'])
        .filter(|(on, _)| **on)
        .map(|(_, s)| s)
        .collect();
    let named = match key {
        "space" => Some("Space"),
        "plus" => Some("+"),
        "enter" => Some("↩"),
        "tab" => Some("⇥"),
        "escape" => Some("⎋"),
        "backspace" => Some("⌫"),
        "delete" => Some("⌦"),
        "insert" => Some("⎀"),
        "up" => Some("↑"),
        "down" => Some("↓"),
        "left" => Some("←"),
        "right" => Some("→"),
        "home" => Some("↖"),
        "end" => Some("↘"),
        "page_up" => Some("⇞"),
        "page_down" => Some("⇟"),
        _ => None,
    };
    match named {
        Some(n) => out.push_str(n),
        None if is_function_key(key) => out.push_str(&key.to_uppercase()),
        None => out.extend(key.chars().flat_map(char::to_uppercase)),
    }
    out
}

fn is_function_key(key: &str) -> bool {
    key.len() > 1 && key.starts_with('f') && key[1..].bytes().all(|b| b.is_ascii_digit())
}

/// What a shortcut recorded by the app means for the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// A shortcut to use.
    Chord(String),
    /// ⌫ (or ⌦) on its own: clear the shortcut, as in macOS's key
    /// recorders.
    Clear,
    /// `t` on its own: type the shortcut instead.
    TypeIt,
    /// A key with no ⌘, ⌃ or ⌥ (Shift alone included): it types, so it
    /// can't be a shortcut (F-keys can).
    Plain(String),
}

/// What `chord`, pressed while recording, means.
pub fn recorded(chord: &str) -> Recorded {
    let ([ctrl, alt, shift, cmd], key) = split(chord);
    let bare = !ctrl && !alt && !cmd;
    match key {
        "backspace" | "delete" if bare && !shift => Recorded::Clear,
        "t" if bare && !shift => Recorded::TypeIt,
        _ if bare && !is_function_key(key) => Recorded::Plain(chord.to_string()),
        _ => Recorded::Chord(chord.to_string()),
    }
}

/// Whether a typed or recorded chord can be a shortcut: it needs ⌘, ⌃
/// or ⌥, or is an F-key.
pub fn usable(chord: &str) -> bool {
    let ([ctrl, alt, _, cmd], key) = split(chord);
    ctrl || alt || cmd || is_function_key(key)
}

/// An action and its shortcuts, as the list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionKeys {
    pub name: String,
    pub label: String,
    /// The group's name ([`MENUS`]); an unknown menu is shown as is.
    pub menu: String,
    /// The chords that do it now, the menus' default ones first.
    pub chords: Vec<String>,
    /// The chords it has with no config.
    pub defaults: Vec<String>,
    /// Where its shortcuts come from: set in the edited file if any of
    /// them is (or one of its defaults was changed there), else the
    /// layer below or above, else the default.
    pub origin: Origin,
}

/// The chords bound to action `name`.
pub fn chords_of(binds: &[Bind], name: &str) -> Vec<String> {
    binds
        .iter()
        .filter(|b| matches!(&b.bound, Bound::Action(a) if a == name))
        .map(|b| b.chord.clone())
        .collect()
}

/// Every action with its shortcuts, grouped by menu in [`MENUS`]'
/// order (unknown menus last), in `actions`' order within a group.
pub fn action_keys(actions: &[Action], binds: &[Bind]) -> Vec<ActionKeys> {
    let rank = |menu: &str| MENUS.iter().position(|(m, _)| *m == menu).unwrap_or(MENUS.len());
    let mut out: Vec<ActionKeys> = actions
        .iter()
        .map(|a| {
            let related = binds
                .iter()
                .filter(|b| matches!(&b.bound, Bound::Action(x) if *x == a.name) || b.default.as_deref() == Some(a.name.as_str()));
            let mut origin = Origin::Default;
            for b in related {
                let better = matches!(
                    (&b.origin, &origin),
                    (Origin::Here, _)
                        | (Origin::Overridden(_), Origin::Default | Origin::Inherited(_))
                        | (Origin::Inherited(_), Origin::Default)
                );
                if better {
                    origin = b.origin.clone();
                }
            }
            ActionKeys {
                name: a.name.clone(),
                label: a.label.clone(),
                menu: MENUS
                    .iter()
                    .find(|(m, _)| *m == a.menu)
                    .map_or_else(|| a.menu.clone(), |(_, shown)| shown.to_string()),
                chords: chords_of(binds, &a.name),
                defaults: binds
                    .iter()
                    .filter(|b| b.default.as_deref() == Some(a.name.as_str()))
                    .map(|b| b.chord.clone())
                    .collect(),
                origin,
            }
        })
        .collect();
    out.sort_by_key(|a| rank(actions.iter().find(|x| x.name == a.name).map_or("", |x| x.menu.as_str())));
    out
}

/// What `chord` does now, seen from action `name` choosing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Taken {
    /// Nothing: free to use.
    Free,
    /// It is already this action's.
    Same,
    /// Another action's (by name).
    Action(String),
    /// It sends text.
    Text(String),
}

pub fn taken(binds: &[Bind], chord: &str, name: &str) -> Taken {
    match binds.iter().find(|b| b.chord == chord).map(|b| &b.bound) {
        Some(Bound::Action(a)) if a == name => Taken::Same,
        Some(Bound::Action(a)) => Taken::Action(a.clone()),
        Some(Bound::Text(t)) => Taken::Text(t.clone()),
        Some(Bound::Unbound) | None => Taken::Free,
    }
}

/// How a new shortcut goes on an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// It becomes the action's only shortcut; whatever had it loses it.
    Replace,
    /// It is added to the action's shortcuts.
    Add,
    /// It becomes the action's only shortcut, and the action that had it
    /// takes this one's old shortcuts.
    Swap,
}

/// The bindings that put `chord` on action `name`, `how`: each chord and
/// what it should do. The first is always `chord` itself.
pub fn plan(binds: &[Bind], name: &str, chord: &str, how: How) -> Vec<(String, Bound)> {
    let mut out = vec![(chord.to_string(), Bound::Action(name.to_string()))];
    let old: Vec<String> = chords_of(binds, name).into_iter().filter(|c| c != chord).collect();
    let others = match (how, taken(binds, chord, name)) {
        (How::Add, _) => return out,
        (How::Swap, Taken::Action(other)) => Bound::Action(other),
        _ => Bound::Unbound,
    };
    out.extend(old.into_iter().map(|c| (c, others.clone())));
    out
}

/// The bindings that leave action `name` with no shortcut.
pub fn plan_clear(binds: &[Bind], name: &str) -> Vec<(String, Bound)> {
    chords_of(binds, name).into_iter().map(|c| (c, Bound::Unbound)).collect()
}

/// The bindings that give action `name` back the shortcuts the layers
/// below the edited file give it (`below`; the defaults for the user's
/// own file): every chord it has now or had there, as it is there.
pub fn plan_reset(binds: &[Bind], below: &[Bind], name: &str) -> Vec<(String, Bound)> {
    let mut chords = chords_of(binds, name);
    for c in chords_of(below, name) {
        if !chords.contains(&c) {
            chords.push(c);
        }
    }
    chords
        .into_iter()
        .map(|c| {
            let was = below.iter().find(|b| b.chord == c).map_or(Bound::Unbound, |b| b.bound.clone());
            (c, was)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(chord: &str, bound: Bound, default: Option<&str>, origin: Origin) -> Bind {
        Bind {
            chord: chord.into(),
            here: origin == Origin::Here,
            bound,
            default: default.map(String::from),
            origin,
        }
    }

    fn act(a: &str) -> Bound {
        Bound::Action(a.into())
    }

    /// The defaults: New Window ⌘N, New Tab ⌘T, Show Next Tab ⌘} and ⌃⇥.
    fn defaults() -> Vec<Bind> {
        vec![
            bind("cmd+n", act("new_window"), Some("new_window"), Origin::Default),
            bind("cmd+t", act("new_tab"), Some("new_tab"), Origin::Default),
            bind("cmd+}", act("next_tab"), Some("next_tab"), Origin::Default),
            bind("ctrl+tab", act("next_tab"), Some("next_tab"), Origin::Default),
        ]
    }

    fn action(name: &str, label: &str, menu: &str) -> Action {
        Action {
            name: name.into(),
            label: label.into(),
            menu: menu.into(),
        }
    }

    #[test]
    fn chords_are_shown_as_the_menus_show_them() {
        assert_eq!(symbols("cmd+t"), "⌘T");
        assert_eq!(symbols("ctrl+alt+shift+cmd+k"), "⌃⌥⇧⌘K");
        assert_eq!(symbols("shift+cmd+enter"), "⇧⌘↩");
        assert_eq!(symbols("ctrl+tab"), "⌃⇥");
        assert_eq!(symbols("alt+cmd+left"), "⌥⌘←");
        assert_eq!(symbols("cmd+}"), "⌘}");
        assert_eq!(symbols("cmd+plus"), "⌘+");
        assert_eq!(symbols("ctrl+space"), "⌃Space");
        assert_eq!(symbols("f5"), "F5");
        assert_eq!(symbols("shift+f12"), "⇧F12");
        assert_eq!(symbols("cmd+page_down"), "⌘⇟");
        assert_eq!(symbols("f"), "F", "the letter f, not a function key");
        assert_eq!(symbols("cmd+с"), "⌘С", "a non-Latin key");
    }

    #[test]
    fn what_a_recorded_key_means() {
        assert_eq!(recorded("cmd+n"), Recorded::Chord("cmd+n".into()));
        assert_eq!(recorded("ctrl+t"), Recorded::Chord("ctrl+t".into()));
        assert_eq!(recorded("alt+x"), Recorded::Chord("alt+x".into()));
        assert_eq!(recorded("f5"), Recorded::Chord("f5".into()), "F-keys can be shortcuts");
        assert_eq!(recorded("backspace"), Recorded::Clear);
        assert_eq!(recorded("delete"), Recorded::Clear);
        assert_eq!(recorded("cmd+backspace"), Recorded::Chord("cmd+backspace".into()));
        assert_eq!(recorded("t"), Recorded::TypeIt);
        assert_eq!(recorded("shift+t"), Recorded::Plain("shift+t".into()));
        assert_eq!(recorded("x"), Recorded::Plain("x".into()));
        assert_eq!(recorded("enter"), Recorded::Plain("enter".into()));
        assert!(usable("cmd+k") && usable("f1") && !usable("k") && !usable("shift+enter"));
    }

    #[test]
    fn actions_are_grouped_by_menu_with_their_shortcuts() {
        let actions = [
            action("next_tab", "Show Next Tab", "Window"),
            action("new_window", "New Window", "Shell"),
            action("settings", "Settings…", "App"),
            action("new_tab", "New Tab", "Shell"),
        ];
        let mut binds = defaults();
        // This file moves New Tab to ⌃⌘T.
        binds[1].bound = Bound::Unbound;
        binds[1].origin = Origin::Here;
        binds[1].here = true;
        binds.push(bind("ctrl+cmd+t", act("new_tab"), None, Origin::Here));
        let keys = action_keys(&actions, &binds);
        let names: Vec<&str> = keys.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(
            names,
            ["new_window", "new_tab", "next_tab", "settings"],
            "Shell, Window, then the app's"
        );
        assert_eq!(keys[0].chords, ["cmd+n"]);
        assert_eq!(keys[0].origin, Origin::Default);
        assert_eq!(keys[1].chords, ["ctrl+cmd+t"]);
        assert_eq!(keys[1].defaults, ["cmd+t"]);
        assert_eq!(keys[1].origin, Origin::Here);
        assert_eq!(keys[2].chords, ["cmd+}", "ctrl+tab"], "both, the menu's first");
        assert!(keys[3].chords.is_empty());
        assert_eq!(keys[3].menu, "Terminal Decline");
    }

    #[test]
    fn a_clash_is_found_before_anything_is_written() {
        let mut binds = defaults();
        binds.push(bind("ctrl+t", Bound::Text("\x1bt".into()), None, Origin::Here));
        binds.push(bind("cmd+d", Bound::Unbound, Some("split_right"), Origin::Here));
        assert_eq!(taken(&binds, "cmd+n", "new_tab"), Taken::Action("new_window".into()));
        assert_eq!(taken(&binds, "cmd+t", "new_tab"), Taken::Same);
        assert_eq!(taken(&binds, "ctrl+t", "new_tab"), Taken::Text("\x1bt".into()));
        assert_eq!(taken(&binds, "cmd+d", "new_tab"), Taken::Free, "unbound is free");
        assert_eq!(taken(&binds, "cmd+j", "new_tab"), Taken::Free);
    }

    #[test]
    fn replace_add_and_swap() {
        let binds = defaults();
        // New Tab on ⌘N, replacing: New Tab loses ⌘T; New Window loses ⌘N
        // by ⌘N changing hands.
        assert_eq!(
            plan(&binds, "new_tab", "cmd+n", How::Replace),
            [("cmd+n".into(), act("new_tab")), ("cmd+t".into(), Bound::Unbound)]
        );
        // Swapping: New Window takes ⌘T.
        assert_eq!(
            plan(&binds, "new_tab", "cmd+n", How::Swap),
            [("cmd+n".into(), act("new_tab")), ("cmd+t".into(), act("new_window"))]
        );
        // Adding keeps ⌘T.
        assert_eq!(plan(&binds, "new_tab", "cmd+j", How::Add), [("cmd+j".into(), act("new_tab"))]);
        // Both of Show Next Tab's go, or go over in a swap.
        assert_eq!(
            plan(&binds, "next_tab", "cmd+t", How::Swap),
            [
                ("cmd+t".into(), act("next_tab")),
                ("cmd+}".into(), act("new_tab")),
                ("ctrl+tab".into(), act("new_tab"))
            ]
        );
        // A swap with a free chord is a replace.
        assert_eq!(
            plan(&binds, "new_tab", "cmd+j", How::Swap),
            [("cmd+j".into(), act("new_tab")), ("cmd+t".into(), Bound::Unbound)]
        );
        // Recording the shortcut it has: nothing else changes.
        assert_eq!(plan(&binds, "new_tab", "cmd+t", How::Replace), [("cmd+t".into(), act("new_tab"))]);
    }

    #[test]
    fn clear_and_reset() {
        let below = defaults();
        assert_eq!(
            plan_clear(&below, "next_tab"),
            [("cmd+}".into(), Bound::Unbound), ("ctrl+tab".into(), Bound::Unbound)]
        );
        // This file moved New Tab to ⌃⌘T and unbound ⌘T: a reset puts
        // ⌘T back and gives ⌃⌘T what it had below (nothing).
        let mut binds = defaults();
        binds[1].bound = Bound::Unbound;
        binds.push(bind("ctrl+cmd+t", act("new_tab"), None, Origin::Here));
        assert_eq!(
            plan_reset(&binds, &below, "new_tab"),
            [("ctrl+cmd+t".into(), Bound::Unbound), ("cmd+t".into(), act("new_tab"))]
        );
        // Untouched: the reset is what is there already.
        assert_eq!(plan_reset(&below, &below, "new_window"), [("cmd+n".into(), act("new_window"))]);
    }
}
