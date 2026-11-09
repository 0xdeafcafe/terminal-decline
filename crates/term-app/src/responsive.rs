//! While the app is active (one of its windows takes the keys), tell macOS
//! that it is latency-critical and doing what the user asked, so its main
//! thread's wake-ups (a key from the window server, the reader's echo) are
//! neither coalesced nor delayed by App Nap. Idle system sleep stays
//! allowed, and the activity ends when the app is no longer active.

use std::cell::RefCell;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::NSString;

/// `NSActivityUserInitiatedAllowingIdleSystemSleep | NSActivityLatencyCritical`.
const OPTIONS: u64 = 0x00EF_FFFF | 0xFF_0000_0000;

thread_local! {
    static ACTIVITY: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

/// The app became active (`true`) or stopped being (`false`). Main thread.
pub fn set_active(active: bool) {
    ACTIVITY.with(|a| {
        let mut a = a.borrow_mut();
        // SAFETY: NSProcessInfo's documented class methods, on the main
        // thread; the token is kept until it is handed back to `endActivity:`.
        unsafe {
            let info: Retained<AnyObject> = msg_send![objc2::class!(NSProcessInfo), processInfo];
            match (active, a.take()) {
                (true, None) => {
                    let reason = NSString::from_str("typing in a terminal");
                    let token: Option<Retained<AnyObject>> = msg_send![&*info, beginActivityWithOptions: OPTIONS, reason: &*reason];
                    *a = token;
                }
                (true, Some(t)) => *a = Some(t),
                (false, Some(t)) => {
                    let _: () = msg_send![&*info, endActivity: &*t];
                }
                (false, None) => {}
            }
        }
    });
}
