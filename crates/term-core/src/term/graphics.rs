//! kitty graphics (images.md §3): `ESC _ G <keys> ; <payload> ESC \`
//! routed from vte's APC calls, chunked uploads, transmit, place, query
//! and delete, and the replies.
//!
//! - **Routing**: the first byte of an APC string picks it: `G` with
//!   images on is graphics, anything else is skipped. Keys are collected
//!   up to the `;` (at most 4 KB), then the payload streams through
//!   base64 (fresh in each chunk, as kitty decodes each chunk on its own),
//!   zlib if `o=z`, and into the upload's sink: the pixel buffer for raw
//!   data, an encoded buffer for PNG, a short name for files and shared
//!   memory.
//! - **Chunks**: one upload in progress per terminal. While it is, a
//!   transmit with `t=d` is its next chunk (only `m` and `q` read from
//!   it), a delete abandons it, anything else runs and the upload waits.
//! - **Media** are opened and checked at once, through the host; their
//!   data is read by a job with the lock released. A PNG from a file
//!   needs its size before the next byte is parsed: with
//!   [`Terminal::set_reads_off_lock`] the parser stops after the command
//!   ([`Terminal::take_head_read`]) so the session can read the header
//!   unlocked; otherwise it is read at once.
//! - **Replies** go on the reply buffer in stream order, so a query's
//!   reply comes before anything sent after it (DA1, say). PNG and media
//!   transmissions reply when their job is done.

use std::sync::Arc;

use term_image::base64::Base64;
use term_image::convert::{Converter, Layout};
use term_image::inflate::Inflate;
use term_image::kitty::{self, Action, Command, DeleteWhat, Medium as KMedium};
use term_image::{MAX_SIDE, PixelTarget, probe};

use super::Terminal;
use crate::image::{
    AfterRead, Encoded, Fit, Image, ImageError, ImageJob, ImageKey, ImagePool, ImageState, JobId, Medium, MediumRequest, OpenMedium, PAGE,
    PixelBuffer, Placement, PoolId, VirtualPlacement,
};

/// Where an APC string is.
#[derive(Default)]
pub(super) enum Apc {
    #[default]
    Idle,
    /// Started; its first byte not seen yet.
    Start,
    /// A graphics command's keys so far.
    Keys(Vec<u8>),
    /// The payload, going to the upload.
    Payload,
    /// The payload of a command that takes none, or of a skipped string.
    Ignore,
}

/// A failed command's reply: kitty's code and a short text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Failure(&'static str, &'static str);

const EINVAL: Failure = Failure("EINVAL", "bad command");
const ENOENT: Failure = Failure("ENOENT", "no such image");
const EBADF: Failure = Failure("EBADF", "cannot use the medium");
const ENODATA: Failure = Failure("ENODATA", "insufficient image data");
const EFBIG: Failure = Failure("EFBIG", "image too big");
const EBADPNG: Failure = Failure("EBADPNG", "image doesn't decode");

impl Failure {
    fn of(e: &ImageError) -> Failure {
        match e {
            ImageError::NotFound | ImageError::Refused | ImageError::Io => EBADF,
            ImageError::TooBig => EFBIG,
            ImageError::BadData(_) => EBADPNG,
        }
    }
}

/// Who a reply goes to: the keys a reply repeats, and how quiet.
#[derive(Clone, Copy, Debug)]
pub(super) struct ReplyTo {
    id: u32,
    number: u32,
    placement: u32,
    quiet: u8,
}

impl ReplyTo {
    fn of(cmd: &Command, id: u32) -> ReplyTo {
        ReplyTo {
            id,
            number: cmd.image_number,
            placement: cmd.placement_id,
            quiet: cmd.quiet,
        }
    }
}

/// Where an upload's bytes go.
enum Sink {
    /// Raw pixels, converted into the buffer as they come.
    Raw(Box<dyn PixelBuffer>, Converter),
    /// PNG (or a query's data), counted against the budget piece by piece.
    Encoded(Encoded),
    /// A path or shared memory name.
    Name(Vec<u8>),
    /// Bytes counted only (a raw query, or an upload already failed).
    Count(usize),
}

