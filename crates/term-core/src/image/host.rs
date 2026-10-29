//! A pure-Rust [`ImageHost`] (images.md §4.2, §10) and [`run_job`], which
//! runs an [`ImageJob`] with the terminal's lock released.
//!
//! [`RustHost`] allocates `Vec`-backed buffers and decodes with
//! term-image. With media off (the default, [`RustHost::new`]) only
//! direct transmissions work, so a terminal with no host of the app's yet
//! still shows images sent in the stream. [`RustHost::with_media`] also
//! opens files with `std::fs` (portable, with simpler rules than the
//! app's: regular files only; temporary files only in the temp
//! directory, named `tty-graphics-protocol`, removed when opened) and
//! keeps shared memory in a map, for tests.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use term_image::convert::{Converter, Layout};
use term_image::{PixelTarget, decode, inflate};

use super::{AfterRead, ImageError, ImageHost, ImageJob, JobDone, Medium, MediumRequest, OpenMedium, PixelBuffer, VecBuffer};

/// See the module documentation.
#[derive(Default)]
pub struct RustHost {
    media: bool,
    shared: Mutex<HashMap<Vec<u8>, Vec<u8>>>,
}

enum Handle {
    File(Mutex<std::fs::File>, u64),
    Bytes(Arc<Vec<u8>>, usize),
}

impl RustHost {
    /// Direct transmissions only: files and shared memory are refused.
    pub fn new() -> RustHost {
        RustHost::default()
    }

    /// Files and (fake) shared memory too.
    pub fn with_media() -> RustHost {
        RustHost {
            media: true,
            ..RustHost::default()
        }
    }

    /// Make a shared memory object `name` holding `bytes` (what a program
    /// would do with `shm_open` and `mmap`).
    pub fn put_shared_memory(&self, name: &[u8], bytes: Vec<u8>) {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_vec(), bytes);
    }

    /// Whether shared memory object `name` still exists (it is unlinked
    /// when opened).
    pub fn has_shared_memory(&self, name: &[u8]) -> bool {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).contains_key(name)
    }
}

fn window(len: u64, offset: u64, size: u64) -> Result<(u64, u64), ImageError> {
    if offset > len {
        return Err(ImageError::BadData("offset past the end"));
    }
    let left = len - offset;
    Ok((offset, if size == 0 { left } else { size.min(left) }))
}

impl ImageHost for RustHost {
    fn alloc(&self, width: u32, height: u32) -> Option<Box<dyn PixelBuffer>> {
        Some(Box::new(VecBuffer::new(width, height)))
    }

    fn open_medium(&self, req: &MediumRequest) -> Result<OpenMedium, ImageError> {
        if !self.media {
            return Err(ImageError::Refused);
        }
        match req.medium {
            Medium::SharedMemory => {
                let name = &req.name;
                if name.first() != Some(&b'/') || name[1..].contains(&b'/') || name.len() > 31 {
                    return Err(ImageError::Refused);
                }
                // Taken out of the map: unlinked, as the app does at once.
                let bytes = self
                    .shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(name)
                    .ok_or(ImageError::NotFound)?;
                let (offset, len) = window(bytes.len() as u64, req.offset, req.size)?;
                Ok(OpenMedium {
                    len,
                    handle: Box::new(Handle::Bytes(Arc::new(bytes), offset as usize)),
                })
            }
            Medium::File | Medium::TempFile => {
                let path = std::path::PathBuf::from(String::from_utf8(req.name.clone()).map_err(|_| ImageError::Refused)?);
                if req.medium == Medium::TempFile {
                    let name_ok = path
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().contains("tty-graphics-protocol"));
                    let parent = path.parent().and_then(|p| p.canonicalize().ok());
                    let temp = std::env::temp_dir().canonicalize().ok();
                    let tmp = std::path::Path::new("/tmp").canonicalize().ok();
                    if !name_ok || parent.is_none() || (parent != temp && parent != tmp) {
                        return Err(ImageError::Refused);
                    }
                }
                let file = std::fs::File::open(&path).map_err(|_| ImageError::NotFound)?;
                let meta = file.metadata().map_err(|_| ImageError::Io)?;
                if !meta.is_file() {
                    return Err(ImageError::Refused);
                }
                if req.medium == Medium::TempFile {
                    let _ = std::fs::remove_file(&path);
                }
                let (offset, len) = window(meta.len(), req.offset, req.size)?;
                Ok(OpenMedium {
                    len,
                    handle: Box::new(Handle::File(Mutex::new(file), offset)),
                })
            }
        }
    }

    fn read_head(&self, medium: &OpenMedium, max: usize) -> Result<Vec<u8>, ImageError> {
        let want = (medium.len as usize).min(max);
        match medium.handle.downcast_ref::<Handle>() {
            Some(Handle::Bytes(b, offset)) => Ok(b[*offset..*offset + want].to_vec()),
            Some(Handle::File(f, offset)) => {
                let mut f = f.lock().unwrap_or_else(|e| e.into_inner());
                f.seek(SeekFrom::Start(*offset)).map_err(|_| ImageError::Io)?;
                let mut out = Vec::with_capacity(want);
                (&mut *f).take(want as u64).read_to_end(&mut out).map_err(|_| ImageError::Io)?;
                Ok(out)
            }
            None => Err(ImageError::Io),
        }
    }

    fn decode(&self, bytes: &[u8], into: &mut dyn PixelBuffer, _max_size: (u32, u32)) -> Result<(), ImageError> {
        let target: &mut dyn PixelTarget = into;
        decode::decode(bytes, target, &decode::Options::default())
            .map(|_| ())
            .map_err(|_| ImageError::BadData("image doesn't decode"))
    }
}

