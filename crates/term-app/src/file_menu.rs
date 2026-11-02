//! The click menu on a file path, as Finder would have it: the file's
//! icon (a small preview for an image), name, kind, size and (for an
//! image) its dimensions at the top; then Open in its app (named), Open
//! With, Quick Look, Reveal in Finder, Copy (the file itself, to paste
//! into Finder, Mail or a chat) and Copy Path. The pane view adds the
//! items and answers their actions.

use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{NSColor, NSFont, NSImage, NSMenu, NSMenuItem, NSPasteboard, NSPasteboardTypeTIFF, NSWorkspace};
use objc2_foundation::{NSArray, NSAttributedString, NSDictionary, NSSize, NSString, NSURL};

// Quick Look's panel (`QLPreviewPanel`) lives in Quartz.
#[link(name = "Quartz", kind = "framework")]
unsafe extern "C" {}

/// Images get a preview at the top of the menu, and Copy Image.
const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "tif", "tiff", "bmp", "ico", "icns", "avif",
];

/// Larger images get their icon instead of a preview: reading one for a
/// menu shouldn't stall.
const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;

/// The most apps listed under Open With.
const MAX_APPS: usize = 24;

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// A size as Finder writes it: `512 bytes`, `4.2 KB`, `412 KB`, `1.2 MB`
/// (1000 to a kilobyte).
pub fn size_text(bytes: u64) -> String {
    if bytes < 1000 {
        return if bytes == 1 { "1 byte".into() } else { format!("{bytes} bytes") };
    }
    let mut value = bytes as f64;
    for unit in ["KB", "MB", "GB", "TB"] {
        value /= 1000.0;
        if value < 999.5 || unit == "TB" {
            return if value < 10.0 {
                format!("{value:.1} {unit}")
            } else {
                format!("{value:.0} {unit}")
            };
        }
    }
    unreachable!()
}

/// The line under the name: kind, size (not for a folder) and, for an
/// image, its pixels.
pub fn detail_text(kind: Option<&str>, size: Option<u64>, pixels: Option<(isize, isize)>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(k) = kind.filter(|k| !k.is_empty()) {
        parts.push(k.to_string());
    }
    if let Some(s) = size {
        parts.push(size_text(s));
    }
    if let Some((w, h)) = pixels.filter(|&(w, h)| w > 0 && h > 0) {
        parts.push(format!("{w}\u{d7}{h}"));
    }
    parts.join(" \u{b7} ")
}

/// An app's name as Finder shows it: `Preview` for `/System/Applications/Preview.app`.
pub fn app_name(app: &Path) -> String {
    app.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn file_url(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
}

fn url_path(url: &NSURL) -> Option<PathBuf> {
    url.path().map(|p| PathBuf::from(p.to_string()))
}

/// What Finder calls the file's kind: `PNG image`, `Folder`, `Rust Source`.
fn kind_of(url: &NSURL) -> Option<String> {
    let mut value: Option<Retained<AnyObject>> = None;
    let key = NSString::from_str("NSURLLocalizedTypeDescriptionKey");
    let ok: bool = unsafe { msg_send![url, getResourceValue: &mut value, forKey: &*key, error: std::ptr::null_mut::<*mut AnyObject>()] };
    let value = value.filter(|_| ok)?;
    value.downcast_ref::<NSString>().map(|s| s.to_string())
}

/// The app that opens `path` when it is opened.
pub fn default_app(path: &Path) -> Option<PathBuf> {
    let url = file_url(path);
    let app: Option<Retained<NSURL>> = unsafe { msg_send![&*NSWorkspace::sharedWorkspace(), URLForApplicationToOpenURL: &*url] };
    url_path(&app?)
}

/// Every app that says it can open `path`, the default first, then by
/// name, each name once.
pub fn apps_for(path: &Path) -> Vec<PathBuf> {
    let url = file_url(path);
    let found: Retained<NSArray<NSURL>> = unsafe { msg_send![&*NSWorkspace::sharedWorkspace(), URLsForApplicationsToOpenURL: &*url] };
    let mut apps: Vec<PathBuf> = found.iter().filter_map(|u| url_path(&u)).collect();
    apps.sort_by_key(|a| app_name(a).to_lowercase());
    let default = default_app(path);
    if let Some(d) = &default {
        apps.retain(|a| a != d);
        apps.insert(0, d.clone());
    }
    let mut names = std::collections::HashSet::new();
    apps.retain(|a| names.insert(app_name(a)));
    apps.truncate(MAX_APPS);
    apps
}

/// `path`'s icon as Finder draws it, `side` points square.
fn icon(path: &Path, side: f64) -> Retained<NSImage> {
    let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
    image.setSize(NSSize::new(side, side));
    image
}

/// The image in the file, and its pixels, unless it is too big to read
/// for a menu.
fn picture(path: &Path, size: u64) -> Option<(Retained<NSImage>, Option<(isize, isize)>)> {
    if size > MAX_PREVIEW_BYTES {
        return None;
    }
    let image = NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(&path.to_string_lossy()))?;
    let reps: Retained<NSArray<AnyObject>> = unsafe { msg_send![&*image, representations] };
    let pixels = reps.firstObject().map(|rep| {
        let w: isize = unsafe { msg_send![&*rep, pixelsWide] };
        let h: isize = unsafe { msg_send![&*rep, pixelsHigh] };
        (w, h)
    });
    Some((image, pixels))
}