/// The upload's bytes and their accounting, apart from its decoders so
/// both can be borrowed at once.
struct Store {
    sink: Sink,
    /// The pool entry the bytes count in (none for queries), released
    /// when the store is dropped unless taken.
    pool: Option<PoolId>,
    pool_handle: Arc<ImagePool>,
    /// Bytes reserved for `Encoded` so far.
    counted: usize,
    /// Most decoded bytes accepted.
    cap: usize,
    failed: Option<Failure>,
}

/// A transmission in progress (§3.2).
pub(super) struct Upload {
    cmd: Command,
    /// The image it makes (none for queries and id-less `a=t`).
    key: Option<ImageKey>,
    base64: Base64,
    inflate: Option<Inflate>,
    store: Store,
}

/// A command waiting for its medium's header, read with the lock released.
pub(super) struct Parked {
    cmd: Command,
    key: ImageKey,
    medium: Option<OpenMedium>,
}

/// A header to read with the terminal's lock released: take it with
/// [`Terminal::take_head_read`], read `max` bytes of [`HeadRead::medium`]
/// through the host, and hand them to [`Terminal::head_read_done`].
pub struct HeadRead {
    medium: OpenMedium,
    pub max: usize,
}

impl HeadRead {
    pub fn medium(&self) -> &OpenMedium {
        &self.medium
    }
}

/// How much of a medium is looked at for its header (§3.3).
const HEAD: usize = 64 * 1024;
/// Most bytes of a file path or shared memory name.
const MAX_NAME: usize = 4096;

fn page_round(bytes: usize) -> usize {
    bytes.div_ceil(PAGE).max(1) * PAGE
}

/// A raw image's pixel bytes as the pool counts them.
fn pixel_bytes(w: u32, h: u32) -> usize {
    page_round(w as usize * h as usize * 4)
}

impl Drop for Store {
    fn drop(&mut self) {
        if let Some(id) = self.pool.take() {
            self.pool_handle.release(id);
        }
    }
}

impl Store {
    fn put(&mut self, bytes: &[u8], pool: &ImagePool) {
        if self.failed.is_some() {
            return;
        }
        match &mut self.sink {
            Sink::Raw(buf, conv) => {
                let target: &mut dyn PixelTarget = &mut **buf;
                conv.feed(bytes, target);
            }
            Sink::Encoded(e) => {
                if e.len() + bytes.len() > self.cap {
                    if self.pool.is_some() {
                        self.failed = Some(EFBIG);
                    }
                    // A query looks at the start only.
                    return;
                }
                e.push(bytes);
                let need = e.len().div_ceil(Encoded::PIECE) * Encoded::PIECE;
                if let Some(id) = self.pool
                    && need > self.counted
                {
                    if pool.grow(id, need - self.counted).is_err() {
                        self.failed = Some(EFBIG);
                        return;
                    }
                    self.counted = need;
                }
            }
            Sink::Name(n) => {
                if n.len() + bytes.len() > MAX_NAME {
                    self.failed = Some(EINVAL);
                } else {
                    n.extend_from_slice(bytes);
                }
            }
            Sink::Count(n) => *n += bytes.len(),
        }
    }
}

impl Terminal {
    /// Read media headers with the lock released: the parser stops after
    /// a command that needs one ([`Terminal::take_head_read`]). Off by
    /// default, when headers are read at once through the host.
    pub fn set_reads_off_lock(&mut self, on: bool) {
        self.reads_off_lock = on;
    }

    /// The host media and decoding go through (kept across RIS).
    pub fn set_image_host(&mut self, host: Arc<dyn crate::image::ImageHost>) {
        self.image_host = host;
    }

    pub fn image_host(&self) -> &Arc<dyn crate::image::ImageHost> {
        &self.image_host
    }

    /// A command is waiting for a medium's header: read it with the lock
    /// released and call [`Terminal::head_read_done`] before parsing on.
    pub fn take_head_read(&mut self) -> Option<HeadRead> {
        let parked = self.parked.as_mut()?;
        Some(HeadRead {
            medium: parked.medium.take()?,
            max: HEAD,
        })
    }

    /// The header [`Terminal::take_head_read`] asked for was read.
    pub fn head_read_done(&mut self, read: HeadRead, head: Result<Vec<u8>, ImageError>) {
        if let Some(p) = self.parked.take() {
            self.finish_media_png(p.cmd, p.key, read.medium, head);
        }
    }

    /// Whether the parser must stop for a header read.
    pub(super) fn waiting_for_head(&self) -> bool {
        self.parked.is_some()
    }

