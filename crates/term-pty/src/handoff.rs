//! Handing PTYs to another process over a Unix socket
//! (docs/design/handoff.md §2, §3): the old app serves what it has, the
//! new one takes it, and the programs carry on.
//!
//! One connection, served once. Each [`Item`] goes as a 12-byte frame
//! (the lengths of its header and body) sent with the descriptor in the
//! same message's ancillary data (`SCM_RIGHTS`), then the header and
//! the body. The taker answers [`TAKEN`] when it has everything, and
//! only then may the server let go of its own descriptors. Both ends
//! must be the same user (`getpeereid`).

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

const MAGIC: &[u8; 8] = b"TDHAND1\n";
const TAKEN: &[u8; 6] = b"taken\n";
/// More than any app has panes, and than any header or snapshot is big.
const MAX_ITEMS: u32 = 4096;
const MAX_HEADER: u32 = 1 << 20;
const MAX_BODY: u64 = 1 << 32;

/// One PTY with what its taker needs to carry on.
#[derive(Debug)]
pub struct Item {
    /// The PTY's master.
    pub fd: OwnedFd,
    /// What it is (the caller's own format).
    pub header: Vec<u8>,
    /// Its terminal's snapshot.
    pub body: Vec<u8>,
}

/// A socket at `path` (0600) for [`serve`]; a file left there is replaced.
pub fn listen(path: &Path) -> io::Result<UnixListener> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn same_user(stream: &UnixStream) -> io::Result<()> {
    let (mut uid, mut gid) = (0, 0);
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if uid != unsafe { libc::geteuid() } {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the other end is another user"));
    }
    Ok(())
}

fn timed_out(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("nobody {what} in time"))
}

/// Wait up to `timeout` for one taker and give it `items`. `Ok` once it
/// said it has them all; until then the caller keeps its descriptors.
pub fn serve(listener: &UnixListener, items: &[Item], timeout: Duration) -> io::Result<()> {
    accept(listener, timeout)?.give(items, timeout)
}

/// A taker that has connected, and is waiting for the items.
pub struct Taker {
    stream: UnixStream,
}

/// Wait up to `timeout` for the taker to connect. Nothing has been given
/// yet: a caller that never gets one has lost nothing.
pub fn accept(listener: &UnixListener, timeout: Duration) -> io::Result<Taker> {
    let deadline = Instant::now() + timeout;
    listener.set_nonblocking(true)?;
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(timed_out("took the programs"));
                }
                let mut p = libc::pollfd {
                    fd: listener.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                unsafe { libc::poll(&mut p, 1, left.as_millis().min(i32::MAX as u128) as i32) };
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    };
    stream.set_nonblocking(false)?;
    same_user(&stream)?;
    Ok(Taker { stream })
}

impl Taker {
    /// Give it `items`, each step within `timeout`. `Ok` once it said it
    /// has them all.
    pub fn give(self, items: &[Item], timeout: Duration) -> io::Result<()> {
        let mut stream = self.stream;
        stream.set_write_timeout(Some(timeout))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.write_all(MAGIC)?;
        stream.write_all(&(items.len() as u32).to_le_bytes())?;
        for item in items {
            let mut frame = [0u8; 12];
            frame[..4].copy_from_slice(&(item.header.len() as u32).to_le_bytes());
            frame[4..].copy_from_slice(&(item.body.len() as u64).to_le_bytes());
            send_with_fd(&stream, &frame, item.fd.as_raw_fd())?;
            stream.write_all(&item.header)?;
            stream.write_all(&item.body)?;
        }
        stream.flush()?;
        let mut answer = [0u8; TAKEN.len()];
        stream.read_exact(&mut answer)?;
        if &answer != TAKEN {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not taken"));
        }
        Ok(())
    }
}

/// Take everything the server at `path` has, each step within `timeout`.
pub fn take(path: &Path, timeout: Duration) -> io::Result<Vec<Item>> {
    let mut stream = UnixStream::connect(path)?;
    same_user(&stream)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut magic = [0u8; MAGIC.len()];
    stream.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a hand-off"));
    }
    let mut count = [0u8; 4];
    stream.read_exact(&mut count)?;
    let count = u32::from_le_bytes(count);
    if count > MAX_ITEMS {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many items"));
    }
    let mut items = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut frame = [0u8; 12];
        let fd = recv_with_fd(&mut stream, &mut frame)?;
        let header = u32::from_le_bytes(frame[..4].try_into().expect("4 bytes"));
        let body = u64::from_le_bytes(frame[4..].try_into().expect("8 bytes"));
        if header > MAX_HEADER || body > MAX_BODY {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "an item is too big"));
        }
        let mut item = Item {
            fd,
            header: vec![0; header as usize],
            body: vec![0; body as usize],
        };
        stream.read_exact(&mut item.header)?;
        stream.read_exact(&mut item.body)?;
        items.push(item);
    }
    stream.write_all(TAKEN)?;
    stream.flush()?;
    Ok(items)
}

