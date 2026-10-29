//! Making a stub pin's bundle (docs/design/pins.md §3.1, §6): the app's
//! pin stub copied in, quarantine removed, a fresh Mach-O UUID, the frozen
//! Info.plist, the icon, one ad-hoc hardened signature with the pin
//! entitlements, then an atomic swap into place and LaunchServices.
//!
//! Nothing in a pin's bundle changes after this, so its code hash, which
//! is its identity to macOS's privacy settings, never changes either.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use term_pin::abi;

use super::{ID_PREFIX, REGISTER, Runner, succeeds, xml_escape};
use crate::paths::Paths;

/// `packaging/entitlements-pin.plist`: what every pin is signed with.
pub const ENTITLEMENTS: &str = include_str!("../../../../packaging/entitlements-pin.plist");

/// The app's bundle id, which pins look the app up by when its own
/// Info.plist doesn't say.
pub const APP_BUNDLE_ID: &str = crate::BUNDLE_ID;

/// Where the stub is inside Terminal Decline.app.
pub const STUB_IN_APP: &str = "Contents/Helpers/terminal-decline-pin";

/// The requirement a pin writes for the library it will load (§3.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// The library is ad-hoc signed: any library with our identifier.
    Dev,
    /// The library is signed by this team's Developer ID.
    Release(String),
}

impl Requirement {
    /// From how the library at `lib` is signed.
    pub fn of_library(lib: &Path) -> Result<Requirement, String> {
        Ok(match term_pin::sec::team_id(lib).map_err(|e| format!("{}: {e}", lib.display()))? {
            Some(team) => Requirement::Release(team),
            None => Requirement::Dev,
        })
    }

    /// The code requirement text (`TDLibraryRequirement`).
    pub fn text(&self) -> String {
        match self {
            Requirement::Dev => abi::dev_requirement(),
            Requirement::Release(team) => abi::team_requirement(team),
        }
    }

    /// What the pin's record says: `dev` or `release`.
    pub fn kind(&self) -> &'static str {
        match self {
            Requirement::Dev => "dev",
            Requirement::Release(_) => "release",
        }
    }
}

/// What stub pins are made from: the running Terminal Decline.app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub app: PathBuf,
    pub stub: PathBuf,
    /// The app's `CFBundleIdentifier` (`TDAppBundleId`).
    pub app_id: String,
    pub requirement: Requirement,
    /// `"<version> (<build>)"`, for the record's `made_by`.
    pub made_by: String,
}

impl Source {
    /// The app this code runs from, if it has a pin stub: `None` for a
    /// bare binary (whose pins are clones, §5.6).
    pub fn of(paths: &Paths) -> Option<Result<Source, String>> {
        let app = paths.app_bundle.clone()?;
        let stub = app.join(STUB_IN_APP);
        if !stub.is_file() {
            return None;
        }
        Some(Source::from_app(app, stub))
    }

    fn from_app(app: PathBuf, stub: PathBuf) -> Result<Source, String> {
        let lib = app.join("Contents/Frameworks").join(abi::LIBRARY);
        let requirement = Requirement::of_library(&lib)?;
        let info = term_pin::cf::InfoPlist::read(&app);
        let key = |k: &str| info.as_ref().and_then(|i| i.string(k));
        let version = key("CFBundleShortVersionString").unwrap_or_else(|| env!("CARGO_PKG_VERSION").into());
        let made_by = match key("CFBundleVersion") {
            Some(build) => format!("{version} ({build})"),
            None => version,
        };
        Ok(Source {
            app_id: key("CFBundleIdentifier").unwrap_or_else(|| APP_BUNDLE_ID.into()),
            app,
            stub,
            requirement,
            made_by,
        })
    }
}

