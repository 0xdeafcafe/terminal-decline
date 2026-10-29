//! Sparkle in the process (docs/design/updater.md §3): loaded at run time
//! from `Contents/Frameworks/Sparkle.framework`, only when [`super::decide`]
//! says so, so pins and the bare binary never load it. The library does
//! not link Sparkle; its classes are found by name once it is loaded.
//!
//! [`Driver`] is the silent `SPUUserDriver` and [`Delegate`] the
//! `SPUUpdaterDelegate`; both only forward to [`super::answer`] and
//! [`super::decide`]. Everything here runs on the main thread.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_void};
use std::path::{Path, PathBuf};

use block2::{DynBlock, RcBlock};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, NSObjectProtocol, Sel};
use objc2::{ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSApplication, NSWorkspace};
use objc2_foundation::{NSBundle, NSDictionary, NSError, NSObject, NSSet, NSString, NSURL};

use super::{Answer, Call, Choice, Context, Decision, Item, Stage, answer, decide, log, set_status};
use crate::entry::Role;

/// Sparkle's protocols, which our classes declare once it is loaded.
const DRIVER_PROTOCOL: &CStr = c"SPUUserDriver";
const DELEGATE_PROTOCOL: &CStr = c"SPUUpdaterDelegate";

/// The updater, kept for the life of the process (Sparkle holds its
/// delegate weakly, and a framework can't be unloaded anyway).
struct Running {
    updater: Retained<AnyObject>,
    _driver: Retained<Driver>,
    _delegate: Retained<Delegate>,
    context: Context,
}

thread_local! {
    static RUNNING: RefCell<Option<Running>> = const { RefCell::new(None) };
    /// Whether one of our alerts is in its modal loop.
    static ALERT_OPEN: Cell<bool> = const { Cell::new(false) };
}

/// The updater lock, held open for the life of the process.
static LOCK: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();

/// Decide, log why, and start Sparkle if the updater is on. Returns
/// whether it runs (the menu then gains Check for Updates…).
pub fn start(role: &Role, setting: bool) -> bool {
    let context = gather(role, setting);
    let decision = decide(&context);
    match decision {
        Decision::Off(why) => {
            log(&format!("off: {why}"));
            set_status(format!("off: {why}"));
            return false;
        }
        Decision::On => {}
    }
    let Some(mtm) = MainThreadMarker::new() else { return false };
    match start_sparkle(mtm, context) {
        Ok(()) => {
            log("on");
            set_status("on");
            true
        }
        Err(e) => {
            log(&format!("off: {e}"));
            set_status(format!("off: {e}"));
            false
        }
    }
}

/// The `update` setting changed (a config reload): checks Sparkle asks
/// for from now on follow it.
pub fn set_setting(setting: bool) {
    RUNNING.with(|r| {
        if let Some(r) = r.borrow_mut().as_mut()
            && r.context.setting != setting
        {
            r.context.setting = setting;
            log(&format!("update = {setting}"));
        }
    });
}

/// Whether the app menu shows Check for Updates….
pub fn menu_shown() -> bool {
    RUNNING.with(|r| r.borrow().as_ref().is_some_and(|r| r.context.setting))
}

/// Check for Updates…: a check the user asked for, with alerts. With a
/// session already going, its alert comes to the front instead.
pub fn check_now() {
    let Some(updater) = updater() else { return };
    let in_progress: bool = unsafe { msg_send![&*updater, sessionInProgress] };
    if in_progress {
        focus();
        return;
    }
    let can: bool = unsafe { msg_send![&*updater, canCheckForUpdates] };
    if can {
        set_status("checking");
        let _: () = unsafe { msg_send![&*updater, checkForUpdates] };
    }
}

/// A background check now, as the daily one is (the test-only
/// `update-check` control command). Nothing while the updater is off.
pub fn check_in_background() {
    let Some(updater) = updater() else {
        log("update-check: the updater is off");
        return;
    };
    log("update-check: a background check");
    set_status("checking");
    let _: () = unsafe { msg_send![&*updater, checkForUpdatesInBackground] };
}

