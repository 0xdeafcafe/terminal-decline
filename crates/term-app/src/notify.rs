//! Desktop notifications: the ones a program asks for (OSC 9 and OSC
//! 777), a bell or a long command finishing where you aren't looking
//! (see [`crate::activity`]), and the app's own about updates; and a
//! program's progress (OSC 9;4) on the Dock icon.
//!
//! Notifications go through `UNUserNotificationCenter`, which throws when
//! the process isn't an app bundle (a bare `cargo install` binary), so
//! [`bundled`] is checked first; without a bundle, or when notifications
//! are turned off for the app, the caller falls back to the bell's
//! behaviour (bouncing the Dock icon). Permission is asked once, on the
//! first notification; Focus and the app's settings in System Settings
//! apply as for any app. Clicking one brings its window, tab and pane to
//! the front, or does what its [`Target`] says. With the test control on
//! ([`crate::control::ENV`]) nothing reaches the notification center:
//! each notification is listed in the dump instead ([`Stats::sent`]),
//! unless `DECLINE_TEST_REAL_NOTIFY=1` asks for the real one.
//!
//! The Dock icon's badge shows progress: the lowest percentage of any
//! window reporting one, "!" if one reports an error, "…" if they only
//! report that they are busy. Without progress it shows how many
//! notifications came for windows you haven't looked at since; looking at
//! one (its window becoming key in the active app) takes its count off.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker, define_class, msg_send};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSArray, NSBundle, NSError, NSObject, NSObjectProtocol, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction, UNNotificationActionOptions,
    UNNotificationCategory, UNNotificationCategoryOptions, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationResponse, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use term_core::Progress;

/// What the app knows about its permission to notify.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    /// Not asked yet.
    Unknown,
    /// Asked; the answer hasn't come.
    Asking,
    Granted,
    Denied,
    /// Not an app bundle: notifications can't be used at all.
    Unbundled,
    /// The test control is on: notifications go to the dump.
    Test,
}

/// What a click on a notification does.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// Bring window (view id) `view`'s tab to the front and focus pane
    /// `pane` in it.
    Pane { view: usize, pane: usize },
    /// Open this file or URL.
    Open(String),
    /// Nothing but bring the app forward.
    App,
}

/// A button on a notification, besides clicking it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    /// "Update": update a build from source now ([`crate::update::source`]).
    Update,
}

impl Button {
    const ALL: [Button; 1] = [Button::Update];

    fn id(self) -> &'static str {
        match self {
            Button::Update => "update",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Button::Update => "Update",
        }
    }

    fn category(self) -> String {
        format!("decline.{}", self.id())
    }
}

/// A notification to show.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Note {
    pub title: String,
    pub body: String,
    pub target: Target,
    /// Notifications with the same group are stacked together (a
    /// window's).
    pub group: Option<String>,
    /// Shown in place of the one shown before with this key, if any.
    pub key: Option<String>,
    pub button: Option<Button>,
    /// Counted on the Dock icon until this window (view id) is looked at.
    pub unseen: Option<usize>,
}

impl Note {
    pub fn new(title: impl Into<String>, body: impl Into<String>, target: Target) -> Note {
        Note {
            title: title.into(),
            body: body.into(),
            target,
            group: None,
            key: None,
            button: None,
            unseen: None,
        }
    }
}

/// Counts for the smoke tests' dumps.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Stats {
    pub permission: Permission,
    /// Handed to the notification center (or queued while asking, or
    /// listed in `sent` under the test control).
    pub posted: u64,
    /// Fell back to bouncing the Dock icon.
    pub bounced: u64,
    /// Under the test control: every notification, oldest first.
    pub sent: Vec<Note>,
}

/// Brings view `view`'s window and pane `pane` to the front.
type ShowPane = fn(usize, usize);

