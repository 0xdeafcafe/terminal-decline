//! The app's [`ImageHost`] (images.md §3.4, §4.2, §6.1): pixels in Metal
//! buffers from term-render's [`PixelAlloc`], decoding with term-image's
//! pure-Rust decoders (not ImageIO, §12 question 3), and kitty's files,
//! temporary files and shared memory opened at once and read later, off
//! the terminal's lock.
//!
//! The rules for media, checked when they are opened (on the reader
//! thread, in stream order):
//! - shared memory: a name starting with `/`, no other `/`, at most 31
//!   bytes; opened read-only, mapped, and unlinked at once (mpv reuses
//!   one name for every frame);
//! - files: an absolute path; opened without following into FIFOs or
//!   devices (`O_NONBLOCK | O_NOCTTY`), regular files only, on a local
//!   volume (`MNT_LOCAL`: no network or FUSE volumes, which can hang),
//!   not an iCloud file that isn't downloaded (`SF_DATALESS`);
//! - temporary files also: the directory's real path must be `/tmp`'s or
//!   `$TMPDIR`'s, the name must contain `tty-graphics-protocol` and not
//!   be a symlink, and the file is removed at once, as kitty does (the
//!   open descriptor keeps its contents);
//! - and what is read must fit in the budget.
//!
//! Every refusal is the same [`ImageError::Refused`] or
//! [`ImageError::NotFound`], with no path or detail.

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use term_core::ImagePool;
use term_core::image::{ImageError, ImageHost, Medium, MediumRequest, OpenMedium, PixelBuffer};
use term_image::{PixelTarget, decode};
use term_render::PixelAlloc;

/// macOS's `PSHMNAMLEN`.
const SHM_NAME_MAX: usize = 31;
/// `<sys/stat.h>`: the file's contents aren't on this Mac (iCloud).
const SF_DATALESS: u32 = 0x4000_0000;
/// What temporary files must be named (kitty's rule).
const TEMP_MARKER: &[u8] = b"tty-graphics-protocol";

/// See the module documentation.
pub struct AppHost {
    /// Made on the first image, not with the host: creating the system's
    /// Metal device can take seconds in a fresh process, and every
    /// session installs this host, images on or not.
    alloc: OnceLock<Option<PixelAlloc>>,
    pool: Arc<ImagePool>,
    /// `image_file_transfer`: off, only direct transmissions work.
    media: AtomicBool,
    /// The real paths temporary files may be in.
    temp_dirs: Vec<PathBuf>,
}

/// An opened medium, read later on the decode queue.
enum Handle {
    File { file: File, offset: u64 },
    Shared(Mapping),
}

/// A read-only mapping of a shared memory object, unmapped when dropped.
struct Mapping {
    ptr: *mut libc::c_void,
    len: usize,
    offset: usize,
}

// SAFETY: a read-only mapping, never written; unmapped once, in drop.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: mapped by `open_shared` with this length.
        unsafe { libc::munmap(self.ptr, self.len) };
    }
}

impl AppHost {
    /// On the system's Metal device (the one the renderers use), with the
    /// process's pool. Without a Metal device nothing can be allocated,
    /// so every image is refused. The device is looked up on the first
    /// allocation, so making the host costs next to nothing.
    pub fn new(pool: Arc<ImagePool>, media: bool) -> AppHost {
        AppHost::with_lock(OnceLock::new(), pool, media)
    }

    pub fn with_alloc(alloc: Option<PixelAlloc>, pool: Arc<ImagePool>, media: bool) -> AppHost {
        AppHost::with_lock(OnceLock::from(alloc), pool, media)
    }

    fn with_lock(alloc: OnceLock<Option<PixelAlloc>>, pool: Arc<ImagePool>, media: bool) -> AppHost {
        let mut temp_dirs = Vec::new();
        for dir in [PathBuf::from("/tmp"), std::env::temp_dir()] {
            if let Ok(real) = dir.canonicalize()
                && !temp_dirs.contains(&real)
            {
                temp_dirs.push(real);
            }
        }
        AppHost {
            alloc,
            pool,
            media: AtomicBool::new(media),
            temp_dirs,
        }
    }

    /// Turn `image_file_transfer` on or off; opened media stay readable.
    pub fn set_media(&self, on: bool) {
        self.media.store(on, Ordering::Relaxed);
    }

    /// The bytes `req` asks for in an object of `len` bytes: (offset, length).
    fn window(&self, len: u64, req: &MediumRequest) -> Result<(u64, u64), ImageError> {
        if req.offset > len {
            return Err(ImageError::BadData("offset past the end"));
        }
        let left = len - req.offset;
        let take = if req.size == 0 { left } else { req.size.min(left) };
        if take > self.pool.limit() as u64 {
            return Err(ImageError::TooBig);
        }
        Ok((req.offset, take))
    }