fn updater() -> Option<Retained<AnyObject>> {
    RUNNING.with(|r| r.borrow().as_ref().map(|r| r.updater.clone()))
}

fn context() -> Option<Context> {
    RUNNING.with(|r| r.borrow().as_ref().map(|r| r.context.clone()))
}

fn setting() -> bool {
    context().is_some_and(|c| c.setting)
}

/// The inputs of [`decide`], read from the bundle and the process. Only
/// the app gathers them (and takes the lock); a pin or the bare binary is
/// off whatever they are.
fn gather(role: &Role, setting: bool) -> Context {
    let test = std::env::var_os(crate::control::ENV).is_some_and(|v| !v.is_empty());
    let test_feed = test && test_feed().is_some();
    let mut c = Context {
        role: role.clone(),
        primary: false,
        bundle_id: None,
        team_id: None,
        dogfood: false,
        version: String::new(),
        setting,
        public_key: false,
        framework: false,
        test,
        test_feed,
    };
    if *role != Role::App {
        return c;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    c.primary = LOCK.get().is_some() || super::take_lock(&super::lock_path(&home)).is_some_and(|f| LOCK.set(f).is_ok());
    let bundle = NSBundle::mainBundle();
    let info = |key: &str| {
        bundle
            .objectForInfoDictionaryKey(&NSString::from_str(key))
            .and_then(|o| o.downcast::<NSString>().ok())
            .map(|s| s.to_string())
    };
    c.bundle_id = bundle.bundleIdentifier().map(|s| s.to_string());
    c.version = info("CFBundleShortVersionString").unwrap_or_default();
    c.public_key = info("SUPublicEDKey").is_some_and(|k| super::is_public_key(&k));
    let path = PathBuf::from(bundle.bundlePath().to_string());
    c.dogfood = crate::reload::is_dogfood_bundle(&path);
    c.framework = framework_path(&path).is_dir();
    c.team_id = own_team_id();
    c
}

fn framework_path(bundle: &Path) -> PathBuf {
    bundle.join("Contents/Frameworks/Sparkle.framework")
}

/// The test feed, honoured only with the test control on.
fn test_feed() -> Option<String> {
    std::env::var_os(crate::control::ENV).filter(|v| !v.is_empty())?;
    std::env::var(super::TEST_FEED_ENV).ok().filter(|v| !v.is_empty())
}

/// The team ID in this process's own code signature; `None` when ad-hoc
/// or unsigned.
fn own_team_id() -> Option<String> {
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecCodeCopySelf(flags: u32, code: *mut *mut c_void) -> i32;
        fn SecCodeCopySigningInformation(code: *mut c_void, flags: u32, info: *mut *const c_void) -> i32;
        static kSecCodeInfoTeamIdentifier: *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }
    const SIGNING_INFORMATION: u32 = 1 << 1;
    unsafe {
        let mut code = std::ptr::null_mut();
        if SecCodeCopySelf(0, &mut code) != 0 || code.is_null() {
            return None;
        }
        let mut info = std::ptr::null();
        let status = SecCodeCopySigningInformation(code, SIGNING_INFORMATION, &mut info);
        CFRelease(code);
        if status != 0 || info.is_null() {
            return None;
        }
        // CFDictionary and CFString are toll-free bridged.
        let dict = &*(info as *const NSDictionary<NSString, AnyObject>);
        let key = &*(kSecCodeInfoTeamIdentifier as *const NSString);
        let team = dict
            .objectForKey(key)
            .and_then(|o| o.downcast::<NSString>().ok())
            .map(|s| s.to_string());
        CFRelease(info);
        team.filter(|t| !t.is_empty())
    }
}

/// Load Sparkle.framework and the class named `name` in it.
fn load_framework(bundle: &Path) -> Result<(), String> {
    let path = framework_path(bundle);
    let fw = NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))
        .ok_or_else(|| format!("{} is not a bundle", path.display()))?;
    unsafe { fw.loadAndReturnError() }.map_err(|e| format!("couldn't load Sparkle: {}", e.localizedDescription()))
}