    pub(super) fn apc_put_bytes(&mut self, data: &[u8]) {
        let mut data = data;
        if let Apc::Start = self.apc {
            if data.is_empty() {
                return;
            }
            self.apc = if data[0] == b'G' && self.images_enabled() {
                Apc::Keys(Vec::new())
            } else {
                Apc::Ignore
            };
            data = &data[1..];
        }
        if let Apc::Keys(keys) = &mut self.apc {
            match memchr::memchr(b';', data) {
                Some(at) => {
                    keys.extend_from_slice(&data[..at]);
                    data = &data[at + 1..];
                    let keys = std::mem::take(keys);
                    self.kitty_begin(&keys);
                }
                None => {
                    if keys.len() + data.len() > kitty::MAX_KEYS_LEN {
                        self.apc = Apc::Ignore;
                    } else {
                        keys.extend_from_slice(data);
                    }
                    return;
                }
            }
        }
        if let Apc::Payload = self.apc
            && !data.is_empty()
        {
            self.kitty_payload(data);
        }
    }

    pub(super) fn apc_finish(&mut self, complete: bool) {
        if let Apc::Keys(keys) = &mut self.apc {
            let keys = std::mem::take(keys);
            self.kitty_begin(&keys);
        }
        let state = std::mem::take(&mut self.apc);
        if let Apc::Idle | Apc::Start = state {
            return;
        }
        if !complete {
            // Abandoned: the whole transmission goes.
            if matches!(state, Apc::Payload) {
                self.abandon_upload();
            }
            self.chunk = None;
            return;
        }
        if let Some(cmd) = self.chunk.take() {
            self.kitty_end(cmd);
        }
    }

    /// The keys are in: decide where the payload goes.
    fn kitty_begin(&mut self, keys: &[u8]) {
        let cmd = match kitty::Keys::parse(keys) {
            Ok(c) => c,
            Err(_) => {
                // Reply if the keys say who to.
                if let Ok(k) = kitty::Keys::read(keys) {
                    let id = k.get(b'i').unwrap_or(0);
                    let number = k.get(b'I').unwrap_or(0);
                    let quiet = k.get(b'q').unwrap_or(0).min(2) as u8;
                    self.reply(
                        ReplyTo {
                            id,
                            number,
                            placement: 0,
                            quiet,
                        },
                        number != 0 || id != 0,
                        Err(EINVAL),
                    );
                }
                self.apc = Apc::Ignore;
                return;
            }
        };
        self.apc = Apc::Ignore;
        if cmd.unsupported {
            // As kitty does with an unknown action: nothing, no reply.
            return;
        }
        let transmit = matches!(cmd.action, Action::Transmit | Action::TransmitPlace | Action::Query);
        if self.upload.is_some() {
            if transmit && cmd.medium == KMedium::Direct {
                // The next chunk of the upload in progress.
                if let Some(up) = &mut self.upload {
                    up.cmd.more = cmd.more;
                    if keys.contains(&b'q') {
                        up.cmd.quiet = cmd.quiet;
                    }
                }
                self.apc = Apc::Payload;
                self.chunk = Some(cmd);
                return;
            }
            if cmd.action == Action::Delete || transmit {
                self.abandon_upload();
            }
        }
        if transmit {
            self.start_upload(cmd);
            if self.upload.is_some() {
                self.apc = Apc::Payload;
            }
        }
        self.chunk = Some(cmd);
    }

    fn kitty_payload(&mut self, data: &[u8]) {
        let pool = self.graphics.pool().clone();
        let Some(up) = self.upload.as_deref_mut() else { return };
        let Upload {
            base64, inflate, store, ..
        } = up;
        let mut bad = false;
        let r = base64.feed_to(data, &mut |bytes| match inflate {
            Some(inf) => {
                if inf.feed_to(bytes, &mut |b| store.put(b, &pool)).is_err() {
                    bad = true;
                }
            }
            None => store.put(bytes, &pool),
        });
        if (r.is_err() || bad) && store.failed.is_none() {
            store.failed = Some(EINVAL);
        }
    }