    fn open_shared(&self, req: &MediumRequest) -> Result<OpenMedium, ImageError> {
        let name = &req.name;
        if name.first() != Some(&b'/') || name[1..].contains(&b'/') || name.len() > SHM_NAME_MAX {
            return Err(ImageError::Refused);
        }
        let cname = CString::new(name.clone()).map_err(|_| ImageError::Refused)?;
        // SAFETY: a valid C string; the result is checked.
        let fd = unsafe { libc::shm_open(cname.as_ptr(), libc::O_RDONLY) };
        if fd < 0 {
            return Err(ImageError::NotFound);
        }
        // SAFETY: just opened, owned here.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // Taken at once: the program may write the next frame under the
        // same name.
        // SAFETY: as above.
        unsafe { libc::shm_unlink(cname.as_ptr()) };
        let st = fstat(&fd)?;
        let (offset, len) = self.window(st.st_size.max(0) as u64, req)?;
        let map_len = (offset + len) as usize;
        if map_len == 0 {
            return Err(ImageError::BadData("empty"));
        }
        // SAFETY: a fresh read-only shared mapping of the object; the
        // result is checked.
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), map_len, libc::PROT_READ, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
        if ptr == libc::MAP_FAILED {
            return Err(ImageError::Io);
        }
        Ok(OpenMedium {
            len,
            handle: Box::new(Handle::Shared(Mapping {
                ptr,
                len: map_len,
                offset: offset as usize,
            })),
        })
    }

    fn open_file(&self, req: &MediumRequest) -> Result<OpenMedium, ImageError> {
        let path = Path::new(OsStr::from_bytes(&req.name));
        if !path.is_absolute() || req.name.contains(&0) {
            return Err(ImageError::Refused);
        }
        let temp = req.medium == Medium::TempFile;
        let flags = libc::O_NONBLOCK | libc::O_NOCTTY;
        let (file, dir) = if temp {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                return Err(ImageError::Refused);
            };
            if !name.as_bytes().windows(TEMP_MARKER.len()).any(|w| w == TEMP_MARKER) {
                return Err(ImageError::Refused);
            }
            // The directory's real path, so `..` and symlinks can't escape.
            let real = parent.canonicalize().map_err(|_| ImageError::NotFound)?;
            if !self.temp_dirs.contains(&real) {
                return Err(ImageError::Refused);
            }
            let dir = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY)
                .open(&real)
                .map_err(|_| ImageError::NotFound)?;
            let cname = CString::new(name.as_bytes()).map_err(|_| ImageError::Refused)?;
            // The file itself must not be a symlink out of the directory.
            // SAFETY: a valid directory descriptor and C string; checked.
            let fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    cname.as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | flags,
                )
            };
            if fd < 0 {
                return Err(ImageError::NotFound);
            }
            // SAFETY: just opened, owned here.
            (unsafe { File::from_raw_fd(fd) }, Some((dir, cname)))
        } else {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(flags)
                .open(path)
                .map_err(|_| ImageError::NotFound)?;
            (file, None)
        };
        let st = fstat(&file)?;
        if st.st_mode & libc::S_IFMT != libc::S_IFREG || st.st_flags & SF_DATALESS != 0 {
            return Err(ImageError::Refused);
        }
        let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
        // SAFETY: an open descriptor and a statfs to fill.
        if unsafe { libc::fstatfs(file.as_raw_fd(), &mut fs) } != 0 || fs.f_flags & libc::MNT_LOCAL as u32 == 0 {
            return Err(ImageError::Refused);
        }
        let (offset, len) = self.window(st.st_size.max(0) as u64, req)?;
        if let Some((dir, name)) = dir {
            // Removed at once, as kitty does: the descriptor keeps it.
            // SAFETY: the directory checked above and the name in it.
            unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
        }
        Ok(OpenMedium {
            len,
            handle: Box::new(Handle::File { file, offset }),
        })
    }
}

fn fstat(fd: &impl AsRawFd) -> Result<libc::stat, ImageError> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: an open descriptor and a stat to fill.
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } != 0 {
        return Err(ImageError::Io);
    }
    Ok(st)
}

impl ImageHost for AppHost {
    fn alloc(&self, width: u32, height: u32) -> Option<Box<dyn PixelBuffer>> {
        self.alloc.get_or_init(PixelAlloc::system_default).as_ref()?.alloc(width, height)
    }