/// Declare `protocol` on `cls`, at run time: Sparkle's protocols exist
/// only once it is loaded.
fn conform(cls: &AnyClass, protocol: &CStr) -> Result<(), String> {
    let p = AnyProtocol::get(protocol).ok_or_else(|| format!("Sparkle has no {protocol:?} protocol"))?;
    unsafe { objc2::ffi::class_addProtocol((cls as *const AnyClass).cast_mut(), p) };
    Ok(())
}

/// Register our classes for `Sparkle`'s protocols. Only after the load.
fn register() -> Result<(), String> {
    conform(Driver::class(), DRIVER_PROTOCOL)?;
    conform(Delegate::class(), DELEGATE_PROTOCOL)
}

fn start_sparkle(mtm: MainThreadMarker, context: Context) -> Result<(), String> {
    let main = NSBundle::mainBundle();
    load_framework(Path::new(&main.bundlePath().to_string()))?;
    register()?;
    let cls = AnyClass::get(c"SPUUpdater").ok_or("Sparkle has no SPUUpdater")?;
    let driver = Driver::new(mtm);
    let delegate = Delegate::new(mtm);
    let updater: Option<Retained<AnyObject>> = unsafe {
        let a: Allocated<AnyObject> = msg_send![cls, alloc];
        msg_send![a, initWithHostBundle: &*main, applicationBundle: &*main, userDriver: &*driver, delegate: &*delegate]
    };
    let updater = updater.ok_or("SPUUpdater init failed")?;
    // Kept before starting: Sparkle may call the delegate at once.
    RUNNING.with(|r| {
        *r.borrow_mut() = Some(Running {
            updater: updater.clone(),
            _driver: driver,
            _delegate: delegate,
            context,
        })
    });
    let mut error: *mut NSError = std::ptr::null_mut();
    let ok: bool = unsafe { msg_send![&*updater, startUpdater: &mut error] };
    if !ok {
        let why = unsafe { error.as_ref() }.map_or_else(|| "unknown error".into(), |e| e.localizedDescription().to_string());
        RUNNING.with(|r| *r.borrow_mut() = None);
        return Err(format!("Sparkle didn't start: {why}"));
    }
    Ok(())
}

/// What Sparkle's reply blocks are, copied to answer later.
enum Reply {
    None,
    Choice(RcBlock<dyn Fn(isize)>),
    Ack(RcBlock<dyn Fn()>),
    Permission(RcBlock<dyn Fn(*mut AnyObject)>),
}

impl Reply {
    fn choose(&self, c: Choice) {
        if let Reply::Choice(b) = self {
            b.call((c as isize,));
        }
    }

    fn ack(&self) {
        if let Reply::Ack(b) = self {
            b.call(());
        }
    }
}

/// Run `f` on the main queue after this call returns: Sparkle's calls
/// return before our alerts run their modal loops.
fn later(f: impl FnOnce() + 'static) {
    struct OnMain<F>(F);
    // Made and run on the main thread only.
    unsafe impl<F> Send for OnMain<F> {}
    impl<F: FnOnce()> OnMain<F> {
        fn run(self) {
            (self.0)()
        }
    }
    let f = OnMain(f);
    dispatch2::DispatchQueue::main().exec_async(move || f.run());
}

/// A standard alert: the button chosen (0, 1, …), or `None` if it was
/// closed because Sparkle ended the session.
fn alert(title: &str, text: &str, buttons: &[&str]) -> Option<usize> {
    let mtm = MainThreadMarker::new()?;
    let a = NSAlert::new(mtm);
    a.setMessageText(&NSString::from_str(title));
    a.setInformativeText(&NSString::from_str(text));
    for b in buttons {
        a.addButtonWithTitle(&NSString::from_str(b));
    }
    ALERT_OPEN.set(true);
    let r = a.runModal();
    ALERT_OPEN.set(false);
    let i = r - NSAlertFirstButtonReturn;
    (0..buttons.len() as isize).contains(&i).then_some(i as usize)
}