/// Usage strings (§2.3), each "A program running in <name> (a Terminal
/// Decline pin) wants to …". The same keys as scripts/bundle.sh.
const USAGE: [(&str, &str); 22] = [
    ("NSMicrophoneUsageDescription", "use the microphone"),
    ("NSCameraUsageDescription", "use the camera"),
    ("NSAudioCaptureUsageDescription", "record audio from other apps"),
    ("NSAppleEventsUsageDescription", "control other apps"),
    ("NSAppleMusicUsageDescription", "use your music library"),
    ("NSLocationUsageDescription", "use your location"),
    ("NSLocationWhenInUseUsageDescription", "use your location"),
    ("NSContactsUsageDescription", "use your contacts"),
    ("NSCalendarsUsageDescription", "use your calendars"),
    ("NSCalendarsFullAccessUsageDescription", "use your calendars"),
    ("NSRemindersUsageDescription", "use your reminders"),
    ("NSRemindersFullAccessUsageDescription", "use your reminders"),
    ("NSPhotoLibraryUsageDescription", "use your photos"),
    ("NSBluetoothAlwaysUsageDescription", "use Bluetooth"),
    (
        "NSLocalNetworkUsageDescription",
        "find and connect to devices on your local network",
    ),
    ("NSSpeechRecognitionUsageDescription", "use speech recognition"),
    ("NSDesktopFolderUsageDescription", "use files in your Desktop folder"),
    ("NSDocumentsFolderUsageDescription", "use files in your Documents folder"),
    ("NSDownloadsFolderUsageDescription", "use files in your Downloads folder"),
    ("NSRemovableVolumesUsageDescription", "use files on a removable volume"),
    ("NSNetworkVolumesUsageDescription", "use files on a network volume"),
    ("NSSystemAdministrationUsageDescription", "change system settings"),
];

/// A stub pin's Info.plist (§3.1), written once. `name` is the display
/// name, `id` the pin's id.
pub fn info_plist(name: &str, id: &str, app_id: &str, requirement: &str) -> String {
    let n = xml_escape(name);
    let mut usage = String::new();
    for (key, what) in USAGE {
        usage.push_str(&format!(
            "\t<key>{key}</key>\n\t<string>A program running in {n} (a Terminal Decline pin) wants to {what}.</string>\n"
        ));
    }
    let v = abi::ABI_LATEST;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>{ID_PREFIX}{id}</string>
	<key>CFBundleExecutable</key>
	<string>{id}</string>
	<key>CFBundleName</key>
	<string>{n}</string>
	<key>CFBundleDisplayName</key>
	<string>{n}</string>
	<key>CFBundleIconFile</key>
	<string>icon</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{v}</string>
	<key>CFBundleVersion</key>
	<string>{v}</string>
	<key>LSMinimumSystemVersion</key>
	<string>14.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Folder</string>
			<key>CFBundleTypeRole</key>
			<string>Shell</string>
			<key>LSHandlerRank</key>
			<string>None</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>public.folder</string>
			</array>
		</dict>
	</array>
{usage}	<key>TDPinAbi</key>
	<integer>{v}</integer>
	<key>TDPinId</key>
	<string>{id}</string>
	<key>TDAppBundleId</key>
	<string>{app_id}</string>
	<key>TDLibraryRequirement</key>
	<string>{req}</string>
</dict>
</plist>
"#,
        app_id = xml_escape(app_id),
        req = xml_escape(requirement),
    )
}

const LC_UUID: u32 = 0x1b;