    /// The end of a chunk (`complete`): finish what it started.
    fn kitty_end(&mut self, cmd: Command) {
        let transmit = matches!(cmd.action, Action::Transmit | Action::TransmitPlace | Action::Query);
        if !transmit || self.upload.is_none() {
            match cmd.action {
                Action::Place => self.kitty_place(&cmd),
                Action::Delete => self.kitty_delete(&cmd),
                _ => {}
            }
            return;
        }
        // The chunk's base64 ends here.
        let pool = self.graphics.pool().clone();
        let up = self.upload.as_deref_mut().expect("an upload");
        {
            let Upload {
                base64, inflate, store, ..
            } = up;
            let mut bad = false;
            let r = base64.finish_to(&mut |bytes| match inflate {
                Some(inf) => {
                    if inf.feed_to(bytes, &mut |b| store.put(b, &pool)).is_err() {
                        bad = true;
                    }
                }
                None => store.put(bytes, &pool),
            });
            base64.reset();
            if (r.is_err() || bad) && store.failed.is_none() {
                store.failed = Some(EINVAL);
            }
        }
        if up.cmd.more {
            return;
        }
        let up = self.upload.take().expect("an upload");
        self.finish_upload(*up);
    }

    pub(super) fn abandon_upload(&mut self) {
        // Its store gives the bytes back.
        self.upload = None;
    }

    /// The first chunk of a transmission (or a query).
    fn start_upload(&mut self, cmd: Command) {
        let query = cmd.action == Action::Query;
        let limit = self.graphics.pool().limit();
        // Which image (§3.3, step 1).
        let key = if query {
            None
        } else if cmd.image_id != 0 {
            let key = ImageKey::Kitty(cmd.image_id);
            // Replaced: its bytes go back before the new ones are counted.
            self.graphics.remove(key);
            Some(key)
        } else if cmd.image_number != 0 {
            Some(ImageKey::Kitty(self.graphics.free_kitty_id()))
        } else if cmd.action == Action::TransmitPlace {
            Some(self.graphics.new_key(ImageKey::Anonymous))
        } else {
            None
        };
        let mut store = Store {
            sink: Sink::Count(0),
            pool: None,
            pool_handle: self.graphics.pool().clone(),
            counted: 0,
            cap: usize::MAX,
            failed: None,
        };
        let fail = |store: &mut Store, f: Failure| {
            store.failed = Some(f);
            store.sink = Sink::Count(0);
        };
        let raw = cmd.format.bytes_per_pixel();
        if cmd.medium != KMedium::Direct {
            store.sink = Sink::Name(Vec::new());
        } else if let Some(bpp) = raw {
            let (w, h) = (cmd.data_width, cmd.data_height);
            if w == 0 || h == 0 {
                fail(&mut store, EINVAL);
            } else if w > MAX_SIDE || h > MAX_SIDE || w as usize * h as usize * 4 > limit {
                fail(&mut store, EFBIG);
            } else if query || key.is_none() {
                store.sink = Sink::Count(0);
                store.cap = w as usize * h as usize * bpp as usize;
            } else {
                match self.graphics.pool().reserve(self.terminal_id, pixel_bytes(w, h), None) {
                    Err(_) => fail(&mut store, EFBIG),
                    Ok(id) => {
                        store.pool = Some(id);
                        match self.image_host.alloc(w, h) {
                            Some(buf) => {
                                let layout = if bpp == 3 { Layout::Rgb } else { Layout::Rgba };
                                store.sink = Sink::Raw(buf, Converter::new(layout, w, h).display_p3());
                            }
                            None => fail(&mut store, EFBIG),
                        }
                    }
                }
            }
        } else {
            store.sink = Sink::Encoded(Encoded::new());
            if query {
                store.cap = HEAD;
            } else if key.is_some() {
                // One terminal's uploads hold at most a quarter of the budget.
                store.cap = limit / 4;
                match self.graphics.pool().reserve(self.terminal_id, 0, None) {
                    Ok(id) => store.pool = Some(id),
                    Err(_) => fail(&mut store, EFBIG),
                }
            } else {
                store.sink = Sink::Count(0);
            }
        }
        let inflate = cmd.compressed.then(|| {
            Inflate::new(match raw {
                Some(bpp) if cmd.medium == KMedium::Direct => cmd.data_width as usize * cmd.data_height as usize * bpp as usize,
                _ => store.cap.min(limit),
            })
        });
        self.upload = Some(Box::new(Upload {
            cmd,
            key,
            base64: Base64::new(),
            inflate,
            store,
        }));
    }

