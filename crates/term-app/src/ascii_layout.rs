//! What each key types on the ASCII-capable keyboard layout, so Ctrl and
//! Meta combinations work on non-Latin layouts: Ctrl+С on a Russian layout
//! sends ^C, as it would on the Latin layout the user also has.
//!
//! macOS keeps an "ASCII-capable" layout alongside every non-Latin one
//! (the Latin layout used for Cmd shortcuts). Its characters are read
//! once with `UCKeyTranslate` and kept until the selected input source
//! changes, which macOS announces with a distributed notification.
//!
//! Text Input Sources must be used on the main thread; key events arrive
//! there.

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

/// Virtual key codes looked up (every key on the main keyboard is below).
const KEYS: usize = 128;

/// Unshifted and shifted characters of each key; `None` where the key
/// types nothing printable.
type Table = [(Option<char>, Option<char>); KEYS];

/// The table needs reading again: the input source changed, or it was
/// never read.
static STALE: AtomicBool = AtomicBool::new(true);
static OBSERVE: Once = Once::new();

thread_local! {
    static TABLE: RefCell<Option<Box<Table>>> = const { RefCell::new(None) };
}

/// The characters `keycode` types on the ASCII-capable layout, without and
/// with Shift. Read again only after a layout change.
pub fn chars(keycode: u16) -> (Option<char>, Option<char>) {
    OBSERVE.call_once(observe_changes);
    TABLE.with(|t| {
        let mut t = t.borrow_mut();
        if STALE.swap(false, Ordering::AcqRel) || t.is_none() {
            *t = read_table();
        }
        t.as_ref().and_then(|t| t.get(keycode as usize).copied()).unwrap_or((None, None))
    })
}

/// The first character of UTF-16 `units`, if it is printable.
fn printable(units: &[u16]) -> Option<char> {
    let c = char::decode_utf16(units.iter().copied()).next()?.ok()?;
    (!c.is_control()).then_some(c)
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> *mut c_void;
    fn TISGetInputSourceProperty(source: *mut c_void, key: *const c_void) -> *mut c_void;
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
    static kTISNotifySelectedKeyboardInputSourceChanged: *const c_void;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        keycode: u16,
        action: u16,
        modifiers: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_len: usize,
        len: *mut usize,
        out: *mut u16,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFNotificationCenterGetDistributedCenter() -> *mut c_void;
    fn CFNotificationCenterAddObserver(
        center: *mut c_void,
        observer: *const c_void,
        callback: extern "C" fn(*mut c_void, *mut c_void, *const c_void, *const c_void, *const c_void),
        name: *const c_void,
        object: *const c_void,
        suspension: isize,
    );
}

/// `kUCKeyActionDown`.
const KEY_DOWN: u16 = 0;
/// `kUCKeyTranslateNoDeadKeysMask`: a dead key gives its accent.
const NO_DEAD_KEYS: u32 = 1;
/// `shiftKey >> 8`, as `UCKeyTranslate` takes modifiers.
const SHIFT: u32 = 0x02;
/// `CFNotificationSuspensionBehaviorDeliverImmediately`.
const DELIVER_IMMEDIATELY: isize = 4;

/// Delivered even while decline is in the background, so the table is
/// never read for a layout that is no longer selected.
extern "C" fn layout_changed(_: *mut c_void, _: *mut c_void, _: *const c_void, _: *const c_void, _: *const c_void) {
    STALE.store(true, Ordering::Release);
}

fn observe_changes() {
    unsafe {
        CFNotificationCenterAddObserver(
            CFNotificationCenterGetDistributedCenter(),
            std::ptr::null(),
            layout_changed,
            kTISNotifySelectedKeyboardInputSourceChanged,
            std::ptr::null(),
            DELIVER_IMMEDIATELY,
        );
    }
}

fn read_table() -> Option<Box<Table>> {
    unsafe {
        let source = TISCopyCurrentASCIICapableKeyboardLayoutInputSource();
        if source.is_null() {
            return None;
        }
        let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        let table = (!data.is_null()).then(|| {
            let layout = CFDataGetBytePtr(data) as *const c_void;
            let kind = LMGetKbdType() as u32;
            let translate = |code: u16, mods: u32| {
                let mut dead = 0u32;
                let mut out = [0u16; 4];
                let mut len = 0usize;
                let status = UCKeyTranslate(
                    layout,
                    code,
                    KEY_DOWN,
                    mods,
                    kind,
                    NO_DEAD_KEYS,
                    &mut dead,
                    out.len(),
                    &mut len,
                    out.as_mut_ptr(),
                );
                if status == 0 { printable(&out[..len.min(out.len())]) } else { None }
            };
            let mut table: Box<Table> = Box::new([(None, None); KEYS]);
            for (code, slot) in table.iter_mut().enumerate() {
                *slot = (translate(code as u16, 0), translate(code as u16, SHIFT));
            }
            table
        });
        // The layout data belongs to the source, so it is read before this.
        CFRelease(source);
        table
    }
}

#[cfg(test)]
mod tests {
    use super::printable;

    #[test]
    fn only_printable_characters_count() {
        assert_eq!(printable(&[0x63]), Some('c'));
        assert_eq!(printable(&[0x43, 0x44]), Some('C'));
        assert_eq!(printable(&[0x0441]), Some('с'));
        assert_eq!(printable(&[0xD83D, 0xDE00]), Some('😀'));
        for units in [&[][..], &[0x0d], &[0x1b], &[0x7f], &[0xDC00]] {
            assert_eq!(printable(units), None, "{units:?}");
        }
    }
}