fn u32_at(b: &[u8], at: usize, big: bool) -> Option<u32> {
    let w: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(if big { u32::from_be_bytes(w) } else { u32::from_le_bytes(w) })
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Writes `uuid` into the `LC_UUID` of a thin Mach-O starting at `at`.
fn set_uuid_thin(b: &mut [u8], at: usize, uuid: [u8; 16]) -> Result<(), String> {
    let header = match u32_at(b, at, false) {
        Some(0xfeed_facf) => 32,
        Some(0xfeed_face) => 28,
        _ => return Err("not a Mach-O file".into()),
    };
    let ncmds = u32_at(b, at + 16, false).ok_or("truncated Mach-O header")?;
    let mut lc = at + header;
    for _ in 0..ncmds {
        let cmd = u32_at(b, lc, false).ok_or("truncated load command")?;
        let size = u32_at(b, lc + 4, false).ok_or("truncated load command")? as usize;
        if cmd == LC_UUID {
            let dest = b.get_mut(lc + 8..lc + 24).ok_or("truncated LC_UUID")?;
            dest.copy_from_slice(&uuid);
            return Ok(());
        }
        if size < 8 {
            return Err("bad load command size".into());
        }
        lc += size;
    }
    Err("no LC_UUID".into())
}

/// Gives a Mach-O file (thin or universal) a new UUID, so pins made from
/// one build don't share the stub's: macOS keeps local network permission
/// by the main executable's UUID (TN3179). Each slice gets `uuid` with
/// its index in the last byte mixed in. The signature it breaks is made
/// again right after. Returns the number of slices changed.
pub fn set_uuid(b: &mut [u8], uuid: [u8; 16]) -> Result<usize, String> {
    let fat = match u32_at(b, 0, true) {
        Some(0xcafe_babe) => Some(false),
        Some(0xcafe_babf) => Some(true),
        _ => None,
    };
    let Some(wide) = fat else {
        set_uuid_thin(b, 0, uuid)?;
        return Ok(1);
    };
    let n = u32_at(b, 4, true).ok_or("truncated fat header")? as usize;
    for i in 0..n {
        let entry = 8 + i * if wide { 32 } else { 20 };
        let offset = if wide {
            u64_at(b, entry + 8).ok_or("truncated fat header")? as usize
        } else {
            u32_at(b, entry + 8, true).ok_or("truncated fat header")? as usize
        };
        let mut u = uuid;
        u[15] ^= i as u8;
        set_uuid_thin(b, offset, u)?;
    }
    Ok(n)
}

/// A random version 4 UUID.
pub fn random_uuid() -> [u8; 16] {
    let mut u = [0u8; 16];
    // SAFETY: a valid buffer of the given length.
    unsafe { libc::arc4random_buf(u.as_mut_ptr().cast(), u.len()) };
    u[6] = (u[6] & 0x0f) | 0x40;
    u[8] = (u[8] & 0x3f) | 0x80;
    u
}

/// The pin's icon.
pub enum Icon<'a> {
    /// An `.icns` already made (generated, or the pin's old one kept).
    Icns(&'a [u8]),
    /// Any image macOS reads (`--icon FILE`).
    Image(&'a Path),
    None,
}

/// Where a bundle is put together before it is swapped in: beside `dest`,
/// named without `.app` so LaunchServices never registers a half-made pin.
pub fn staging_for(dest: &Path) -> PathBuf {
    let stem = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    dest.with_file_name(format!(".{stem}.pin-new"))
}

/// Builds the stub pin `id` named `name` at `dest`, replacing what is
/// there in one swap. A failure at any step leaves `dest` as it was.
pub fn build(src: &Source, name: &str, id: &str, dest: &Path, icon: Icon, run: &mut Runner) -> Result<(), String> {
    let parent = dest.parent().ok_or("a pin needs a folder to go in")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let stage = staging_for(dest);
    let _ = std::fs::remove_dir_all(&stage);
    let made = make(src, name, id, &stage, icon, run).and_then(|()| swap_in(&stage, dest));
    let _ = std::fs::remove_dir_all(&stage);
    made?;
    if !succeeds(run, Command::new(REGISTER).arg(dest)) {
        eprintln!("warning: LaunchServices registration failed; the Dock may not pick up the icon until you open the app once");
    }
    Ok(())
}

fn make(src: &Source, name: &str, id: &str, stage: &Path, icon: Icon, run: &mut Runner) -> Result<(), String> {
    let io = |p: &Path| {
        let p = p.to_path_buf();
        move |e: std::io::Error| format!("{}: {e}", p.display())
    };
    let contents = stage.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos).map_err(io(&macos))?;
    std::fs::create_dir_all(&resources).map_err(io(&resources))?;

    // 1. The stub, the Info.plist and the icon.
    let exe = macos.join(id);
    if !(succeeds(run, Command::new("/bin/cp").arg("-c").arg(&src.stub).arg(&exe)) && exe.is_file()) {
        std::fs::copy(&src.stub, &exe).map_err(|e| format!("copying the pin stub {}: {e}", src.stub.display()))?;
    }
    std::fs::write(
        contents.join("Info.plist"),
        info_plist(name, id, &src.app_id, &src.requirement.text()),
    )
    .map_err(io(&contents))?;
    let icns = resources.join("icon.icns");
    match icon {
        Icon::Icns(bytes) => std::fs::write(&icns, bytes).map_err(io(&icns))?,
        Icon::Image(image) => super::make_icns(image, &icns, run)?,
        Icon::None => {}
    }

    // 2. A cask-installed app's quarantine would come along with the copy.
    let _ = run(Command::new("/usr/bin/xattr").args(["-dr", "com.apple.quarantine"]).arg(stage));

    // 3. A UUID of its own.
    let mut bytes = std::fs::read(&exe).map_err(io(&exe))?;
    set_uuid(&mut bytes, random_uuid()).map_err(|e| format!("{}: {e}", src.stub.display()))?;
    std::fs::write(&exe, bytes).map_err(io(&exe))?;

    // 4. Signed once.
    let ents = stage.with_extension("entitlements.plist");
    std::fs::write(&ents, ENTITLEMENTS).map_err(io(&ents))?;
    let identifier = format!("{ID_PREFIX}{id}");
    let signed = succeeds(
        run,
        Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--options", "runtime", "--entitlements"])
            .arg(&ents)
            .args(["--identifier", &identifier])
            .arg(stage)
            .stderr(Stdio::null()),
    );
    let _ = std::fs::remove_file(&ents);
    if !signed {
        return Err(format!("codesign failed for {name}"));
    }
    Ok(())
}

/// What to say when macOS refuses to change an app bundle.
fn refused(dest: &Path, e: &std::io::Error) -> String {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if e.raw_os_error() == Some(libc::EPERM) {
        format!(
            "macOS refused to change {} (App Management): allow your terminal under System Settings › \
             Privacy & Security › App Management, or remove {name} in Finder; nothing was changed",
            dest.display()
        )
    } else {
        format!("{}: {e}; nothing was changed", dest.display())
    }
}

/// `stage` becomes `dest`: one `renamex_np(RENAME_SWAP)` when `dest`
/// exists (plain `rename` can't replace a non-empty directory), after
/// which `stage` holds the old bundle and is deleted by the caller.
fn swap_in(stage: &Path, dest: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    if dest.symlink_metadata().is_err() {
        return std::fs::rename(stage, dest).map_err(|e| refused(dest, &e));
    }
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| format!("{}: a NUL in the path", p.display()));
    let (from, to) = (c(stage)?, c(dest)?);
    // SAFETY: two NUL-terminated paths.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) } != 0 {
        return Err(refused(dest, &std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thin 64-bit Mach-O with a segment command and an LC_UUID.
    fn thin(uuid: u8) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(0xfeed_facfu32.to_le_bytes());
        b.extend([0u8; 12]);
        b.extend(2u32.to_le_bytes()); // ncmds
        b.extend(40u32.to_le_bytes()); // sizeofcmds
        b.extend([0u8; 8]);
        b.extend(0x19u32.to_le_bytes()); // LC_SEGMENT_64, short
        b.extend(16u32.to_le_bytes());
        b.extend([0u8; 8]);
        b.extend(LC_UUID.to_le_bytes());
        b.extend(24u32.to_le_bytes());
        b.extend([uuid; 16]);
        b
    }

    #[test]
    fn uuids_are_replaced_in_thin_and_universal_files() {
        let mut b = thin(7);
        assert_eq!(set_uuid(&mut b, [1; 16]), Ok(1));
        assert_eq!(&b[56..72], &[1; 16]);

        let (a, c) = (thin(7), thin(8));
        let mut fat = Vec::new();
        fat.extend(0xcafe_babeu32.to_be_bytes());
        fat.extend(2u32.to_be_bytes());
        for (off, len) in [(64u32, a.len()), (256, c.len())] {
            fat.extend([0u8; 8]);
            fat.extend(off.to_be_bytes());
            fat.extend((len as u32).to_be_bytes());
            fat.extend([0u8; 4]);
        }
        fat.resize(64, 0);
        fat.extend(&a);
        fat.resize(256, 0);
        fat.extend(&c);
        assert_eq!(set_uuid(&mut fat, [2; 16]), Ok(2));
        assert_eq!(&fat[64 + 56..64 + 72], &[2; 16]);
        let mut second = [2u8; 16];
        second[15] ^= 1;
        assert_eq!(&fat[256 + 56..256 + 72], &second);

        assert!(set_uuid(&mut b"not a binary".to_vec(), [0; 16]).is_err());
        let mut no_uuid = thin(0);
        no_uuid[16..20].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(set_uuid(&mut no_uuid, [0; 16]), Err("no LC_UUID".into()));
    }

    #[test]
    fn random_uuids_are_version_4_and_differ() {
        let (a, b) = (random_uuid(), random_uuid());
        assert_ne!(a, b);
        assert_eq!(a[6] >> 4, 4);
        assert_eq!(a[8] >> 6, 2);
    }

    /// The frozen keys (§3.1, crates/term-pin/FROZEN.md), read back the
    /// way the stub reads them.
    #[test]
    fn info_plist_has_the_frozen_keys() {
        let dir = crate::testutil::TempDir::new("pin-plist");
        let app = dir.join("R & D.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        let req = abi::team_requirement("ABCDE12345");
        std::fs::write(
            app.join("Contents/Info.plist"),
            info_plist("R & D", "r-d", "red.forbes.terminal-decline", &req),
        )
        .unwrap();
        let p = term_pin::cf::InfoPlist::read(&app).expect("a plist CoreFoundation reads");
        assert_eq!(p.integer("TDPinAbi"), Some(1));
        assert_eq!(p.string("TDPinId").as_deref(), Some("r-d"));
        assert_eq!(p.string("TDAppBundleId").as_deref(), Some("red.forbes.terminal-decline"));
        assert_eq!(p.string("TDLibraryRequirement"), Some(req));
        assert_eq!(
            p.string("CFBundleIdentifier").as_deref(),
            Some("red.forbes.terminal-decline.pin.r-d")
        );
        assert_eq!(p.string("CFBundleExecutable").as_deref(), Some("r-d"));
        assert_eq!(p.string("CFBundleName").as_deref(), Some("R & D"));
        assert_eq!(p.string("CFBundleShortVersionString").as_deref(), Some("1"));
        assert_eq!(p.string("CFBundleIconFile").as_deref(), Some("icon"));
        assert_eq!(
            p.string("NSCameraUsageDescription").as_deref(),
            Some("A program running in R & D (a Terminal Decline pin) wants to use the camera.")
        );
        let text = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
        assert!(
            text.contains("<string>public.folder</string>") && !text.contains("NSServices"),
            "{text}"
        );
        assert!(!text.contains("SUPublicEDKey") && !text.contains("SUFeedURL"));
    }

    /// The same usage strings as the app's (scripts/bundle.sh), and the
    /// pin entitlements: the app's resource entitlements plus library
    /// validation off.
    #[test]
    fn usage_strings_and_entitlements_match_the_app() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let bundle = std::fs::read_to_string(repo.join("scripts/bundle.sh")).unwrap();
        for (key, what) in USAGE {
            assert!(bundle.contains(&format!("\"{key}:{what}\"")), "{key} differs from bundle.sh");
        }
        assert_eq!(bundle.matches("UsageDescription:").count(), USAGE.len());
        let keys = |text: &str| -> Vec<String> {
            text.lines()
                .filter_map(|l| l.trim().strip_prefix("<key>")?.strip_suffix("</key>").map(String::from))
                .collect()
        };
        let mut app = keys(&std::fs::read_to_string(repo.join("packaging/entitlements.plist")).unwrap());
        app.push("com.apple.security.cs.disable-library-validation".into());
        assert_eq!(keys(ENTITLEMENTS), app);
    }

    #[test]
    fn staging_has_no_app_extension() {
        assert_eq!(staging_for(Path::new("/a/My Pin.app")), Path::new("/a/.My Pin.pin-new"));
    }

    #[test]
    fn swap_replaces_a_full_directory() {
        let dir = crate::testutil::TempDir::new("pin-swap");
        let (stage, dest) = (dir.join("new"), dir.join("X.app"));
        std::fs::create_dir_all(dest.join("old")).unwrap();
        std::fs::create_dir_all(stage.join("new")).unwrap();
        swap_in(&stage, &dest).unwrap();
        assert!(dest.join("new").exists() && stage.join("old").exists());
        std::fs::remove_dir_all(&stage).unwrap();
        // Nothing there yet: a plain rename.
        swap_in(&dest, &dir.join("Y.app")).unwrap();
        assert!(dir.join("Y.app/new").exists());
    }
}