    fn open_medium(&self, req: &MediumRequest) -> Result<OpenMedium, ImageError> {
        if !self.media.load(Ordering::Relaxed) {
            return Err(ImageError::Refused);
        }
        match req.medium {
            Medium::SharedMemory => self.open_shared(req),
            Medium::File | Medium::TempFile => self.open_file(req),
        }
    }

    fn read_head(&self, medium: &OpenMedium, max: usize) -> Result<Vec<u8>, ImageError> {
        let want = (medium.len.min(max as u64)) as usize;
        match medium.handle.downcast_ref::<Handle>() {
            Some(Handle::Shared(m)) => {
                // SAFETY: `offset + len` is inside the mapping (`window`).
                let bytes = unsafe { std::slice::from_raw_parts((m.ptr as *const u8).add(m.offset), want) };
                Ok(bytes.to_vec())
            }
            Some(Handle::File { file, offset }) => {
                let mut out = vec![0; want];
                let mut got = 0;
                while got < want {
                    match file.read_at(&mut out[got..], offset + got as u64) {
                        Ok(0) => break,
                        Ok(n) => got += n,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => return Err(ImageError::Io),
                    }
                }
                out.truncate(got);
                Ok(out)
            }
            None => Err(ImageError::Io),
        }
    }

    fn decode(&self, bytes: &[u8], into: &mut dyn PixelBuffer, _max_size: (u32, u32)) -> Result<(), ImageError> {
        // At `into`'s size, which the caller fitted inside `max_size`;
        // converted to Display P3 (the default options).
        let target: &mut dyn PixelTarget = into;
        match decode::decode(bytes, target, &decode::Options::default()) {
            Ok(_) => Ok(()),
            Err(decode::DecodeError::TooLarge) => Err(ImageError::TooBig),
            Err(decode::DecodeError::Unsupported) => Err(ImageError::BadData("unsupported image")),
            Err(_) => Err(ImageError::BadData("image doesn't decode")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> AppHost {
        AppHost::with_alloc(None, ImagePool::new(32 << 20), true)
    }

    /// Every session installs the host, images on or not: making it must
    /// not create the Metal device (seconds in a fresh process, which made
    /// quick failures look slow and close their windows).
    #[test]
    fn making_the_host_leaves_the_device_for_the_first_image() {
        let h = AppHost::new(ImagePool::new(32 << 20), true);
        assert!(h.alloc.get().is_none());
    }

    fn req(medium: Medium, name: &[u8]) -> MediumRequest {
        MediumRequest {
            medium,
            name: name.to_vec(),
            offset: 0,
            size: 0,
        }
    }

    fn open(h: &AppHost, medium: Medium, path: &Path) -> Result<OpenMedium, ImageError> {
        h.open_medium(&req(medium, path.as_os_str().as_bytes()))
    }

    /// A fresh directory in the temp dir, removed when dropped.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(what: &str) -> TempDir {
            let dir = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!("td-media-{what}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn files_are_read_from_where_asked() {
        let dir = TempDir::new("file");
        let path = dir.0.join("image.bin");
        std::fs::write(&path, b"0123456789").unwrap();
        let h = host();
        let m = h
            .open_medium(&MediumRequest {
                offset: 2,
                size: 5,
                ..req(Medium::File, path.as_os_str().as_bytes())
            })
            .unwrap();
        assert_eq!(m.len, 5);
        assert_eq!(h.read_head(&m, 3).unwrap(), b"234");
        assert_eq!(h.read_head(&m, 100).unwrap(), b"23456");
        assert!(path.exists(), "t=f leaves the file");
        let past = MediumRequest {
            offset: 11,
            ..req(Medium::File, path.as_os_str().as_bytes())
        };
        assert!(h.open_medium(&past).is_err());
    }

    #[test]
    fn only_regular_absolute_files() {
        let dir = TempDir::new("kinds");
        let h = host();
        assert_eq!(open(&h, Medium::File, &dir.0).unwrap_err(), ImageError::Refused, "a directory");
        assert_eq!(open(&h, Medium::File, Path::new("/dev/null")).unwrap_err(), ImageError::Refused);
        let fifo = dir.0.join("fifo");
        let c = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        // Opened without blocking, then refused.
        assert_eq!(open(&h, Medium::File, &fifo).unwrap_err(), ImageError::Refused);
        assert_eq!(
            h.open_medium(&req(Medium::File, b"relative/path.png")).unwrap_err(),
            ImageError::Refused
        );
        assert_eq!(open(&h, Medium::File, &dir.0.join("missing")).unwrap_err(), ImageError::NotFound);
    }

    #[test]
    fn files_over_the_budget_are_refused() {
        let dir = TempDir::new("big");
        let path = dir.0.join("big.bin");
        std::fs::write(&path, vec![0u8; 2000]).unwrap();
        let h = AppHost::with_alloc(None, ImagePool::new(1000), true);
        assert_eq!(open(&h, Medium::File, &path).unwrap_err(), ImageError::TooBig);
        let part = MediumRequest {
            size: 500,
            ..req(Medium::File, path.as_os_str().as_bytes())
        };
        assert_eq!(h.open_medium(&part).unwrap().len, 500);
    }

    #[test]
    fn temp_files_are_removed_when_opened() {
        let dir = TempDir::new("temp");
        let h = host();
        // Directly in the temp dir, as kitty's icat puts them.
        let tmp = std::env::temp_dir().canonicalize().unwrap();
        let path = tmp.join(format!("tty-graphics-protocol-td-{}.png", std::process::id()));
        std::fs::write(&path, b"pixels").unwrap();
        let m = open(&h, Medium::TempFile, &path).unwrap();
        assert!(!path.exists(), "removed at open");
        assert_eq!(h.read_head(&m, 100).unwrap(), b"pixels");

        // Not in the temp dir itself: a subdirectory, or `..` out of it.
        let sub = dir.0.join("tty-graphics-protocol-1.png");
        std::fs::write(&sub, b"x").unwrap();
        assert_eq!(open(&h, Medium::TempFile, &sub).unwrap_err(), ImageError::Refused);
        assert!(sub.exists(), "a refused file stays");
        let escape = dir.0.join("..").join("..").join("tty-graphics-protocol-2.png");
        assert!(open(&h, Medium::TempFile, &escape).is_err());

        // Without the marker.
        let plain = tmp.join(format!("td-plain-{}.png", std::process::id()));
        std::fs::write(&plain, b"x").unwrap();
        assert_eq!(open(&h, Medium::TempFile, &plain).unwrap_err(), ImageError::Refused);
        assert!(plain.exists());
        let _ = std::fs::remove_file(&plain);

        // A symlink in the temp dir pointing out of it.
        let target = dir.0.join("secret.png");
        std::fs::write(&target, b"secret").unwrap();
        let link = tmp.join(format!("tty-graphics-protocol-link-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(open(&h, Medium::TempFile, &link).is_err());
        assert!(target.exists());
        let _ = std::fs::remove_file(&link);
    }

    #[test]
    fn shared_memory_is_unlinked_when_opened() {
        let h = host();
        let name = format!("/td-test-{}", std::process::id());
        let c = CString::new(name.clone()).unwrap();
        unsafe {
            libc::shm_unlink(c.as_ptr());
            let fd = libc::shm_open(c.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL, 0o600 as libc::c_uint);
            assert!(fd >= 0);
            assert_eq!(libc::ftruncate(fd, 4096), 0);
            let p = libc::mmap(std::ptr::null_mut(), 4096, libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
            assert_ne!(p, libc::MAP_FAILED);
            std::ptr::copy_nonoverlapping(b"frame one".as_ptr(), p as *mut u8, 9);
            libc::munmap(p, 4096);
            libc::close(fd);
        }
        let m = h
            .open_medium(&MediumRequest {
                size: 9,
                ..req(Medium::SharedMemory, name.as_bytes())
            })
            .unwrap();
        assert_eq!(h.read_head(&m, 100).unwrap(), b"frame one");
        let again = unsafe { libc::shm_open(c.as_ptr(), libc::O_RDONLY) };
        assert!(again < 0, "unlinked at open");
        // Names: a leading slash, no other, at most 31 bytes.
        for bad in [&b"no-slash"[..], b"/a/b", b"/0123456789012345678901234567890123"] {
            assert_eq!(h.open_medium(&req(Medium::SharedMemory, bad)).unwrap_err(), ImageError::Refused);
        }
        assert_eq!(
            h.open_medium(&req(Medium::SharedMemory, b"/td-never-made")).unwrap_err(),
            ImageError::NotFound
        );
    }

    #[test]
    fn media_off_refuses_everything() {
        let h = AppHost::with_alloc(None, ImagePool::new(32 << 20), false);
        assert_eq!(h.open_medium(&req(Medium::File, b"/etc/hosts")).unwrap_err(), ImageError::Refused);
    }

    #[test]
    fn local_files_pass() {
        // /etc/hosts is on the local system volume.
        let h = host();
        assert!(h.open_medium(&req(Medium::File, b"/etc/hosts")).is_ok());
    }
}