    fn finish_upload(&mut self, up: Upload) {
        let Upload {
            cmd,
            key,
            mut inflate,
            mut store,
            ..
        } = up;
        let reply_id = match key {
            Some(ImageKey::Kitty(id)) => id,
            _ => cmd.image_id,
        };
        let to = ReplyTo::of(&cmd, reply_id);
        let named = cmd.image_id != 0 || cmd.image_number != 0;
        if let Some(inf) = &mut inflate
            && store.failed.is_none()
            && inf.finish().is_err()
        {
            store.failed = Some(EINVAL);
        }
        if let Some(f) = store.failed {
            return self.reply(to, named, Err(f));
        }
        let query = cmd.action == Action::Query;
        match std::mem::replace(&mut store.sink, Sink::Count(0)) {
            Sink::Count(n) => {
                // A query (or an id-less `a=t`, which is dropped).
                let r = match cmd.format.bytes_per_pixel() {
                    Some(bpp) if n < cmd.data_width as usize * cmd.data_height as usize * bpp as usize => Err(ENODATA),
                    _ => Ok(()),
                };
                if query {
                    self.reply(to, named, r);
                }
            }
            Sink::Encoded(e) if query => {
                let head = e.pieces().next().unwrap_or(&[]);
                let r = match probe::probe(head) {
                    Ok(p) if p.width <= MAX_SIDE && p.height <= MAX_SIDE => Ok(()),
                    Ok(_) => Err(EFBIG),
                    Err(_) => Err(EBADPNG),
                };
                self.reply(to, named, r);
            }
            Sink::Name(name) => self.transmit_medium(cmd, key, name),
            Sink::Raw(buf, conv) => {
                let key = key.expect("raw uploads have a key");
                let id = store.pool.expect("raw uploads are counted");
                match conv.finish() {
                    Err(_) => self.reply(to, named, Err(ENODATA)),
                    Ok(done) => {
                        store.pool = None;
                        let pool = self.graphics.pool().clone();
                        pool.set_pixels(id, Arc::from(buf));
                        pool.set_busy(id, false);
                        let mut image = Image::new(key, cmd.data_width, cmd.data_height);
                        image.number = cmd.image_number;
                        image.opaque = done.opaque;
                        image.state = ImageState::Ready;
                        image.pool = Some(id);
                        self.add_image(image);
                        self.after_transmit(&cmd, key, to, named);
                    }
                }
            }
            Sink::Encoded(e) => {
                let key = key.expect("encoded uploads have a key");
                let id = store.pool.expect("encoded uploads are counted");
                let head = e.pieces().next().unwrap_or(&[]);
                let size = match probe::probe(head) {
                    Ok(p) if p.width <= MAX_SIDE && p.height <= MAX_SIDE => Ok((p.width, p.height)),
                    Ok(_) => Err(EFBIG),
                    Err(_) => Err(EBADPNG),
                };
                let size = size.and_then(|(w, h)| self.graphics.pool().grow(id, pixel_bytes(w, h)).map(|_| (w, h)).map_err(|_| EFBIG));
                match size {
                    Err(f) => self.reply(to, named, Err(f)),
                    Ok((w, h)) => {
                        store.pool = None;
                        let mut image = Image::new(key, w, h);
                        image.number = cmd.image_number;
                        image.pool = Some(id);
                        let generation = image.generation;
                        self.add_image(image);
                        self.image_jobs.push(ImageJob::Decode {
                            job: JobId::next(),
                            image: key,
                            generation,
                            bytes: Arc::new(e),
                            size: (w, h),
                        });
                        self.defer_reply(key, generation, to, named);
                        self.after_transmit(&cmd, key, to, false);
                    }
                }
            }
        }
    }