/// `image` scaled to fit `side` points square, keeping its shape.
fn fit(image: &NSImage, side: f64) {
    let s = image.size();
    if s.width <= 0.0 || s.height <= 0.0 {
        return;
    }
    let k = side / s.width.max(s.height);
    image.setSize(NSSize::new(s.width * k, s.height * k));
}

/// Text in `font` and `color`.
fn styled(text: &str, font: &NSFont, color: &NSColor) -> Retained<NSAttributedString> {
    let attrs: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
        &[&*NSString::from_str("NSFont"), &*NSString::from_str("NSColor")],
        &[font as &AnyObject, color as &AnyObject],
    );
    unsafe { msg_send![NSAttributedString::alloc(), initWithString: &*NSString::from_str(text), attributes: &*attrs] }
}

/// The greyed item at the top: the icon or preview, the name in bold and
/// the detail line under it.
fn header(mtm: MainThreadMarker, path: &Path) -> Retained<NSMenuItem> {
    let url = file_url(path);
    let meta = std::fs::metadata(path).ok();
    let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
    let size = meta.as_ref().filter(|_| !is_dir).map(|m| m.len());
    let shown = if is_image(path) {
        size.and_then(|s| picture(path, s))
    } else {
        None
    };
    let pixels = shown.as_ref().and_then(|(_, p)| *p);
    let image = match shown {
        Some((image, _)) => {
            fit(&image, 48.0);
            image
        }
        None => icon(path, 32.0),
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let detail = detail_text(kind_of(&url).as_deref(), size, pixels);
    let title: Retained<NSAttributedString> = unsafe { msg_send![AnyClass::get(c"NSMutableAttributedString").expect("Foundation"), new] };
    let bold = NSFont::boldSystemFontOfSize(13.0);
    let small = NSFont::systemFontOfSize(11.0);
    let parts = [
        styled(&name, &bold, &NSColor::labelColor()),
        styled(&format!("\n{detail}"), &small, &NSColor::secondaryLabelColor()),
    ];
    for part in &parts {
        let _: () = unsafe { msg_send![&*title, appendAttributedString: &**part] };
    }
    let item = NSMenuItem::new(mtm);
    let _: () = unsafe { msg_send![&*item, setAttributedTitle: &*title] };
    item.setImage(Some(&*image));
    item.setEnabled(false);
    item
}

/// Add the menu for the file at `path`, its items sent to `target`.
pub fn add_items(menu: &NSMenu, mtm: MainThreadMarker, path: &Path, target: &AnyObject) {
    let add = |menu: &NSMenu, title: &str, action: Sel| -> Retained<NSMenuItem> {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::new(),
            )
        };
        unsafe { item.setTarget(Some(target)) };
        menu.addItem(&item);
        item
    };
    menu.addItem(&header(mtm, path));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let apps = apps_for(path);
    match apps.first() {
        Some(app) => add(menu, &format!("Open in {}", app_name(app)), sel!(clickOpen:)),
        None => add(menu, "Open", sel!(clickOpen:)),
    };
    if !apps.is_empty() {
        let with = NSMenu::new(mtm);
        with.setAutoenablesItems(false);
        for (k, app) in apps.iter().enumerate() {
            let title = if k == 0 {
                format!("{} (default)", app_name(app))
            } else {
                app_name(app)
            };
            let item = add(&with, &title, sel!(clickOpenWith:));
            item.setImage(Some(&*icon(app, 16.0)));
            let app_path = NSString::from_str(&app.to_string_lossy());
            unsafe { item.setRepresentedObject(Some(&*app_path as &AnyObject)) };
            if k == 0 && apps.len() > 1 {
                with.addItem(&NSMenuItem::separatorItem(mtm));
            }
        }
        let parent = NSMenuItem::new(mtm);
        parent.setTitle(&NSString::from_str("Open With"));
        parent.setSubmenu(Some(&*with));
        menu.addItem(&parent);
    }
    add(menu, "Quick Look", sel!(clickQuickLook:));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add(menu, "Reveal in Finder", sel!(clickReveal:));
    let copy = if is_image(path) {
        "Copy Image"
    } else if path.is_dir() {
        "Copy Folder"
    } else {
        "Copy File"
    };
    add(menu, copy, sel!(clickCopyFile:));
    add(menu, "Copy Path", sel!(clickCopy:));
}

