//! Programs handed to the new build on a restart, so they keep running
//! (docs/design/handoff.md §3).
//!
//! The old app makes an [`Offer`]: each pane with a program gets an id,
//! saved with its pane ([`LiveRef`]) in the window list the new build
//! reopens. Once the new build has connected, each pane's session is
//! handed over ([`crate::session::Session::hand_over`]) and sent. The
//! new build takes them all before it opens a window ([`take_all`]) and
//! each pane claims its own ([`claim`]); a pane with none, or whose
//! program wasn't handed over, starts a new shell as it always did.

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use term_core::Live;
use term_pty::Handed;
use term_pty::handoff::Item;

use crate::panes::PaneId;
use crate::reload::LiveRef;

/// How long the old app waits for the new build to connect, and each
/// side for the other's next step.
pub const TAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What goes with a pane's PTY: enough to carry on with it. Fields are
/// only ever added, each with a default, as in the saved window list.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Header {
    pub id: u64,
    pub pid: i32,
    #[serde(default)]
    pub tty_name: Option<String>,
    /// The terminal's size when its snapshot was taken.
    pub cols: usize,
    pub lines: usize,
    /// How much of the body is the snapshot's blocks; the rest is its
    /// replay ([`Live`]).
    pub blocks: usize,
    /// The token its shell's reports carry.
    #[serde(default)]
    pub token: Option<String>,
}

/// A pane's program as taken over.
#[derive(Debug)]
pub struct Taken {
    pub header: Header,
    pub handed: Handed,
    pub live: Live,
}

/// One pane's hand-over, for the socket.
pub fn item(header: &Header, handed: Handed, live: Live) -> Item {
    let mut header = header.clone();
    header.blocks = live.blocks.len();
    // What was handed over says whose it is, whatever the header held.
    header.pid = handed.pid;
    header.tty_name = handed.tty_name;
    let mut body = live.blocks;
    body.extend_from_slice(&live.replay);
    Item {
        fd: handed.master,
        header: serde_json::to_vec(&header).expect("a header serialises"),
        body,
    }
}

/// What [`item`] made, read back; `None` for one this build can't read.
pub fn taken(item: Item) -> Option<Taken> {
    let header: Header = serde_json::from_slice(&item.header).ok()?;
    if header.blocks > item.body.len() || header.pid <= 0 || header.cols == 0 || header.lines == 0 {
        return None;
    }
    let mut blocks = item.body;
    let replay = blocks.split_off(header.blocks);
    Some(Taken {
        handed: Handed {
            master: item.fd,
            pid: header.pid,
            tty_name: header.tty_name.clone(),
        },
        live: Live { blocks, replay },
        header,
    })
}

/// The socket of the app process `pid`:
/// `~/Library/Caches/terminal-decline/handoff-<pid>.sock`, its folder
/// made 0700.
pub fn socket_path(home: &Path, pid: u32) -> std::io::Result<PathBuf> {
    let dir = home.join("Library/Caches/terminal-decline");
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    Ok(dir.join(format!("handoff-{pid}.sock")))
}

/// The old app's side: the panes offered, by id, and the socket they
/// will be given on.
pub struct Offer {
    socket: PathBuf,
    listener: UnixListener,
    /// The window (its view's id) and pane of each id, in order.
    panes: RefCell<Vec<(usize, PaneId)>>,
}

impl Offer {
    /// Listen on `socket`.
    pub fn new(socket: PathBuf) -> std::io::Result<Offer> {
        let listener = term_pty::handoff::listen(&socket)?;
        Ok(Offer {
            socket,
            listener,
            panes: RefCell::default(),
        })
    }

    /// Offer pane `pane` of the window whose view is `window`: what to
    /// save with it.
    pub fn add(&self, window: usize, pane: PaneId) -> Option<LiveRef> {
        let mut panes = self.panes.borrow_mut();
        panes.push((window, pane));
        Some(LiveRef {
            socket: self.socket.to_str()?.to_string(),
            id: panes.len() as u64 - 1,
        })
    }

    /// The panes offered, with their ids.
    pub fn panes(&self) -> Vec<(u64, usize, PaneId)> {
        let panes = self.panes.borrow();
        panes.iter().enumerate().map(|(id, &(w, p))| (id as u64, w, p)).collect()
    }

    /// Wait for the new build to connect.
    pub fn accept(&self) -> std::io::Result<term_pty::handoff::Taker> {
        term_pty::handoff::accept(&self.listener, TAKE_TIMEOUT)
    }
}

impl Drop for Offer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// What this process took and no pane has claimed yet.
static TAKEN: Mutex<Option<HashMap<u64, Taken>>> = Mutex::new(None);

/// Take everything offered on `socket` (the new build, before it opens a
/// window): how many programs. An error means none: every pane starts a
/// new shell.
pub fn take_all(socket: &Path) -> std::io::Result<usize> {
    let items = term_pty::handoff::take(socket, TAKE_TIMEOUT)?;
    let taken: HashMap<u64, Taken> = items.into_iter().filter_map(taken).map(|t| (t.header.id, t)).collect();
    let n = taken.len();
    TAKEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .extend(taken);
    Ok(n)
}

