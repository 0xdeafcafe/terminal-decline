//! Checking the library's code signature before loading it
//! (docs/design/pins.md §3.4).

#![allow(non_upper_case_globals)]

use std::ffi::c_void;
use std::path::Path;

use crate::cf::{self, CFTypeRef, Owned};

const kSecCSStrictValidate: u32 = 1 << 4;
const kSecCSNoNetworkAccess: u32 = 1 << 29;

/// The flags the stub checks with: strict, and never the network (a
/// revocation check online costs ~15 ms more and may hang offline).
pub const CHECK_FLAGS: u32 = kSecCSStrictValidate | kSecCSNoNetworkAccess;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecStaticCodeCreateWithPath(path: CFTypeRef, flags: u32, code: *mut CFTypeRef) -> i32;
    fn SecRequirementCreateWithString(text: CFTypeRef, flags: u32, req: *mut CFTypeRef) -> i32;
    fn SecStaticCodeCheckValidityWithErrors(code: CFTypeRef, flags: u32, req: CFTypeRef, errors: *mut CFTypeRef) -> i32;
    fn SecCodeCopySigningInformation(code: CFTypeRef, flags: u32, info: *mut CFTypeRef) -> i32;
    static kSecCodeInfoTeamIdentifier: CFTypeRef;
}

const kSecCSSigningInformation: u32 = 1 << 1;

/// The team ID the file at `path` is signed with: `None` for ad-hoc
/// signing (or none). The pin module writes a pin's requirement from it
/// (pins.md §3.4); the stub itself never calls it.
pub fn team_id(path: &Path) -> Result<Option<String>, String> {
    let url = cf::url(path, false).ok_or("not a usable path")?;
    let mut code: CFTypeRef = std::ptr::null();
    // SAFETY: a live URL and a valid out pointer.
    let status = unsafe { SecStaticCodeCreateWithPath(url.get(), 0, &mut code) };
    let code = Owned::new(code)
        .filter(|_| status == 0)
        .ok_or_else(|| format!("can't read its signature (OSStatus {status})"))?;
    let mut info: CFTypeRef = std::ptr::null();
    // SAFETY: live code and a valid out pointer; Copy rule.
    let status = unsafe { SecCodeCopySigningInformation(code.get(), kSecCSSigningInformation, &mut info) };
    let info = Owned::new(info)
        .filter(|_| status == 0)
        .ok_or_else(|| format!("can't read its signing information (OSStatus {status})"))?;
    // SAFETY: the key is an immutable CFString constant of Security.
    let key = unsafe { kSecCodeInfoTeamIdentifier };
    Ok(cf::to_string(cf::dict_value(info.get(), key)).filter(|t| !t.is_empty()))
}

/// Checks that the file at `path` is validly signed and meets
/// `requirement` (code requirement language). The error says why not.
pub fn check(path: &Path, requirement: &str) -> Result<(), String> {
    let url = cf::url(path, false).ok_or("not a usable path")?;
    let mut code: CFTypeRef = std::ptr::null();
    // SAFETY: a live URL and a valid out pointer.
    let status = unsafe { SecStaticCodeCreateWithPath(url.get(), 0, &mut code) };
    let code = Owned::new(code)
        .filter(|_| status == 0)
        .ok_or_else(|| format!("can't read its signature (OSStatus {status})"))?;
    let text = cf::string(requirement);
    let mut req: CFTypeRef = std::ptr::null();
    // SAFETY: a live string and a valid out pointer.
    let status = unsafe { SecRequirementCreateWithString(text.get(), 0, &mut req) };
    let req = Owned::new(req)
        .filter(|_| status == 0)
        .ok_or_else(|| format!("the pin's requirement doesn't parse (OSStatus {status})"))?;
    let mut err: *const c_void = std::ptr::null();
    // SAFETY: live code and requirement, a valid out pointer; the error
    // (Copy rule) is released by Owned.
    let status = unsafe { SecStaticCodeCheckValidityWithErrors(code.get(), CHECK_FLAGS, req.get(), &mut err) };
    let err = Owned::new(err);
    if status == 0 {
        return Ok(());
    }
    let why = err.and_then(|e| cf::error_description(e.get()));
    Err(match (status, why) {
        // errSecCSReqFailed
        (-67050, _) => format!("it isn't signed as the pin requires ({requirement})"),
        // errSecCSUnsigned
        (-67062, _) => "it isn't signed".into(),
        (_, Some(why)) => format!("{why} (OSStatus {status})"),
        (_, None) => format!("OSStatus {status}"),
    })
}