/// Open `path` in the app at `app`. False if it couldn't.
pub fn open_with(path: &Path, app: &str) -> bool {
    let file = NSString::from_str(&path.to_string_lossy());
    let app = NSString::from_str(app);
    unsafe { msg_send![&*NSWorkspace::sharedWorkspace(), openFile: &*file, withApplication: &*app] }
}

/// Put the file itself on the clipboard, as Finder's Copy does (pasted
/// into Finder it is copied there; into Mail or a chat, attached), and
/// for an image its picture too, for apps that take a picture.
pub fn copy_file(path: &Path) {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    let urls = NSArray::from_retained_slice(&[file_url(path)]);
    let _: bool = unsafe { msg_send![&*pb, writeObjects: &*urls] };
    if is_image(path)
        && let Some((image, _)) = std::fs::metadata(path).ok().and_then(|m| picture(path, m.len()))
    {
        let tiff: Option<Retained<AnyObject>> = unsafe { msg_send![&*image, TIFFRepresentation] };
        if let Some(tiff) = tiff {
            let _: bool = unsafe { msg_send![&*pb, setData: &*tiff, forType: NSPasteboardTypeTIFF] };
        }
    }
}

/// Show Quick Look's panel; it asks the responder chain (the pane view)
/// what to show.
pub fn quick_look() {
    let Some(class) = AnyClass::get(c"QLPreviewPanel") else { return };
    let panel: Option<Retained<AnyObject>> = unsafe { msg_send![class, sharedPreviewPanel] };
    let Some(panel) = panel else { return };
    let visible: bool = unsafe { msg_send![&*panel, isVisible] };
    if visible {
        let _: () = unsafe { msg_send![&*panel, reloadData] };
    }
    let _: () = unsafe { msg_send![&*panel, makeKeyAndOrderFront: None::<&AnyObject>] };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_as_finder_writes_them() {
        assert_eq!(size_text(0), "0 bytes");
        assert_eq!(size_text(1), "1 byte");
        assert_eq!(size_text(999), "999 bytes");
        assert_eq!(size_text(4_200), "4.2 KB");
        assert_eq!(size_text(412_000), "412 KB");
        assert_eq!(size_text(1_200_000), "1.2 MB");
        assert_eq!(size_text(999_999_999), "1.0 GB");
        assert_eq!(size_text(3_000_000_000), "3.0 GB");
    }

    #[test]
    fn detail_lines() {
        assert_eq!(
            detail_text(Some("WebP image"), Some(412_000), Some((5120, 304))),
            "WebP image \u{b7} 412 KB \u{b7} 5120\u{d7}304"
        );
        assert_eq!(detail_text(Some("Folder"), None, None), "Folder");
        assert_eq!(detail_text(None, Some(10), Some((0, 0))), "10 bytes");
    }

    #[test]
    fn images_by_extension() {
        assert!(is_image(Path::new("/a/Screenshot 2026.PNG")));
        assert!(is_image(Path::new("x.webp")));
        assert!(!is_image(Path::new("x.rs")));
        assert!(!is_image(Path::new("png")));
        assert_eq!(app_name(Path::new("/System/Applications/Preview.app")), "Preview");
    }
}
