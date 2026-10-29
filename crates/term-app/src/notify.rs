//! Desktop notifications a program asks for (OSC 9 and OSC 777), and its
//! progress (OSC 9;4) on the Dock icon.
//!
//! Notifications go through `UNUserNotificationCenter`, which throws when
//! the process isn't an app bundle (a bare `cargo install` binary), so
//! [`bundled`] is checked first; without a bundle, or when notifications
//! are turned off for the app, the caller falls back to the bell's
//! behaviour (bouncing the Dock icon). Permission is asked once, on the
//! first notification. Clicking one brings its window or tab to the front.
//!
//! Progress shows as the Dock icon's badge: the lowest percentage of any
//! window reporting one, "!" if one reports an error, "…" if they only
//! report that they are busy.

use std::cell::RefCell;
use std::collections::BTreeMap;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker, define_class, msg_send};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions, UNNotificationRequest,
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
}

/// Counts for the smoke tests' dumps.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Stats {
    pub permission: Permission,
    /// Handed to the notification center (or queued while asking).
    pub posted: u64,
    /// Fell back to bouncing the Dock icon.
    pub bounced: u64,
}

struct State {
    permission: Permission,
    /// Notifications waiting for the answer to the permission request.
    waiting: Vec<(usize, String, String)>,
    posted: u64,
    bounced: u64,
    seq: u64,
    delegate: Option<Retained<Delegate>>,
    /// Each window's progress, by view id.
    progress: BTreeMap<usize, Progress>,
}

thread_local! {
    static STATE: RefCell<State> = const {
        RefCell::new(State {
            permission: Permission::Unknown,
            waiting: Vec::new(),
            posted: 0,
            bounced: 0,
            seq: 0,
            delegate: None,
            progress: BTreeMap::new(),
        })
    };
    /// Brings a view's window to the front: set by the view module.
    static FOCUS: RefCell<Option<fn(usize)>> = const { RefCell::new(None) };
}

/// Whether this process is an app bundle, so the notification center can
/// be used. A bare binary has no bundle identifier.
pub fn bundled() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
}

/// How a click on a notification for view `id` brings it to the front.
pub fn on_click(focus: fn(usize)) {
    FOCUS.with(|f| *f.borrow_mut() = Some(focus));
}

pub fn stats() -> Stats {
    STATE.with(|s| {
        let s = s.borrow();
        Stats {
            permission: s.permission,
            posted: s.posted,
            bounced: s.bounced,
        }
    })
}

/// Show a notification for view `id`, asking for permission the first
/// time. Without a bundle or permission, bounces the Dock icon instead.
pub fn post(mtm: MainThreadMarker, id: usize, title: &str, body: &str) {
    let permission = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.permission == Permission::Unknown && !bundled() {
            s.permission = Permission::Unbundled;
        }
        s.permission
    });
    match permission {
        Permission::Unbundled | Permission::Denied => bounce(mtm),
        Permission::Granted => add_request(id, title, body),
        Permission::Asking => STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.posted += 1;
            s.waiting.push((id, title.into(), body.into()));
        }),
        Permission::Unknown => {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.permission = Permission::Asking;
                s.posted += 1;
                s.waiting.push((id, title.into(), body.into()));
            });
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let delegate = Delegate::new();
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            STATE.with(|s| s.borrow_mut().delegate = Some(delegate));
            let reply = RcBlock::new(|granted: Bool, _error: *mut NSError| {
                let granted = granted.as_bool();
                DispatchQueue::main().exec_async(move || answered(granted));
            });
            center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &reply);
        }
    }
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
    for (id, title, body) in waiting {
        if granted {
            add_request(id, &title, &body);
        } else {
            bounce(mtm);
        }
    }
}

fn add_request(id: usize, title: &str, body: &str) {
    let seq = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.posted += 1;
        s.seq += 1;
        s.seq
    });
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&identifier(id, seq)), &content, None);
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

/// A notification's identifier names its view, so a click can find it.
fn identifier(view: usize, seq: u64) -> String {
    format!("decline.{view}.{seq}")
}

fn view_of(identifier: &str) -> Option<usize> {
    let mut parts = identifier.strip_prefix("decline.")?.split('.');
    let view = parts.next()?.parse().ok()?;
    parts.next()?.parse::<u64>().ok()?;
    parts.next().is_none().then_some(view)
}

/// Record view `id`'s progress and update the Dock icon.
pub fn set_progress(mtm: MainThreadMarker, id: usize, progress: Progress) {
    let label = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let before = badge(s.progress.values().copied());
        match progress {
            Progress::Clear => s.progress.remove(&id),
            p => s.progress.insert(id, p),
        };
        let after = badge(s.progress.values().copied());
        (after != before).then_some(after)
    });
    if let Some(label) = label {
        let tile = NSApplication::sharedApplication(mtm).dockTile();
        tile.setBadgeLabel(label.map(|l| NSString::from_str(&l)).as_deref());
    }
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

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "DeclineNotificationDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        /// Shown even while the app is in front: it was posted for a window
        /// or tab that isn't.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            handler.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
        }

        /// A click: bring its window or tab to the front.
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _center: &UNUserNotificationCenter, response: &UNNotificationResponse, handler: &block2::DynBlock<dyn Fn()>) {
            let id = response.notification().request().identifier().to_string();
            if let Some(view) = view_of(&id) {
                DispatchQueue::main().exec_async(move || {
                    if let Some(focus) = FOCUS.with(|f| *f.borrow()) {
                        focus(view);
                    }
                });
            }
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
    fn identifiers_name_their_view() {
        assert_eq!(view_of(&identifier(7, 3)), Some(7));
        for bad in ["", "decline.", "decline.x.1", "decline.7", "decline.7.1.2", "other.7.1"] {
            assert_eq!(view_of(bad), None, "{bad:?}");
        }
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