fn focus() {
    if let Some(mtm) = MainThreadMarker::new() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

fn own_version() -> String {
    NSBundle::mainBundle()
        .objectForInfoDictionaryKey(&NSString::from_str("CFBundleShortVersionString"))
        .and_then(|o| o.downcast::<NSString>().ok())
        .map_or_else(|| env!("CARGO_PKG_VERSION").into(), |s| s.to_string())
}

/// Do what [`answer`] said. `version` is the update's, `detail` an
/// error's text or an information-only update's link.
fn act(a: Answer, reply: Reply, version: Option<String>, detail: Option<String>) {
    let product = crate::PRODUCT_NAME;
    match a {
        Answer::Nothing => {}
        Answer::Permission { automatic_checks } => {
            let Reply::Permission(b) = reply else { return };
            let Some(cls) = AnyClass::get(c"SUUpdatePermissionResponse") else {
                return;
            };
            let response: Option<Retained<AnyObject>> = unsafe {
                let a: Allocated<AnyObject> = msg_send![cls, alloc];
                msg_send![a, initWithAutomaticUpdateChecks: automatic_checks, sendSystemProfile: false]
            };
            if let Some(r) = response {
                b.call((Retained::as_ptr(&r).cast_mut(),));
            }
        }
        Answer::Choose(c) => reply.choose(c),
        Answer::Acknowledge => reply.ack(),
        Answer::QuitAndInstall => later(move || {
            let programs = crate::app::running_programs();
            let (title, text) = super::quit_and_install_text(version.as_deref(), &programs);
            match alert(&title, &text, &["Quit and Install", "Later"]) {
                Some(0) => {
                    log("Quit and Install");
                    crate::app::allow_quit_without_asking();
                    reply.choose(Choice::Install);
                }
                Some(_) => {
                    log("Quit and Install: later");
                    reply.choose(Choice::Dismiss);
                }
                None => {}
            }
        }),
        Answer::InformationOnly => later(move || {
            let title = match &version {
                Some(v) => format!("{product} {v} is available"),
                None => format!("A new version of {product} is available"),
            };
            if alert(&title, "Its release page has the details.", &["Open Release Page", "Later"]) == Some(0)
                && let Some(url) = detail.as_deref().and_then(|u| NSURL::URLWithString(&NSString::from_str(u)))
            {
                NSWorkspace::sharedWorkspace().openURL(&url);
            }
            reply.choose(Choice::Dismiss);
        }),
        Answer::UpToDate => later(move || {
            let text = format!("{product} {} is the newest version.", own_version());
            if alert("You're up to date", &text, &["OK"]).is_some() {
                reply.ack();
            }
        }),
        Answer::ShowError => later(move || {
            let text = detail.unwrap_or_default();
            if alert("Couldn't check for updates", &text, &["OK"]).is_some() {
                reply.ack();
            }
        }),
        Answer::Focus => focus(),
        Answer::CloseAlert => {
            if ALERT_OPEN.get()
                && let Some(mtm) = MainThreadMarker::new()
            {
                NSApplication::sharedApplication(mtm).abortModal();
            }
        }
    }
}

/// `SUAppcastItem`'s flags and version.
fn read_item(item: &AnyObject) -> (Item, Option<String>, Option<String>) {
    let flags = unsafe {
        Item {
            information_only: msg_send![item, isInformationOnlyUpdate],
            critical: msg_send![item, isCriticalUpdate],
            major_upgrade: msg_send![item, isMajorUpgrade],
        }
    };
    let version: Option<Retained<NSString>> = unsafe { msg_send![item, displayVersionString] };
    let info: Option<Retained<NSURL>> = unsafe { msg_send![item, infoURL] };
    (
        flags,
        version.map(|v| v.to_string()),
        info.and_then(|u| u.absoluteString()).map(|s| s.to_string()),
    )
}

fn error_text(e: Option<&NSError>) -> String {
    e.map_or_else(|| "no error".into(), |e| e.localizedDescription().to_string())
}

pub struct DriverIvars {
    /// The session in progress is one the user started.
    user: Cell<bool>,
    /// The update found in this session.
    version: RefCell<Option<String>>,
}

define_class!(
    /// The silent `SPUUserDriver` (docs/design/updater.md §3.2).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineUpdateDriver"]
    #[ivars = DriverIvars]
    pub struct Driver;

    unsafe impl NSObjectProtocol for Driver {}

    impl Driver {
        #[unsafe(method(showUpdatePermissionRequest:reply:))]
        fn permission(&self, _request: &AnyObject, reply: &DynBlock<dyn Fn(*mut AnyObject)>) {
            act(answer(Call::PermissionRequest, false, setting()), Reply::Permission(reply.copy()), None, None);
        }

        #[unsafe(method(showUserInitiatedUpdateCheckWithCancellation:))]
        fn user_check(&self, _cancel: &DynBlock<dyn Fn()>) {
            self.ivars().user.set(true);
        }

        #[unsafe(method(showUpdateFoundWithAppcastItem:state:reply:))]
        fn found(&self, item: &AnyObject, state: &AnyObject, reply: &DynBlock<dyn Fn(isize)>) {
            let (item, version, info) = read_item(item);
            let stage = Stage::from_raw(unsafe { msg_send![state, stage] });
            let user: bool = unsafe { msg_send![state, userInitiated] };
            self.ivars().user.set(user);
            *self.ivars().version.borrow_mut() = version.clone();
            let a = answer(Call::UpdateFound(item, stage), user, setting());
            let v = version.as_deref().unwrap_or("?");
            if !user {
                log(&format!("{v} found in the background ({}): {a:?}", super::why_found(item, stage)));
            }
            act(a, Reply::Choice(reply.copy()), version, info);
        }

        #[unsafe(method(showUpdateReleaseNotesWithDownloadData:))]
        fn release_notes(&self, _data: &AnyObject) {}

        #[unsafe(method(showUpdateReleaseNotesFailedToDownloadWithError:))]
        fn release_notes_failed(&self, _error: &NSError) {}

        #[unsafe(method(showUpdateNotFoundWithError:acknowledgement:))]
        fn not_found(&self, _error: &NSError, ack: &DynBlock<dyn Fn()>) {
            let a = answer(Call::UpdateNotFound, self.ivars().user.get(), setting());
            act(a, Reply::Ack(ack.copy()), None, None);
        }

        #[unsafe(method(showUpdaterError:acknowledgement:))]
        fn error(&self, error: &NSError, ack: &DynBlock<dyn Fn()>) {
            let text = error_text(Some(error));
            log(&format!("error: {text}"));
            let a = answer(Call::UpdaterError, self.ivars().user.get(), setting());
            act(a, Reply::Ack(ack.copy()), None, Some(text));
        }

        #[unsafe(method(showDownloadInitiatedWithCancellation:))]
        fn download_started(&self, _cancel: &DynBlock<dyn Fn()>) {}

        #[unsafe(method(showDownloadDidReceiveExpectedContentLength:))]
        fn download_length(&self, _length: u64) {}

        #[unsafe(method(showDownloadDidReceiveDataOfLength:))]
        fn download_data(&self, _length: u64) {}

        #[unsafe(method(showDownloadDidStartExtractingUpdate))]
        fn extracting(&self) {}

        #[unsafe(method(showExtractionReceivedProgress:))]
        fn extraction_progress(&self, _progress: f64) {}

        #[unsafe(method(showReadyToInstallAndRelaunch:))]
        fn ready(&self, reply: &DynBlock<dyn Fn(isize)>) {
            let a = answer(Call::ReadyToInstall, self.ivars().user.get(), setting());
            let version = self.ivars().version.borrow().clone();
            act(a, Reply::Choice(reply.copy()), version, None);
        }

        #[unsafe(method(showInstallingUpdateWithApplicationTerminated:retryTerminatingApplication:))]
        fn installing(&self, _terminated: bool, _retry: &DynBlock<dyn Fn()>) {}

        #[unsafe(method(showUpdateInstalledAndRelaunched:acknowledgement:))]
        fn installed(&self, _relaunched: bool, ack: &DynBlock<dyn Fn()>) {
            act(answer(Call::InstalledAndRelaunched, false, setting()), Reply::Ack(ack.copy()), None, None);
        }

        #[unsafe(method(showUpdateInFocus))]
        fn in_focus(&self) {
            act(answer(Call::InFocus, self.ivars().user.get(), setting()), Reply::None, None, None);
        }

        #[unsafe(method(dismissUpdateInstallation))]
        fn dismiss(&self) {
            act(answer(Call::DismissInstallation, self.ivars().user.get(), setting()), Reply::None, None, None);
            self.ivars().user.set(false);
            *self.ivars().version.borrow_mut() = None;
        }
    }
);

impl Driver {
    fn new(mtm: MainThreadMarker) -> Retained<Driver> {
        let this = Self::alloc(mtm).set_ivars(DriverIvars {
            user: Cell::new(false),
            version: RefCell::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    /// The `SPUUpdaterDelegate` (docs/design/updater.md §3.3).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeclineUpdaterDelegate"]
    pub struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    impl Delegate {
        /// Decided again: the setting may have changed through a reload.
        #[unsafe(method(updater:mayPerformUpdateCheck:error:))]
        fn may_check(&self, _updater: &AnyObject, _check: isize, error: *mut *mut NSError) -> bool {
            let decision = context().map_or(Decision::Off("not started"), |c| decide(&c));
            match decision {
                Decision::On => true,
                Decision::Off(why) => {
                    log(&format!("check refused: {why}"));
                    if !error.is_null() {
                        let domain = NSString::from_str("red.forbes.terminal-decline.update");
                        let e = NSError::new(1, &domain);
                        unsafe { *error = Retained::autorelease_ptr(e) };
                    }
                    false
                }
            }
        }

        #[unsafe(method_id(feedURLStringForUpdater:))]
        fn feed(&self, _updater: &AnyObject) -> Option<Retained<NSString>> {
            test_feed().map(|f| NSString::from_str(&f))
        }

        /// Stable only.
        #[unsafe(method_id(allowedChannelsForUpdater:))]
        fn channels(&self, _updater: &AnyObject) -> Retained<NSSet<NSString>> {
            NSSet::new()
        }

        /// Keep waiting for the app to quit.
        #[unsafe(method(updater:willInstallUpdateOnQuit:immediateInstallationBlock:))]
        fn on_quit(&self, _updater: &AnyObject, item: &AnyObject, _now: &DynBlock<dyn Fn()>) -> bool {
            let (_, version, _) = read_item(item);
            log(&format!("ready: {} installs when the app quits", version.as_deref().unwrap_or("?")));
            set_status("ready");
            false
        }

        /// The app doesn't relaunch itself after Quit and Install.
        #[unsafe(method(updaterShouldRelaunchApplication:))]
        fn relaunch(&self, _updater: &AnyObject) -> bool {
            false
        }

        #[unsafe(method(updater:didFindValidUpdate:))]
        fn found(&self, _updater: &AnyObject, item: &AnyObject) {
            let (_, version, _) = read_item(item);
            log(&format!("found {}", version.as_deref().unwrap_or("?")));
            if super::status() != "ready" {
                set_status("found");
            }
        }

        #[unsafe(method(updaterDidNotFindUpdate:error:))]
        fn not_found(&self, _updater: &AnyObject, error: &NSError) {
            log(&format!("no update: {}", error_text(Some(error))));
            if super::status() != "ready" {
                set_status("none");
            }
        }

        #[unsafe(method(updater:failedToDownloadUpdate:error:))]
        fn download_failed(&self, _updater: &AnyObject, _item: &AnyObject, error: &NSError) {
            log(&format!("download failed: {}", error_text(Some(error))));
        }

        #[unsafe(method(updater:didAbortWithError:))]
        fn aborted(&self, _updater: &AnyObject, error: &NSError) {
            let text = error_text(Some(error));
            log(&format!("aborted: {text}"));
            if super::status() != "ready" {
                set_status(format!("error: {text}"));
            }
        }

        #[unsafe(method(updater:didFinishUpdateCycleForUpdateCheck:error:))]
        fn finished(&self, _updater: &AnyObject, check: isize, error: Option<&NSError>) {
            let what = ["user", "background", "information"].get(check as usize).copied().unwrap_or("?");
            match error {
                Some(e) => log(&format!("{what} check finished: {}", error_text(Some(e)))),
                None => log(&format!("{what} check finished")),
            }
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Delegate> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

/// The selectors in `protocol`'s list (`required` or optional, instance
/// methods).
fn protocol_methods(protocol: &AnyProtocol, required: bool) -> Vec<Sel> {
    #[repr(C)]
    struct MethodDescription {
        name: Option<Sel>,
        types: *const c_char,
    }
    unsafe extern "C" {
        fn protocol_copyMethodDescriptionList(
            p: *const AnyProtocol,
            required: bool,
            instance: bool,
            count: *mut u32,
        ) -> *mut MethodDescription;
    }
    let mut count = 0;
    let list = unsafe { protocol_copyMethodDescriptionList(protocol, required, true, &mut count) };
    if list.is_null() {
        return Vec::new();
    }
    let sels = unsafe { std::slice::from_raw_parts(list, count as usize) }
        .iter()
        .filter_map(|m| m.name)
        .collect();
    unsafe { libc::free(list.cast()) };
    sels
}

/// What [`protocol_methods`] finds missing: every required method of
/// Sparkle's `SPUUserDriver` our driver lacks, and every delegate method
/// ours has that Sparkle doesn't declare (a typo would never be called).
pub fn missing_methods() -> Result<Vec<String>, String> {
    let driver = AnyProtocol::get(DRIVER_PROTOCOL).ok_or("Sparkle isn't loaded")?;
    let delegate = AnyProtocol::get(DELEGATE_PROTOCOL).ok_or("Sparkle isn't loaded")?;
    let mut missing = Vec::new();
    for sel in protocol_methods(driver, true) {
        if Driver::class().instance_method(sel).is_none() {
            missing.push(format!("driver lacks -{}", sel.name().to_string_lossy()));
        }
    }
    let declared: Vec<Sel> = [true, false].into_iter().flat_map(|r| protocol_methods(delegate, r)).collect();
    for m in Delegate::class().instance_methods().iter() {
        let sel = m.name();
        if Delegate::class().superclass().is_some_and(|s| s.instance_method(sel).is_some()) {
            continue;
        }
        if !declared.contains(&sel) {
            missing.push(format!("SPUUpdaterDelegate has no -{}", sel.name().to_string_lossy()));
        }
    }
    let optional = protocol_methods(driver, false);
    for m in Driver::class().instance_methods().iter() {
        let sel = m.name();
        let known = protocol_methods(driver, true).contains(&sel) || optional.contains(&sel);
        if !known && !Driver::class().superclass().is_some_and(|s| s.instance_method(sel).is_some()) {
            missing.push(format!("SPUUserDriver has no -{}", sel.name().to_string_lossy()));
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With Sparkle fetched by `scripts/sparkle.sh` (macOS CI does), our
    /// classes answer every required method of the pinned Sparkle's
    /// driver protocol, and every method they add is Sparkle's.
    #[test]
    fn our_classes_cover_sparkles_protocols() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let Some(fw) = std::fs::read_dir(root.join("target/sparkle"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path().join("Sparkle.framework"))
            .find(|p| p.is_dir())
        else {
            eprintln!("skipped: no target/sparkle/*/Sparkle.framework (run scripts/sparkle.sh)");
            return;
        };
        let bundle = NSBundle::bundleWithPath(&NSString::from_str(&fw.to_string_lossy())).expect("a bundle");
        unsafe { bundle.loadAndReturnError() }.expect("Sparkle loads");
        register().unwrap();
        let missing = missing_methods().unwrap();
        assert!(missing.is_empty(), "{missing:#?}");
        let driver = AnyProtocol::get(DRIVER_PROTOCOL).unwrap();
        assert!(protocol_methods(driver, true).len() >= 16);
        assert!(Driver::class().conforms_to(driver));
    }
}