struct State {
    permission: Permission,
    /// Notifications waiting for the answer to the permission request.
    waiting: Vec<Note>,
    posted: u64,
    bounced: u64,
    seq: u64,
    sent: Vec<Note>,
    delegate: Option<Retained<Delegate>>,
    /// Each window's progress, by view id.
    progress: BTreeMap<usize, Progress>,
    /// Notifications for each window not looked at since, by view id.
    unseen: BTreeMap<usize, u32>,
    /// The badge last set.
    badge: Option<String>,
    /// What a click on each notification shown does, by identifier.
    targets: HashMap<String, Target>,
    /// Identifiers by [`Note::key`], so a later note replaces it.
    keyed: HashMap<String, String>,
    /// Every [`Note::key`] shown: a note with one of these replaces one
    /// already counted.
    keys: std::collections::HashSet<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        permission: Permission::Unknown,
        waiting: Vec::new(),
        posted: 0,
        bounced: 0,
        seq: 0,
        sent: Vec::new(),
        delegate: None,
        progress: BTreeMap::new(),
        unseen: BTreeMap::new(),
        badge: None,
        targets: HashMap::new(),
        keyed: HashMap::new(),
        keys: std::collections::HashSet::new(),
    });
    /// Brings a view's window and a pane to the front: set by the view
    /// module.
    static FOCUS: RefCell<Option<ShowPane>> = const { RefCell::new(None) };
    /// What the notification buttons do: set by the app module.
    static PRESSED: RefCell<Option<fn(Button)>> = const { RefCell::new(None) };
}

/// Whether this process is an app bundle, so the notification center can
/// be used. A bare binary has no bundle identifier.
pub fn bundled() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
}

/// Whether notifications go to the dump instead of the notification
/// center: the test control is on, without `DECLINE_TEST_REAL_NOTIFY=1`.
fn to_dump() -> bool {
    std::env::var_os(crate::control::ENV).is_some_and(|v| !v.is_empty())
        && std::env::var_os("DECLINE_TEST_REAL_NOTIFY").is_none_or(|v| v != "1")
}

/// How a click on a notification for pane `pane` of view `view` brings
/// it to the front.
pub fn on_click(focus: ShowPane) {
    FOCUS.with(|f| *f.borrow_mut() = Some(focus));
}

/// What a notification's button does.
pub fn on_button(pressed: fn(Button)) {
    PRESSED.with(|f| *f.borrow_mut() = Some(pressed));
}

pub fn stats() -> Stats {
    STATE.with(|s| {
        let s = s.borrow();
        Stats {
            permission: s.permission,
            posted: s.posted,
            bounced: s.bounced,
            sent: s.sent.clone(),
        }
    })
}

/// Show `note`, asking for permission the first time. Without a bundle
/// or permission, bounces the Dock icon instead.
pub fn show(mtm: MainThreadMarker, note: Note) {
    let permission = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.permission == Permission::Unknown {
            if to_dump() {
                s.permission = Permission::Test;
            } else if !bundled() {
                s.permission = Permission::Unbundled;
            }
        }
        s.permission
    });
    let replacing = note
        .key
        .as_ref()
        .is_some_and(|k| STATE.with(|s| !s.borrow_mut().keys.insert(k.clone())));
    if let Some(view) = note.unseen
        && !replacing
    {
        STATE.with(|s| *s.borrow_mut().unseen.entry(view).or_default() += 1);
        update_badge(mtm);
    }
    match permission {
        Permission::Test => STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.posted += 1;
            s.sent.push(note);
        }),
        Permission::Unbundled | Permission::Denied => bounce(mtm),
        Permission::Granted => add_request(note),
        Permission::Asking => STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.posted += 1;
            s.waiting.push(note);
        }),
        Permission::Unknown => {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.permission = Permission::Asking;
                s.posted += 1;
                s.waiting.push(note);
            });
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let delegate = Delegate::new();
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            STATE.with(|s| s.borrow_mut().delegate = Some(delegate));
            register_buttons(&center);
            let reply = RcBlock::new(|granted: Bool, _error: *mut NSError| {
                let granted = granted.as_bool();
                DispatchQueue::main().exec_async(move || answered(granted));
            });
            center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &reply);
        }
    }
}

