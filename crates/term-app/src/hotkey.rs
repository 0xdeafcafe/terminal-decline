//! A system-wide hotkey through Carbon's `RegisterEventHotKey`, for the
//! quick terminal ([`crate::quick`]). Unlike a global event monitor it
//! needs no Accessibility permission, and it costs nothing while idle:
//! the window server sends the app an event when the keys are pressed,
//! which the main run loop hands to the handler. The key is taken from
//! every app while registered, this one included.

use std::ffi::c_void;

use crate::quick::Hotkey;

type OSStatus = i32;
type EventTargetRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;
type EventRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
type EventHandlerUPP = extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus;

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

const fn four_cc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

const K_EVENT_CLASS_KEYBOARD: u32 = four_cc(b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
/// This app's hotkeys' signature.
const SIGNATURE: u32 = four_cc(b"TDqt");

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: EventHandlerUPP,
        num_types: u32,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OSStatus;
    fn RegisterEventHotKey(
        keycode: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;
    fn UnregisterEventHotKey(hotkey: EventHotKeyRef) -> OSStatus;
}

thread_local! {
    /// What a press calls (main thread), set by [`register`].
    static ON_PRESS: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
    /// The handler is installed once, for the process.
    static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

extern "C" fn pressed(_next: EventHandlerCallRef, _event: EventRef, _user: *mut c_void) -> OSStatus {
    // One hotkey only, so which one isn't asked. Called on the main
    // thread, from the run loop.
    let _ = std::panic::catch_unwind(|| {
        ON_PRESS.with(|f| {
            if let Some(f) = f.borrow().as_ref() {
                f();
            }
        })
    });
    0
}

/// A registered hotkey; dropping it unregisters it.
pub struct Registration(EventHotKeyRef);

impl Drop for Registration {
    fn drop(&mut self) {
        unsafe { UnregisterEventHotKey(self.0) };
    }
}

/// Register `key`, calling `on_press` on the main thread each time it is
/// pressed (main thread only). Fails when another app holds the same
/// keys, or Carbon refuses.
pub fn register(key: Hotkey, on_press: impl Fn() + 'static) -> Result<Registration, String> {
    let target = unsafe { GetApplicationEventTarget() };
    if !INSTALLED.with(|i| i.get()) {
        let spec = EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOT_KEY_PRESSED,
        };
        let mut handler: EventHandlerRef = std::ptr::null_mut();
        let status = unsafe { InstallEventHandler(target, pressed, 1, &spec, std::ptr::null_mut(), &mut handler) };
        if status != 0 {
            return Err(format!("InstallEventHandler failed ({status})"));
        }
        INSTALLED.with(|i| i.set(true));
    }
    let mut out: EventHotKeyRef = std::ptr::null_mut();
    let id = EventHotKeyID {
        signature: SIGNATURE,
        id: 1,
    };
    let status = unsafe { RegisterEventHotKey(key.keycode, key.modifiers, id, target, 0, &mut out) };
    if status != 0 || out.is_null() {
        // -9878: eventHotKeyExistsErr, another app has it.
        return Err(match status {
            -9878 => "another app already uses that hotkey".to_string(),
            s => format!("RegisterEventHotKey failed ({s})"),
        });
    }
    ON_PRESS.with(|f| *f.borrow_mut() = Some(Box::new(on_press)));
    Ok(Registration(out))
}