    /// `t=f`, `t=t`, `t=s`: open and check at once, read later (§3.4).
    fn transmit_medium(&mut self, cmd: Command, key: Option<ImageKey>, name: Vec<u8>) {
        let reply_id = match key {
            Some(ImageKey::Kitty(id)) => id,
            _ => cmd.image_id,
        };
        let to = ReplyTo::of(&cmd, reply_id);
        let named = cmd.image_id != 0 || cmd.image_number != 0;
        let medium = match cmd.medium {
            KMedium::File => Medium::File,
            KMedium::TempFile => Medium::TempFile,
            _ => Medium::SharedMemory,
        };
        let req = MediumRequest {
            medium,
            name,
            offset: cmd.file_offset,
            size: cmd.file_size,
        };
        let opened = match self.image_host.open_medium(&req) {
            Ok(m) if m.len as usize <= self.graphics.pool().limit() => m,
            Ok(_) => return self.reply(to, named, Err(EFBIG)),
            Err(_) => return self.reply(to, named, Err(EBADF)),
        };
        let query = cmd.action == Action::Query;
        if let Some(bpp) = cmd.format.bytes_per_pixel() {
            let (w, h) = (cmd.data_width, cmd.data_height);
            if w == 0 || h == 0 {
                return self.reply(to, named, Err(EINVAL));
            }
            if w > MAX_SIDE || h > MAX_SIDE || w as usize * h as usize * 4 > self.graphics.pool().limit() {
                return self.reply(to, named, Err(EFBIG));
            }
            if !cmd.compressed && (opened.len as usize) < w as usize * h as usize * bpp as usize {
                return self.reply(to, named, Err(ENODATA));
            }
            let Some(key) = key.filter(|_| !query) else {
                return self.reply(to, named, Ok(()));
            };
            let Ok(id) = self.graphics.pool().reserve(self.terminal_id, pixel_bytes(w, h), None) else {
                return self.reply(to, named, Err(EFBIG));
            };
            let mut image = Image::new(key, w, h);
            image.number = cmd.image_number;
            image.pool = Some(id);
            image.opaque = bpp == 3;
            let generation = image.generation;
            self.add_image(image);
            self.image_jobs.push(ImageJob::Read {
                job: JobId::next(),
                image: key,
                generation,
                medium: opened,
                then: AfterRead::Raw { has_alpha: bpp == 4 },
                size: (w, h),
                compressed: cmd.compressed,
            });
            self.defer_reply(key, generation, to, named);
            self.after_transmit(&cmd, key, to, false);
            return;
        }
        // PNG: its size before the next byte is parsed.
        let key = match key {
            Some(k) if !query => k,
            _ => {
                // A query (or nowhere to put it): look at the header now.
                let head = self.image_host.read_head(&opened, HEAD);
                return self.reply(to, named, head_size(&cmd, head).map(|_| ()));
            }
        };
        if self.reads_off_lock {
            self.parked = Some(Box::new(Parked {
                cmd,
                key,
                medium: Some(opened),
            }));
            return;
        }
        let head = self.image_host.read_head(&opened, HEAD);
        self.finish_media_png(cmd, key, opened, head);
    }

    fn finish_media_png(&mut self, cmd: Command, key: ImageKey, medium: OpenMedium, head: Result<Vec<u8>, ImageError>) {
        let to = ReplyTo::of(&cmd, if let ImageKey::Kitty(id) = key { id } else { 0 });
        let named = cmd.image_id != 0 || cmd.image_number != 0;
        let (w, h) = match head_size(&cmd, head) {
            Ok(s) => s,
            Err(f) => return self.reply(to, named, Err(f)),
        };
        // The pixels, and the encoded bytes the job reads (transient).
        let bytes = pixel_bytes(w, h) + medium.len as usize;
        let Ok(id) = self.graphics.pool().reserve(self.terminal_id, bytes, None) else {
            return self.reply(to, named, Err(EFBIG));
        };
        let mut image = Image::new(key, w, h);
        image.number = cmd.image_number;
        image.pool = Some(id);
        let generation = image.generation;
        self.add_image(image);
        self.image_jobs.push(ImageJob::Read {
            job: JobId::next(),
            image: key,
            generation,
            medium,
            then: AfterRead::Decode,
            size: (w, h),
            compressed: cmd.compressed,
        });
        self.defer_reply(key, generation, to, named);
        self.after_transmit(&cmd, key, to, false);
    }

    fn add_image(&mut self, image: Image) {
        self.graphics.make_room_for_image();
        self.graphics.insert(image);
    }

    /// After the image is stored: place it for `a=T`, and reply.
    fn after_transmit(&mut self, cmd: &Command, key: ImageKey, to: ReplyTo, reply_now: bool) {
        let placed = if cmd.action == Action::TransmitPlace {
            self.place_image(cmd, key)
        } else {
            Ok(())
        };
        if reply_now || placed.is_err() {
            self.reply(to, cmd.image_id != 0 || cmd.image_number != 0, placed);
        }
    }