/// Raw pixels converted into a new buffer.
fn convert(host: &dyn ImageHost, data: &[u8], size: (u32, u32), has_alpha: bool) -> Result<(Arc<dyn PixelBuffer>, bool), ImageError> {
    let mut buf = host.alloc(size.0, size.1).ok_or(ImageError::TooBig)?;
    let layout = if has_alpha { Layout::Rgba } else { Layout::Rgb };
    let mut conv = Converter::new(layout, size.0, size.1).display_p3();
    let target: &mut dyn PixelTarget = &mut *buf;
    conv.feed(data, target);
    let done = conv.finish().map_err(|_| ImageError::BadData("insufficient image data"))?;
    Ok((Arc::from(buf), done.opaque))
}

/// Run `job` (with no terminal lock held) and say how it went, for
/// [`crate::Terminal::image_job_done`].
pub fn run_job(host: &dyn ImageHost, terminal_id: u64, job: ImageJob) -> JobDone {
    let (id, image, generation) = match &job {
        ImageJob::Decode {
            job, image, generation, ..
        }
        | ImageJob::Read {
            job, image, generation, ..
        } => (*job, *image, *generation),
    };
    let mut opaque = false;
    let result = match job {
        ImageJob::Decode { bytes, size, .. } => decode_into(host, &bytes.to_vec(), size),
        ImageJob::Read {
            medium,
            then,
            size,
            compressed,
            ..
        } => (|| {
            let mut data = host.read_head(&medium, medium.len as usize)?;
            if compressed {
                let cap = match then {
                    AfterRead::Raw { has_alpha } => size.0 as usize * size.1 as usize * if has_alpha { 4 } else { 3 },
                    AfterRead::Decode => 256 << 20,
                };
                data = inflate::inflate_all(&data, cap).map_err(|_| ImageError::BadData("bad zlib data"))?;
            }
            match then {
                AfterRead::Raw { has_alpha } => convert(host, &data, size, has_alpha).map(|(p, o)| {
                    opaque = o;
                    p
                }),
                AfterRead::Decode => decode_into(host, &data, size),
            }
        })(),
    };
    JobDone {
        job: id,
        terminal_id,
        image,
        generation,
        result,
        opaque,
    }
}

fn decode_into(host: &dyn ImageHost, bytes: &[u8], size: (u32, u32)) -> Result<Arc<dyn PixelBuffer>, ImageError> {
    let mut buf = host.alloc(size.0, size.1).ok_or(ImageError::TooBig)?;
    host.decode(bytes, &mut *buf, size)?;
    Ok(Arc::from(buf))
}
