//! The little of CoreFoundation the stub uses, by hand: no bindings crate,
//! so the stub links nothing but CoreFoundation and Security strongly
//! (docs/design/pins.md §3.5).

#![allow(non_upper_case_globals)]

use std::ffi::{c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub type CFTypeRef = *const c_void;
pub type CFIndex = isize;
pub type CFTypeID = usize;
pub type CFOptionFlags = usize;

const kCFStringEncodingUTF8: u32 = 0x0800_0100;
const kCFNumberSInt64Type: CFIndex = 4;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> CFTypeID;
    fn CFStringGetTypeID() -> CFTypeID;
    fn CFNumberGetTypeID() -> CFTypeID;
    fn CFStringCreateWithBytes(alloc: CFTypeRef, bytes: *const u8, len: CFIndex, encoding: u32, external: u8) -> CFTypeRef;
    fn CFStringGetLength(s: CFTypeRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(len: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(s: CFTypeRef, buf: *mut c_char, size: CFIndex, encoding: u32) -> u8;
    fn CFNumberGetValue(n: CFTypeRef, kind: CFIndex, out: *mut c_void) -> u8;
    fn CFBundleGetMainBundle() -> CFTypeRef;
    fn CFBundleGetValueForInfoDictionaryKey(bundle: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFBundleCopyBundleURL(bundle: CFTypeRef) -> CFTypeRef;
    fn CFBundleCopyInfoDictionaryForURL(url: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFURLCreateFromFileSystemRepresentation(alloc: CFTypeRef, buf: *const u8, len: CFIndex, dir: u8) -> CFTypeRef;
    fn CFURLGetFileSystemRepresentation(url: CFTypeRef, resolve: u8, buf: *mut u8, len: CFIndex) -> u8;
    fn CFArrayGetCount(a: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(a: CFTypeRef, i: CFIndex) -> CFTypeRef;
    fn CFErrorCopyDescription(e: CFTypeRef) -> CFTypeRef;
    fn CFUserNotificationDisplayAlert(
        timeout: f64,
        flags: CFOptionFlags,
        icon: CFTypeRef,
        sound: CFTypeRef,
        localization: CFTypeRef,
        header: CFTypeRef,
        message: CFTypeRef,
        default_button: CFTypeRef,
        alternate_button: CFTypeRef,
        other_button: CFTypeRef,
        response: *mut CFOptionFlags,
    ) -> i32;
}

/// An owned CoreFoundation object, released when dropped.
pub struct Owned(CFTypeRef);

impl Owned {
    /// Takes ownership of `cf` (from a Create or Copy function); `None`
    /// if it is null.
    pub fn new(cf: CFTypeRef) -> Option<Owned> {
        (!cf.is_null()).then(|| Owned(cf))
    }

    pub fn get(&self) -> CFTypeRef {
        self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: we own one reference to a non-null object.
        unsafe { CFRelease(self.0) }
    }
}

/// A `CFString` of `s`.
pub fn string(s: &str) -> Owned {
    // SAFETY: valid bytes and length; UTF-8 from a &str never fails.
    Owned::new(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as CFIndex, kCFStringEncodingUTF8, 0) })
        .expect("CFStringCreateWithBytes")
}

/// `cf` as a Rust string, if it is a `CFString`.
pub(crate) fn to_string(cf: CFTypeRef) -> Option<String> {
    // SAFETY: `cf` is a live CF object or null; the buffer is as large as
    // CoreFoundation says the string can need, plus the NUL.
    unsafe {
        if cf.is_null() || CFGetTypeID(cf) != CFStringGetTypeID() {
            return None;
        }
        let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(cf), kCFStringEncodingUTF8) + 1;
        let mut buf = vec![0u8; size as usize];
        if CFStringGetCString(cf, buf.as_mut_ptr().cast(), size, kCFStringEncodingUTF8) == 0 {
            return None;
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        buf.truncate(end);
        String::from_utf8(buf).ok()
    }
}

/// `cf` as an integer, if it is a `CFNumber`.
pub(crate) fn to_i64(cf: CFTypeRef) -> Option<i64> {
    // SAFETY: as above; the out pointer is an i64 as kCFNumberSInt64Type asks.
    unsafe {
        if cf.is_null() || CFGetTypeID(cf) != CFNumberGetTypeID() {
            return None;
        }
        let mut v = 0i64;
        (CFNumberGetValue(cf, kCFNumberSInt64Type, (&raw mut v).cast()) != 0).then_some(v)
    }
}

/// A file URL of `path`.
pub fn url(path: &Path, dir: bool) -> Option<Owned> {
    let b = path.as_os_str().as_bytes();
    // SAFETY: valid bytes and length.
    Owned::new(unsafe { CFURLCreateFromFileSystemRepresentation(std::ptr::null(), b.as_ptr(), b.len() as CFIndex, dir as u8) })
}

/// The path of a file URL.
pub(crate) fn url_path(url: CFTypeRef) -> Option<PathBuf> {
    let mut buf = vec![0u8; libc::PATH_MAX as usize + 1];
    // SAFETY: `url` is a live CFURL; the buffer's length is passed.
    if unsafe { CFURLGetFileSystemRepresentation(url, 1, buf.as_mut_ptr(), buf.len() as CFIndex) } == 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0)?;
    buf.truncate(end);
    Some(PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(
        buf,
    )))
}

/// The running program's bundle, as Info.plist values and its path.
pub struct MainBundle(CFTypeRef);

impl MainBundle {
    pub fn get() -> Option<MainBundle> {
        // SAFETY: no arguments; the result is not owned (Get rule).
        let b = unsafe { CFBundleGetMainBundle() };
        (!b.is_null()).then_some(MainBundle(b))
    }

    /// The value of `key` in the bundle's Info.plist, not owned: it lives
    /// as long as the main bundle, which is the whole process.
    pub fn value(&self, key: &str) -> CFTypeRef {
        let k = string(key);
        // SAFETY: a live bundle and key; Get rule, nothing to release.
        unsafe { CFBundleGetValueForInfoDictionaryKey(self.0, k.get()) }
    }

    pub fn string(&self, key: &str) -> Option<String> {
        to_string(self.value(key))
    }

    pub fn path(&self) -> Option<PathBuf> {
        // SAFETY: a live bundle; Copy rule, released by Owned.
        let u = Owned::new(unsafe { CFBundleCopyBundleURL(self.0) })?;
        url_path(u.get())
    }
}

/// A bundle's Info.plist, read without registering a `CFBundle` for it.
pub struct InfoPlist(Owned);

impl InfoPlist {
    pub fn read(bundle: &Path) -> Option<InfoPlist> {
        let u = url(bundle, true)?;
        // SAFETY: a live URL; Copy rule.
        Owned::new(unsafe { CFBundleCopyInfoDictionaryForURL(u.get()) }).map(InfoPlist)
    }

    pub fn string(&self, key: &str) -> Option<String> {
        let k = string(key);
        // SAFETY: a live dictionary and key; Get rule.
        to_string(unsafe { CFDictionaryGetValue(self.0.get(), k.get()) })
    }

    pub fn integer(&self, key: &str) -> Option<i64> {
        let k = string(key);
        // SAFETY: a live dictionary and key; Get rule.
        to_i64(unsafe { CFDictionaryGetValue(self.0.get(), k.get()) })
    }
}

/// The value for `key` in a live `CFDictionary`, not owned (Get rule).
pub(crate) fn dict_value(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef {
    // SAFETY: the caller passes a live dictionary and key.
    unsafe { CFDictionaryGetValue(dict, key) }
}

/// The elements of a `CFArray`, not owned.
pub(crate) fn array(a: CFTypeRef) -> Vec<CFTypeRef> {
    // SAFETY: `a` is a live CFArray; indices are in range.
    unsafe { (0..CFArrayGetCount(a)).map(|i| CFArrayGetValueAtIndex(a, i)).collect() }
}

/// A `CFError`'s description.
pub(crate) fn error_description(e: CFTypeRef) -> Option<String> {
    // SAFETY: a live CFError; Copy rule.
    let d = Owned::new(unsafe { CFErrorCopyDescription(e) })?;
    to_string(d.get())
}

/// Which button closed an alert.
#[derive(Debug, PartialEq, Eq)]
pub enum Button {
    Default,
    Alternate,
    /// Timed out or failed.
    None,
}

/// A modal alert with two buttons, through the user notification centre:
/// no AppKit needed. Blocks until a button is pressed.
pub fn alert(header: &str, message: &str, default_button: &str, alternate_button: &str) -> Button {
    let (h, m, d, a) = (string(header), string(message), string(default_button), string(alternate_button));
    let mut response: CFOptionFlags = 0;
    // SAFETY: live strings, null optional arguments, a valid out pointer.
    let r = unsafe {
        CFUserNotificationDisplayAlert(
            0.0,
            0, // kCFUserNotificationStopAlertLevel
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            h.get(),
            m.get(),
            d.get(),
            a.get(),
            std::ptr::null(),
            &mut response,
        )
    };
    match (r, response & 3) {
        (0, 0) => Button::Default,
        (0, 1) => Button::Alternate,
        _ => Button::None,
    }
}