    fn defer_reply(&mut self, key: ImageKey, generation: u32, to: ReplyTo, named: bool) {
        if named {
            self.deferred_replies.push((key, generation, to));
        }
    }

    /// A job finished: its deferred reply, if any.
    pub(super) fn job_reply(&mut self, key: ImageKey, generation: u32, ok: Result<(), ImageError>) {
        if let Some(i) = self.deferred_replies.iter().position(|(k, g, _)| *k == key && *g == generation) {
            let (_, _, to) = self.deferred_replies.swap_remove(i);
            self.reply(to, true, ok.map_err(|e| Failure::of(&e)));
        }
    }

    /// `a=p`.
    fn kitty_place(&mut self, cmd: &Command) {
        let named = cmd.image_id != 0 || cmd.image_number != 0;
        let id = if cmd.image_id != 0 {
            Some(cmd.image_id)
        } else if cmd.image_number != 0 {
            self.graphics.by_number(cmd.image_number)
        } else {
            None
        };
        let to = ReplyTo::of(cmd, id.unwrap_or(0));
        let r = match id {
            None => Err(ENOENT),
            Some(id) => self.place_image(cmd, ImageKey::Kitty(id)),
        };
        self.reply(to, named, r);
    }

    /// Place image `key` at the cursor (or virtually) as `cmd` says (§3.3).
    fn place_image(&mut self, cmd: &Command, key: ImageKey) -> Result<(), Failure> {
        self.graphics.apply_evictions();
        let (iw, ih, opaque) = match self.graphics.image(key) {
            Some(i) if i.state != ImageState::Lost && i.state != ImageState::Failed => (i.width, i.height, i.opaque),
            _ => return Err(ENOENT),
        };
        // Id-less images take no placement id.
        let pid = if matches!(key, ImageKey::Anonymous(_)) {
            0
        } else {
            cmd.placement_id
        };
        let [x, y, w, h] = cmd.src;
        let x = x.min(iw);
        let y = y.min(ih);
        let w = if w == 0 { iw - x } else { w.min(iw - x) };
        let h = if h == 0 { ih - y } else { h.min(ih - y) };
        let (cw, ch) = (self.cell_pixels.0.max(1) as u32, self.cell_pixels.1.max(1) as u32);
        let cols = if cmd.cols != 0 {
            cmd.cols
        } else {
            (w + cmd.cell_offset.0 as u32).div_ceil(cw).clamp(1, u16::MAX as u32) as u16
        };
        let rows = if cmd.rows != 0 {
            cmd.rows
        } else {
            (h + cmd.cell_offset.1 as u32).div_ceil(ch).clamp(1, u16::MAX as u32) as u16
        };
        if cmd.virtual_ {
            self.graphics.set_virtual(VirtualPlacement {
                image: key,
                id: pid,
                cols,
                rows,
                fit: Fit::Contain,
            });
            self.touch();
            return Ok(());
        }
        let line = self.top_line + self.cursor.row as u64;
        let col = self.cursor.col as u16;
        if matches!(key, ImageKey::Anonymous(_)) && opaque {
            // An opaque id-less frame over an earlier one replaces it (mpv).
            let (z, rows_c) = (cmd.z, rows.min(crate::image::MAX_PLACEMENT_ROWS));
            self.graphics.delete_placements(Some((line, line + rows_c as u64)), false, |p| {
                matches!(p.image, ImageKey::Anonymous(_))
                    && p.image != key
                    && p.z == z
                    && p.line >= line
                    && p.line + p.rows as u64 <= line + rows_c as u64
                    && p.col >= col
                    && p.col as u32 + p.cols as u32 <= col as u32 + cols as u32
            });
        }
        self.graphics.place(Placement {
            image: key,
            id: pid,
            line,
            col,
            cols,
            rows,
            offset: cmd.cell_offset,
            src: [x, y, w, h],
            z: cmd.z,
        });
        if !cmd.no_cursor_move {
            // Right by its columns, down by its rows − 1, scrolling as text.
            for _ in 1..rows.min(crate::image::MAX_PLACEMENT_ROWS) {
                let c = self.cursor.col;
                self.linefeed();
                self.cursor.col = c;
            }
            let to = self.cursor.col + cols as usize;
            if to >= self.cols {
                self.cursor.col = 0;
                self.linefeed();
            } else {
                self.cursor.col = to;
            }
            self.cursor.pending_wrap = false;
        }
        self.touch();
        Ok(())
    }