/// Room for one descriptor's control message.
#[repr(C)]
struct Control {
    header: libc::cmsghdr,
    fd: RawFd,
}

/// `bytes`, whole, with `fd` in the message's ancillary data.
fn send_with_fd(stream: &UnixStream, bytes: &[u8], fd: RawFd) -> io::Result<()> {
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr() as *mut libc::c_void,
        iov_len: bytes.len(),
    };
    let mut control: Control = unsafe { std::mem::zeroed() };
    control.header.cmsg_len = unsafe { libc::CMSG_LEN(std::mem::size_of::<RawFd>() as u32) } as _;
    control.header.cmsg_level = libc::SOL_SOCKET;
    control.header.cmsg_type = libc::SCM_RIGHTS;
    control.fd = fd;
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = (&mut control as *mut Control).cast();
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as _;
    loop {
        let n = unsafe { libc::sendmsg(stream.as_raw_fd(), &msg, 0) };
        if n == bytes.len() as isize {
            return Ok(());
        }
        if n >= 0 {
            // A frame is 12 bytes into an empty buffer: never short.
            return Err(io::Error::new(io::ErrorKind::WriteZero, "a frame was cut short"));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Fill `bytes`, the first of which came with a descriptor.
fn recv_with_fd(stream: &mut UnixStream, bytes: &mut [u8]) -> io::Result<OwnedFd> {
    let mut iov = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut control: Control = unsafe { std::mem::zeroed() };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = (&mut control as *mut Control).cast();
    msg.msg_controllen = unsafe { libc::CMSG_SPACE(std::mem::size_of::<RawFd>() as u32) } as _;
    let n = loop {
        let n = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut msg, 0) };
        if n >= 0 {
            break n as usize;
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    };
    if n == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    let got = msg.msg_controllen as usize >= std::mem::size_of::<Control>()
        && msg.msg_flags & libc::MSG_CTRUNC == 0
        && control.header.cmsg_level == libc::SOL_SOCKET
        && control.header.cmsg_type == libc::SCM_RIGHTS
        && control.fd >= 0;
    if !got {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "a frame came without its descriptor"));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(control.fd) };
    unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    stream.read_exact(&mut bytes[n..])?;
    Ok(fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn socket_path() -> std::path::PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("td-ho-{}-{n}.sock", std::process::id()))
    }

    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
    }

    #[test]
    fn descriptors_arrive_with_their_headers_and_bodies() {
        let path = socket_path();
        let listener = listen(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        // Two pipes: what is written to the kept end is read from the
        // descriptor that crossed.
        let (read_a, write_a) = pipe();
        let (read_b, write_b) = pipe();
        let items = vec![
            Item {
                fd: read_a,
                header: b"{\"id\":1}".to_vec(),
                body: vec![7; 300_000],
            },
            Item {
                fd: read_b,
                header: Vec::new(),
                body: Vec::new(),
            },
        ];
        let taker = {
            let path = path.clone();
            std::thread::spawn(move || take(&path, Duration::from_secs(10)).unwrap())
        };
        serve(&listener, &items, Duration::from_secs(10)).unwrap();
        let taken = taker.join().unwrap();
        assert_eq!(taken.len(), 2);
        assert_eq!((taken[0].header.as_slice(), taken[0].body.len()), (&b"{\"id\":1}"[..], 300_000));
        assert!(taken[0].body.iter().all(|&b| b == 7));
        assert!(taken[1].header.is_empty() && taken[1].body.is_empty());
        for (write, item, text) in [(write_a, &taken[0], b"aa"), (write_b, &taken[1], b"bb")] {
            assert_ne!(unsafe { libc::fcntl(item.fd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
            std::fs::File::from(write).write_all(text).unwrap();
            let mut got = [0u8; 2];
            std::fs::File::from(item.fd.try_clone().unwrap()).read_exact(&mut got).unwrap();
            assert_eq!(&got, text);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nobody_taking_is_an_error_in_time() {
        let path = socket_path();
        let listener = listen(&path).unwrap();
        let started = Instant::now();
        let e = serve(&listener, &[], Duration::from_millis(200)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
        // And no socket: the new build falls back to starting shells.
        let _ = std::fs::remove_file(&path);
        assert!(take(&path, Duration::from_millis(200)).is_err());
    }

    #[test]
    fn a_taker_that_goes_away_is_not_taken() {
        let path = socket_path();
        let listener = listen(&path).unwrap();
        let (read, _write) = pipe();
        let items = vec![Item {
            fd: read,
            header: b"h".to_vec(),
            body: b"b".to_vec(),
        }];
        let taker = {
            let path = path.clone();
            // Connects, then hangs up without reading.
            std::thread::spawn(move || drop(UnixStream::connect(&path).unwrap()))
        };
        let result = serve(&listener, &items, Duration::from_secs(5));
        taker.join().unwrap();
        assert!(result.is_err());
        let _ = std::fs::remove_file(&path);
    }
}