/// The sockets the saved windows name, each once.
pub fn sockets(saved: &[crate::reload::SavedWindow]) -> Vec<PathBuf> {
    fn of_node(node: &crate::reload::SavedNode, out: &mut Vec<PathBuf>) {
        match node {
            crate::reload::SavedNode::Pane(p) => out.extend(p.live.as_ref().map(|l| PathBuf::from(&l.socket))),
            crate::reload::SavedNode::Split { a, b, .. } => {
                of_node(a, out);
                of_node(b, out);
            }
        }
    }
    let mut out = Vec::new();
    for w in saved {
        out.extend(w.live.as_ref().map(|l| PathBuf::from(&l.socket)));
        if let Some(panes) = &w.panes {
            of_node(&panes.root, &mut out);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The program with this id, for the pane it was saved with; once.
pub fn claim(id: u64) -> Option<Taken> {
    TAKEN.lock().unwrap_or_else(|e| e.into_inner()).as_mut()?.remove(&id)
}

/// Let go of what no pane claimed: their PTYs close and their programs
/// get the hang-up a closed pane gives. How many.
pub fn drop_unclaimed() -> usize {
    TAKEN.lock().unwrap_or_else(|e| e.into_inner()).take().map_or(0, |t| t.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::{FromRawFd, OwnedFd};

    fn fd() -> OwnedFd {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        unsafe {
            libc::close(fds[1]);
            OwnedFd::from_raw_fd(fds[0])
        }
    }

    fn header(id: u64) -> Header {
        Header {
            id,
            pid: 4242,
            tty_name: Some("/dev/ttys009".into()),
            cols: 80,
            lines: 24,
            blocks: 0,
            token: Some("abc".into()),
        }
    }

    #[test]
    fn an_item_reads_back_as_it_was_made() {
        let live = Live {
            blocks: b"blocks".to_vec(),
            replay: b"\x1b[?2004h".to_vec(),
        };
        let handed = Handed {
            master: fd(),
            pid: 4242,
            tty_name: Some("/dev/ttys009".into()),
        };
        let t = taken(item(&header(3), handed, live.clone())).expect("read back");
        assert_eq!(t.live, live);
        assert_eq!(t.header, Header { blocks: 6, ..header(3) });
        assert_eq!((t.handed.pid, t.handed.tty_name.as_deref()), (4242, Some("/dev/ttys009")));
    }

    #[test]
    fn an_item_this_build_cannot_read_is_left() {
        let bad = |header: &[u8], body: &[u8]| {
            taken(Item {
                fd: fd(),
                header: header.to_vec(),
                body: body.to_vec(),
            })
            .is_none()
        };
        assert!(bad(b"not json", b""));
        // More blocks than there is body; no program; no size.
        assert!(bad(br#"{"id":1,"pid":5,"cols":80,"lines":24,"blocks":9}"#, b"abc"));
        assert!(bad(br#"{"id":1,"pid":0,"cols":80,"lines":24,"blocks":0}"#, b""));
        assert!(bad(br#"{"id":1,"pid":5,"cols":0,"lines":24,"blocks":0}"#, b""));
        // A later build's header, with a field this one doesn't know.
        assert!(!bad(br#"{"id":1,"pid":5,"cols":80,"lines":24,"blocks":0,"later":true}"#, b"replay"));
    }

    #[test]
    fn the_sockets_of_a_window_list_are_each_named_once() {
        use crate::reload::{SavedNode, SavedPane, SavedPanes, SavedWindow};
        let live = |socket: &str, id| Some(LiveRef { socket: socket.into(), id });
        let window = |live, panes| SavedWindow {
            cwd: None,
            frame: [0.0; 4],
            tab_group: None,
            selected: false,
            panes,
            command: None,
            history: None,
            full_screen: false,
            live,
        };
        let pane = |live| {
            Box::new(SavedNode::Pane(SavedPane {
                live,
                ..SavedPane::default()
            }))
        };
        let split = SavedPanes {
            root: SavedNode::Split {
                dir: crate::panes::Dir::Right,
                ratio: 0.5,
                a: pane(live("/a.sock", 1)),
                b: pane(None),
            },
            focused: 0,
            zoomed: None,
        };
        let saved = [window(live("/a.sock", 0), None), window(None, Some(split)), window(None, None)];
        assert_eq!(sockets(&saved), [PathBuf::from("/a.sock")]);
        assert!(sockets(&saved[2..]).is_empty());
    }

    #[test]
    fn an_offer_numbers_its_panes_and_what_is_taken_is_claimed_once() {
        let dir = crate::testutil::TempDir::new("handoff-offer");
        // A short path: a socket's is at most 103 bytes.
        let socket = std::env::temp_dir().join(format!("td-offer-{}.sock", std::process::id()));
        let offer = Offer::new(socket.clone()).unwrap();
        let (a, b): (PaneId, PaneId) = (1, 2);
        assert_eq!(offer.add(7, a).unwrap().id, 0);
        let second = offer.add(7, b).unwrap();
        assert_eq!((second.id, second.socket.as_str()), (1, socket.to_str().unwrap()));
        assert_eq!(offer.panes(), [(0, 7, a), (1, 7, b)]);
        let taker = {
            let socket = socket.clone();
            std::thread::spawn(move || take_all(&socket).unwrap())
        };
        let live = Live {
            blocks: Vec::new(),
            replay: b"r".to_vec(),
        };
        let handed = |pid| Handed {
            master: fd(),
            pid,
            tty_name: None,
        };
        let items = [
            item(&Header { id: 900_001, ..header(0) }, handed(11), live.clone()),
            item(&Header { id: 900_002, ..header(0) }, handed(12), live.clone()),
        ];
        offer.accept().unwrap().give(&items, TAKE_TIMEOUT).unwrap();
        assert_eq!(taker.join().unwrap(), 2);
        assert_eq!(claim(900_001).map(|t| t.handed.pid), Some(11));
        assert!(claim(900_001).is_none());
        assert!(claim(5).is_none());
        // The socket goes with the offer.
        drop(offer);
        assert!(!socket.exists());
        let made = socket_path(dir.path(), 77).unwrap();
        assert!(made.ends_with("Library/Caches/terminal-decline/handoff-77.sock"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(made.parent().unwrap()).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