/// The categories that give notifications their buttons.
fn register_buttons(center: &UNUserNotificationCenter) {
    let categories: Vec<Retained<UNNotificationCategory>> = Button::ALL
        .iter()
        .map(|b| {
            let action = UNNotificationAction::actionWithIdentifier_title_options(
                &NSString::from_str(b.id()),
                &NSString::from_str(b.label()),
                UNNotificationActionOptions::empty(),
            );
            UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
                &NSString::from_str(&b.category()),
                &NSArray::from_retained_slice(&[action]),
                &NSArray::new(),
                UNNotificationCategoryOptions::empty(),
            )
        })
        .collect();
    center.setNotificationCategories(&NSSet::from_retained_slice(&categories));
}

/// The answer to the permission request, on the main thread.
fn answered(granted: bool) {
    let waiting = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.permission = if granted { Permission::Granted } else { Permission::Denied };
        // Counted again below.
        s.posted -= s.waiting.len() as u64;
        std::mem::take(&mut s.waiting)
    });
    let mtm = MainThreadMarker::new().expect("on the main thread");
    for note in waiting {
        if granted {
            add_request(note);
        } else {
            bounce(mtm);
        }
    }
}

fn add_request(note: Note) {
    let ident = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.posted += 1;
        s.seq += 1;
        let fresh = identifier(s.seq);
        let ident = match &note.key {
            Some(k) => s.keyed.entry(k.clone()).or_insert(fresh).clone(),
            None => fresh,
        };
        s.targets.insert(ident.clone(), note.target.clone());
        ident
    });
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&note.title));
    content.setBody(&NSString::from_str(&note.body));
    if let Some(g) = &note.group {
        content.setThreadIdentifier(&NSString::from_str(g));
    }
    if let Some(b) = note.button {
        content.setCategoryIdentifier(&NSString::from_str(&b.category()));
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&ident), &content, None);
    let done = RcBlock::new(|error: *mut NSError| {
        if !error.is_null() {
            // Turned off for this app since, say: ring the bell instead.
            DispatchQueue::main().exec_async(|| bounce(MainThreadMarker::new().expect("on the main thread")));
        }
    });
    UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&done));
}

/// What a bell does while the app isn't in front.
fn bounce(mtm: MainThreadMarker) {
    STATE.with(|s| s.borrow_mut().bounced += 1);
    let app = NSApplication::sharedApplication(mtm);
    if !app.isActive() {
        app.requestUserAttention(objc2_app_kit::NSRequestUserAttentionType::InformationalRequest);
    }
}

/// A notification's identifier: unique in this process.
fn identifier(seq: u64) -> String {
    format!("decline.{}.{seq}", std::process::id())
}

/// Window (view id) `id` was looked at: its notifications come off the
/// Dock icon's count.
pub fn seen(mtm: MainThreadMarker, id: usize) {
    if STATE.with(|s| s.borrow_mut().unseen.remove(&id)).is_some() {
        update_badge(mtm);
    }
}

/// Record view `id`'s progress and update the Dock icon.
pub fn set_progress(mtm: MainThreadMarker, id: usize, progress: Progress) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        match progress {
            Progress::Clear => s.progress.remove(&id),
            p => s.progress.insert(id, p),
        };
    });
    update_badge(mtm);
}

/// Window `id` closed: its progress and count go.
pub fn closed(mtm: MainThreadMarker, id: usize) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.progress.remove(&id);
        s.unseen.remove(&id);
    });
    update_badge(mtm);
}

fn update_badge(mtm: MainThreadMarker) {
    let label = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let label = dock_badge(s.progress.values().copied(), s.unseen.values().sum());
        (label != s.badge).then(|| {
            s.badge = label.clone();
            label
        })
    });
    if let Some(label) = label {
        let tile = NSApplication::sharedApplication(mtm).dockTile();
        tile.setBadgeLabel(label.map(|l| NSString::from_str(&l)).as_deref());
    }
}