    /// `a=d` (§3.5).
    fn kitty_delete(&mut self, cmd: &Command) {
        let Some(d) = cmd.delete else { return };
        let free = d.free;
        let top = self.top_line;
        let screen = Some((top, top + self.lines as u64));
        let in_cell = |p: &Placement, line: u64, col: u64| {
            line >= p.line && line < p.line + p.rows as u64 && col >= p.col as u64 && col < p.col as u64 + p.cols as u64
        };
        let g = &mut self.graphics;
        match d.what {
            DeleteWhat::All => g.delete_placements(screen, free, |_| true),
            DeleteWhat::Id => {
                let id = cmd.image_id;
                g.delete_images(cmd.placement_id, free, |k| k == ImageKey::Kitty(id));
            }
            DeleteWhat::Number => {
                if let Some(id) = g.by_number(cmd.image_number) {
                    g.delete_images(cmd.placement_id, free, |k| k == ImageKey::Kitty(id));
                }
            }
            DeleteWhat::Range { from, to } => {
                g.delete_images(0, free, |k| matches!(k, ImageKey::Kitty(id) if (from..=to).contains(&id)));
            }
            DeleteWhat::Cursor => {
                let (line, col) = (top + self.cursor.row as u64, self.cursor.col as u64);
                g.delete_placements(screen, free, |p| in_cell(p, line, col));
            }
            DeleteWhat::Cell { x, y } => {
                let (line, col) = (top + (y as u64).saturating_sub(1), (x as u64).saturating_sub(1));
                g.delete_placements(screen, free, |p| in_cell(p, line, col));
            }
            DeleteWhat::CellZ { x, y, z } => {
                let (line, col) = (top + (y as u64).saturating_sub(1), (x as u64).saturating_sub(1));
                g.delete_placements(screen, free, |p| p.z == z && in_cell(p, line, col));
            }
            DeleteWhat::Column(x) => {
                let col = (x as u64).saturating_sub(1);
                g.delete_placements(screen, free, |p| col >= p.col as u64 && col < p.col as u64 + p.cols as u64);
            }
            DeleteWhat::Row(y) => {
                let line = top + (y as u64).saturating_sub(1);
                g.delete_placements(screen, free, |p| line >= p.line && line < p.line + p.rows as u64);
            }
            DeleteWhat::Z(z) => g.delete_placements(screen, free, |p| p.z == z),
            DeleteWhat::Frames => {}
        }
        self.touch();
    }

    /// Queue a reply (§3.6): none if the command named no image, `q=1`
    /// drops OK, `q=2` errors too.
    fn reply(&mut self, to: ReplyTo, named: bool, r: Result<(), Failure>) {
        if !named || !self.images_enabled() {
            return;
        }
        match r {
            Ok(()) if to.quiet >= 1 => return,
            Err(_) if to.quiet >= 2 => return,
            _ => {}
        }
        use std::fmt::Write;
        let mut s = String::from("\x1b_G");
        let _ = write!(s, "i={}", to.id);
        if to.number != 0 {
            let _ = write!(s, ",I={}", to.number);
        }
        if to.placement != 0 {
            let _ = write!(s, ",p={}", to.placement);
        }
        match r {
            Ok(()) => s.push_str(";OK"),
            Err(Failure(code, text)) => {
                let _ = write!(s, ";{code}:{text}");
            }
        }
        s.push_str("\x1b\\");
        self.replies.extend_from_slice(s.as_bytes());
    }
}

/// An encoded image's size from the first bytes of its medium.
fn head_size(cmd: &Command, head: Result<Vec<u8>, ImageError>) -> Result<(u32, u32), Failure> {
    let mut head = head.map_err(|_| EBADF)?;
    if cmd.compressed {
        let mut out = Vec::new();
        let _ = Inflate::new(HEAD).feed(&head, &mut out);
        head = out;
    }
    match probe::probe(&head) {
        Ok(p) if p.width <= MAX_SIDE && p.height <= MAX_SIDE => Ok((p.width, p.height)),
        Ok(_) => Err(EFBIG),
        Err(_) => Err(EBADPNG),
    }
}