/// The Dock badge: progress if any window reports it, else the number of
/// notifications not looked at, if any.
pub fn dock_badge(progress: impl Iterator<Item = Progress>, unseen: u32) -> Option<String> {
    badge(progress).or_else(|| (unseen > 0).then(|| unseen.to_string()))
}

/// The Dock badge for these windows' progress: "!" if any reports an
/// error, else the lowest percentage, else "…" if any is busy.
pub fn badge(all: impl Iterator<Item = Progress>) -> Option<String> {
    let mut lowest: Option<u8> = None;
    let mut busy = false;
    for p in all {
        match p {
            Progress::Error(_) => return Some("!".into()),
            Progress::Normal(n) | Progress::Paused(n) => lowest = Some(lowest.map_or(n, |l| l.min(n))),
            Progress::Indeterminate => busy = true,
            Progress::Clear => {}
        }
    }
    match lowest {
        Some(n) => Some(format!("{n}%")),
        None => busy.then(|| "…".into()),
    }
}

/// A click on the notification `ident`, or on its `button`.
fn clicked(ident: &str, button: Option<Button>) {
    if let Some(b) = button {
        if let Some(f) = PRESSED.with(|f| *f.borrow()) {
            f(b);
        }
        return;
    }
    let target = STATE.with(|s| s.borrow().targets.get(ident).cloned());
    match target {
        Some(Target::Pane { view, pane }) => {
            if let Some(focus) = FOCUS.with(|f| *f.borrow()) {
                focus(view, pane);
            }
        }
        Some(Target::Open(path)) => {
            let url = if path.contains("://") {
                objc2_foundation::NSURL::URLWithString(&NSString::from_str(&path))
            } else {
                Some(objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(&path)))
            };
            if let Some(url) = url {
                objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&url);
            }
        }
        Some(Target::App) | None => {}
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "DeclineNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Shown even while the app is in front: it was posted for a window,
        /// tab or pane that isn't.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
        }

        /// A click: bring its window, tab and pane to the front, or press
        /// its button.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _center: &UNUserNotificationCenter, response: &UNNotificationResponse, handler: &block2::DynBlock<dyn Fn()>) {
            let id = response.notification().request().identifier().to_string();
            let action = response.actionIdentifier().to_string();
            let button = Button::ALL.into_iter().find(|b| b.id() == action);
            DispatchQueue::main().exec_async(move || clicked(&id, button));
            handler.call(());
        }
    }
);

impl Delegate {
    fn new() -> Retained<Delegate> {
        unsafe { msg_send![Delegate::alloc(), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_binary_is_not_bundled() {
        // Test binaries are plain executables, like `cargo install`'s.
        assert!(!bundled());
    }

    #[test]
    fn the_count_shows_without_progress() {
        use Progress::*;
        assert_eq!(dock_badge([].into_iter(), 0), None);
        assert_eq!(dock_badge([].into_iter(), 3).as_deref(), Some("3"));
        assert_eq!(dock_badge([Normal(42)].into_iter(), 3).as_deref(), Some("42%"));
        assert_eq!(dock_badge([Clear].into_iter(), 1).as_deref(), Some("1"));
    }

    #[test]
    fn badge_for_progress() {
        use Progress::*;
        for (all, want) in [
            (vec![], None),
            (vec![Normal(42)], Some("42%")),
            (vec![Paused(90), Normal(42), Normal(77)], Some("42%")),
            (vec![Indeterminate], Some("…")),
            (vec![Indeterminate, Normal(5)], Some("5%")),
            (vec![Normal(5), Error(50)], Some("!")),
            (vec![Clear], None),
        ] {
            assert_eq!(badge(all.iter().copied()).as_deref(), want, "{all:?}");
        }
    }
}
