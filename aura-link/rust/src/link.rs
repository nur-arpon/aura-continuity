//! The Aura Link runtime: ONE session per paired device, carried by whatever pipe is up.
//!
//! * Either side may connect (the PC to the phone's / watch's listener, or the device to the PC's).
//!   If both connect at the same moment (within 5 s) the session the PC opened wins on both sides;
//!   otherwise the newest authenticated session replaces the old one (the old one may be dead).
//! * One driver thread per link does every write, in priority order: P0 (lock, unlock, acks,
//!   errors), P1 (control, text, url, clipboard, offers), P2 (file chunks, produced one at a time
//!   so a lock message waits at most one chunk).
//! * Acked messages stay queued until ACK (or ERROR) arrives; after a dropped session they are
//!   resent on the next one; receivers drop duplicate ids. No ACK within `ack_timeout` = the
//!   session is treated as dead and rebuilt.
//! * Reconnect only while something is waiting, with exponential backoff (0.5 s .. 60 s);
//!   messages expire after `msg_ttl`. With nothing queued the driver blocks on a condvar:
//!   no timers, no polling. An open session closes itself after `idle_close` of silence.
//! * Big files ask for the LAN fast path (fast.rs) and fall back to Bluetooth.

use crate::aead::{Opener, Sealer, MAX_PLAINTEXT};
use crate::fast::{self, FastConn, FastListener};
use crate::file::{self, FileOffer, Inbox, RecvJob, SendJob};
use crate::handshake::{self, caps, role, Credential, Ephemeral, Established, Hello};
use crate::msg::{kind, priority, FastOffer, Message};
use crate::wire::{read_record, write_record};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub type Closer = Arc<dyn Fn() + Send + Sync>;

/// One byte pipe (RFCOMM socket, TCP socket, in-memory pipe in tests).
pub struct Conn {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub close: Closer,
    pub label: String,
}

impl Conn {
    pub fn tcp(s: TcpStream, label: &str) -> std::io::Result<Self> {
        s.set_nodelay(true).ok();
        let r = s.try_clone()?;
        let c = s.try_clone()?;
        Ok(Self { reader: Box::new(r), writer: Box::new(s), close: Arc::new(move || { let _ = c.shutdown(std::net::Shutdown::Both); }), label: label.into() })
    }
}

pub trait Connector: Send + Sync {
    /// Open a fresh pipe to the peer (blocking; may take a few seconds on Bluetooth).
    fn connect(&self) -> Result<Conn, String>;
}

pub struct FnConnector(pub Box<dyn Fn() -> Result<Conn, String> + Send + Sync>);

impl Connector for FnConnector {
    fn connect(&self) -> Result<Conn, String> {
        (self.0)()
    }
}

/// "Happy Eyeballs" for pipes (item 17, Aura Anywhere): start `first` (Bluetooth), and if it has
/// not produced a pipe after `head_start`, start `second` too (the tailnet TCP path, when one is
/// offered). The first pipe that opens wins; a pipe that opens later is closed at once. `second`
/// returning None = no tailnet path right now (it is then not waited for). Errors of both are
/// joined. Bluetooth keeps priority at home (it normally connects well inside the head start);
/// when the PC/phone is out of radio range the tailnet path takes over after `head_start`.
pub fn race_connect(
    first: Box<dyn FnOnce() -> Result<Conn, String> + Send>,
    second: Box<dyn FnOnce() -> Option<Result<Conn, String>> + Send>,
    head_start: Duration,
) -> Result<Conn, String> {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel::<(u8, Option<Result<Conn, String>>)>();
    let winner = Arc::new(Mutex::new(false));
    let spawn = |n: u8, f: Box<dyn FnOnce() -> Option<Result<Conn, String>> + Send>, tx: mpsc::Sender<(u8, Option<Result<Conn, String>>)>, winner: Arc<Mutex<bool>>| {
        std::thread::Builder::new()
            .name(format!("aura-race-{n}"))
            .spawn(move || {
                let r = f();
                let mut w = winner.lock().unwrap_or_else(|p| p.into_inner());
                if *w {
                    // somebody already won: a late pipe is closed, never leaked
                    if let Some(Ok(c)) = r {
                        (c.close)();
                    }
                    return;
                }
                if matches!(r, Some(Ok(_))) {
                    *w = true;
                }
                drop(w);
                let _ = tx.send((n, r));
            })
            .is_ok()
    };
    spawn(1, Box::new(move || Some(first())), tx.clone(), winner.clone());
    let mut errs: Vec<String> = Vec::new();
    let mut pending = 1;
    let mut second = Some(second);
    let deadline = Instant::now() + head_start;
    loop {
        let got = if second.is_some() {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(x) => Some(x),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else if pending > 0 {
            match rx.recv() {
                Ok(x) => Some(x),
                Err(_) => break,
            }
        } else {
            break;
        };
        match got {
            // head start over (or the first path already failed): start the second path
            None => {
                if spawn(2, second.take().unwrap(), tx.clone(), winner.clone()) {
                    pending += 1;
                }
            }
            Some((n, r)) => {
                pending -= 1;
                match r {
                    Some(Ok(c)) => return Ok(c),
                    Some(Err(e)) => errs.push(format!("{}: {e}", if n == 1 { "bluetooth" } else { "tailnet" })),
                    None => {}
                }
                if n == 1 && second.is_some() {
                    if spawn(2, second.take().unwrap(), tx.clone(), winner.clone()) {
                        pending += 1;
                    }
                }
            }
        }
    }
    Err(if errs.is_empty() { "no path to the peer".into() } else { errs.join("; ") })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FastRole {
    None,
    /// opens a LAN listener (the phone)
    Server,
    /// connects out (the PC)
    Client,
}

/// What the application sees. Every method has a no-op default.
pub trait Handler: Send + Sync {
    /// A message the link does not handle itself (LOCK, UNLOCK_REQUEST, TEXT, URL, ...).
    /// For acked messages, `Ok` sends ACK and `Err(reason)` sends ERROR(id, reason) instead.
    fn on_message(&self, _link: &Link, _m: &Message) -> Result<(), String> {
        Ok(())
    }
    fn on_file(&self, _link: &Link, _path: &Path, _offer: &FileOffer) {}
    fn on_session(&self, _link: &Link, _up: bool) {}
    fn log(&self, _line: &str) {}
}

pub struct Config {
    pub my_role: u8,
    pub peer_role: u8,
    pub pc_id: [u8; 8],
    pub caps: u32,
    pub name: String,
    pub cred: Credential,
    pub inbox: PathBuf,
    pub idle_close: Duration,
    /// After a user-facing message (sent or received) the session stays open at least this long.
    pub warm_hold: Duration,
    pub ack_timeout: Duration,
    pub handshake_timeout: Duration,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    pub msg_ttl: Duration,
    pub chunk_bt: usize,
    pub chunk_fast: usize,
    pub fast_role: FastRole,
    pub fast_min_size: u64,
    pub fast_wait: Duration,
    pub fast_addrs: Arc<dyn Fn() -> Vec<Ipv4Addr> + Send + Sync>,
}

impl Config {
    pub fn new(my_role: u8, peer_role: u8, pc_id: [u8; 8], cred: Credential, inbox: PathBuf) -> Self {
        Self {
            my_role,
            peer_role,
            pc_id,
            caps: caps::FILES,
            name: String::new(),
            cred,
            inbox,
            idle_close: Duration::from_secs(30),
            warm_hold: Duration::from_secs(180),
            ack_timeout: Duration::from_secs(15),
            handshake_timeout: Duration::from_secs(10),
            backoff_min: Duration::from_millis(500),
            backoff_max: Duration::from_secs(60),
            msg_ttl: Duration::from_secs(300),
            chunk_bt: 16 * 1024,
            chunk_fast: 60 * 1024,
            fast_role: FastRole::None,
            fast_min_size: 256 * 1024,
            fast_wait: Duration::from_secs(4),
            fast_addrs: Arc::new(fast::local_ipv4),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileReport {
    pub bytes: u64,
    /// first chunk -> FILE_DONE
    pub elapsed: Duration,
    pub bytes_fast: u64,
    pub bytes_bt: u64,
}

impl FileReport {
    pub fn mbit_s(&self) -> f64 {
        self.bytes as f64 * 8.0 / 1e6 / self.elapsed.as_secs_f64().max(1e-6)
    }
}

struct Out {
    m: Message,
    deadline: Instant,
    sent_at: Option<Instant>,
}

struct Pipe {
    w: Box<dyn Write + Send>,
    sealer: Sealer,
}

struct SessionRef {
    gen: u64,
    pipe: Arc<Mutex<Pipe>>,
    close: Closer,
    started: Instant,
    pc_initiated: bool,
    peer: Hello,
    k_fast: [u8; 32],
    label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FastState {
    Idle,
    Negotiating(Instant),
    Up,
    Failed,
}

struct State {
    queue: [VecDeque<Out>; 3],
    unacked: Vec<Out>,
    outcomes: VecDeque<(u32, Result<(), String>)>,
    session: Option<SessionRef>,
    gen: u64,
    next_id: u32,
    seen: VecDeque<u32>,
    sends: Vec<SendJob>,
    recvs: HashMap<[u8; 16], RecvJob>,
    done_recv: VecDeque<([u8; 16], u8)>,
    fast: Option<(Arc<Mutex<Pipe>>, Closer)>,
    fast_state: FastState,
    fast_last_use: Instant,
    backoff: Duration,
    next_try: Option<Instant>,
    last_activity: Instant,
    warm_until: Instant,
    stopped: bool,
    sessions: u64,
}

struct Inner {
    cfg: Config,
    connector: Option<Arc<dyn Connector>>,
    handler: Arc<dyn Handler>,
    st: Mutex<State>,
    cv: Condvar,
}

/// Handle to one device's link. Cheap to clone.
#[derive(Clone)]
pub struct Link(Arc<Inner>);

enum Job {
    Write(Arc<Mutex<Pipe>>, Message, u64, bool),
    Connect,
    Wait(Option<Duration>),
    Teardown(u64, String),
    OpenFastServer(u64, [u8; 32]),
    CloseFast,
    Exit,
}

impl Link {
    pub fn new(cfg: Config, connector: Option<Arc<dyn Connector>>, handler: Arc<dyn Handler>) -> Link {
        let now = Instant::now();
        let st = State {
            queue: Default::default(),
            unacked: Vec::new(),
            outcomes: VecDeque::new(),
            session: None,
            gen: 0,
            next_id: u32::from_be_bytes(crate::random()) | 1,
            seen: VecDeque::new(),
            sends: Vec::new(),
            recvs: HashMap::new(),
            done_recv: VecDeque::new(),
            fast: None,
            fast_state: FastState::Idle,
            fast_last_use: now,
            backoff: cfg.backoff_min,
            next_try: None,
            last_activity: now,
            warm_until: now,
            stopped: false,
            sessions: 0,
        };
        let link = Link(Arc::new(Inner { cfg, connector, handler, st: Mutex::new(st), cv: Condvar::new() }));
        let l2 = link.clone();
        std::thread::Builder::new().name(format!("aura-link-{}", role::name(link.0.cfg.peer_role))).spawn(move || l2.driver()).expect("spawn");
        link
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.0.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn log(&self, s: &str) {
        self.0.handler.log(&format!("[{}] {s}", role::name(self.0.cfg.peer_role)));
    }

    pub fn config(&self) -> &Config {
        &self.0.cfg
    }

    // ------------------------------------------------------------------ public API

    /// Queue a message (any priority); connects if needed. Returns its id.
    pub fn send(&self, mut m: Message) -> u32 {
        let mut st = self.st();
        m.id = st.next_id;
        st.next_id = st.next_id.wrapping_add(1).max(1);
        let ttl = if m.kind == kind::LOCK || m.kind == kind::UNLOCK_REQUEST { Duration::from_secs(110) } else { Duration::from_secs(crate::msg::ttl_secs(m.kind, self.0.cfg.msg_ttl.as_secs())).min(self.0.cfg.msg_ttl) };
        let id = m.id;
        if crate::msg::is_user_facing(m.kind) {
            st.warm_until = st.warm_until.max(Instant::now() + self.0.cfg.warm_hold);
        }
        st.queue[priority(m.kind)].push_back(Out { m, deadline: Instant::now() + ttl, sent_at: None });
        self.0.cv.notify_all();
        id
    }

    /// Offer a file. `prefer_fast`: Some(true) = fast path even if small, Some(false) = Bluetooth
    /// only, None = fast path for files >= `fast_min_size`.
    pub fn send_file(&self, path: &Path, prefer_fast: Option<bool>) -> Result<[u8; 16], String> {
        let job = SendJob::new(path, prefer_fast).map_err(|e| format!("{}: {e}", path.display()))?;
        let id = job.offer.id;
        let offer = job.offer.encode();
        let mut st = self.st();
        st.sends.push(job);
        st.queue[1].push_back(Out { m: Message::new(kind::FILE_OFFER, offer), deadline: Instant::now() + Duration::from_secs(3600), sent_at: None });
        self.0.cv.notify_all();
        Ok(id)
    }

    /// Block until the file was confirmed (FILE_DONE ok) or failed.
    pub fn wait_file(&self, id: &[u8; 16], timeout: Duration) -> Result<FileReport, String> {
        let end = Instant::now() + timeout;
        let mut st = self.st();
        loop {
            if let Some(j) = st.sends.iter().find(|j| &j.offer.id == id) {
                if let Some(r) = &j.done {
                    let r = r.clone().map(|_| FileReport {
                        bytes: j.offer.size,
                        elapsed: j.finished.unwrap_or_else(Instant::now).saturating_duration_since(j.first_chunk.unwrap_or(j.created)),
                        bytes_fast: j.bytes_fast,
                        bytes_bt: j.bytes_bt,
                    });
                    st.sends.retain(|j| &j.offer.id != id);
                    return r;
                }
            } else {
                return Err("unknown file".into());
            }
            let now = Instant::now();
            if now >= end {
                return Err("timed out".into());
            }
            st = self.0.cv.wait_timeout(st, end - now).unwrap_or_else(|p| p.into_inner()).0;
        }
    }

    /// Withdraw a queued message that was never written to a session. true = it will not be
    /// delivered (the caller may queue it elsewhere); false = already on the wire (or unknown).
    pub fn cancel(&self, id: u32) -> bool {
        let mut st = self.st();
        for q in st.queue.iter_mut() {
            if let Some(i) = q.iter().position(|o| o.m.id == id && o.sent_at.is_none()) {
                q.remove(i);
                return true;
            }
        }
        false
    }

    /// true while message `id` is still waiting in the queue (not written yet).
    pub fn queued(&self, id: u32) -> bool {
        self.st().queue.iter().any(|q| q.iter().any(|o| o.m.id == id))
    }

    /// Withdraw a file whose offer was not accepted yet. true = withdrawn.
    pub fn cancel_file(&self, id: &[u8; 16]) -> bool {
        let mut st = self.st();
        let Some(j) = st.sends.iter().position(|j| &j.offer.id == id && !j.accepted && j.done.is_none()) else { return false };
        st.sends.remove(j);
        st.queue[1].retain(|o| !(o.m.kind == kind::FILE_OFFER && o.m.body.starts_with(id)));
        true
    }

    /// Block until message `id` was ACKed (Ok) or answered with ERROR (Err).
    pub fn wait_acked(&self, id: u32, timeout: Duration) -> Result<(), String> {
        let end = Instant::now() + timeout;
        let mut st = self.st();
        loop {
            if let Some((_, r)) = st.outcomes.iter().find(|(i, _)| *i == id) {
                return r.clone();
            }
            let now = Instant::now();
            if now >= end {
                return Err("no ack (timed out)".into());
            }
            st = self.0.cv.wait_timeout(st, end - now).unwrap_or_else(|p| p.into_inner()).0;
        }
    }

    /// Keep the session open for at least `d` from now (e.g. while a chat head is visible).
    pub fn keep_warm(&self, d: Duration) {
        let mut st = self.st();
        st.warm_until = st.warm_until.max(Instant::now() + d);
        self.0.cv.notify_all();
    }

    pub fn connected(&self) -> bool {
        self.st().session.is_some()
    }

    pub fn fast_up(&self) -> bool {
        self.st().fast_state == FastState::Up
    }

    /// How the current session is carried ("rfcomm", "tailnet 100.x.y.z", ...); None = no session.
    pub fn session_label(&self) -> Option<String> {
        self.st().session.as_ref().map(|s| s.label.clone())
    }

    pub fn peer(&self) -> Option<Hello> {
        self.st().session.as_ref().map(|s| s.peer.clone())
    }

    pub fn status(&self) -> String {
        let st = self.st();
        format!(
            "{}: session={} fast={:?} queued={}/{}/{} unacked={} files out={} in={} sessions={}",
            role::name(self.0.cfg.peer_role),
            st.session.as_ref().map(|s| format!("{}#{}", s.label, s.gen)).unwrap_or_else(|| "none".into()),
            st.fast_state,
            st.queue[0].len(),
            st.queue[1].len(),
            st.queue[2].len(),
            st.unacked.len(),
            st.sends.len(),
            st.recvs.len(),
            st.sessions
        )
    }

    /// Drop the current session (it reconnects when something is queued).
    pub fn close(&self) {
        let g = self.st().session.as_ref().map(|s| s.gen);
        if let Some(g) = g {
            self.teardown(g, "closed by the application");
        }
    }

    pub fn stop(&self) {
        self.close();
        self.st().stopped = true;
        self.0.cv.notify_all();
    }

    /// Our HELLO, a fresh ephemeral key and the credential, for a listener that answers an
    /// incoming connection (see [respond_and_attach]).
    pub fn responder_parts(&self) -> (Hello, Ephemeral, Credential) {
        let e = Ephemeral::generate();
        let c = &self.0.cfg;
        (Hello::new(c.my_role, c.pc_id, &e, c.caps, &c.name), e, c.cred.clone())
    }

    /// Attach an incoming, already authenticated connection.
    pub fn accept(&self, conn: Conn, est: Established) -> bool {
        self.install(conn, est)
    }

    // ------------------------------------------------------------------ sessions

    fn install(&self, conn: Conn, est: Established) -> bool {
        let pc_initiated = (est.initiator && self.0.cfg.my_role == role::PC) || (!est.initiator && est.peer.role == role::PC);
        let mut st = self.st();
        if let Some(cur) = &st.session {
            if cur.started.elapsed() < Duration::from_secs(5) && cur.pc_initiated && !pc_initiated {
                drop(st);
                (conn.close)();
                self.log("simultaneous connect: kept the PC-initiated session");
                return false;
            }
            let g = cur.gen;
            drop(st);
            self.teardown(g, "replaced by a newer session");
            st = self.st();
        }
        st.gen += 1;
        st.sessions += 1;
        let gen = st.gen;
        let Conn { reader, writer, close, label } = conn;
        let pipe = Arc::new(Mutex::new(Pipe { w: writer, sealer: est.sealer() }));
        st.session = Some(SessionRef { gen, pipe, close, started: Instant::now(), pc_initiated, peer: est.peer.clone(), k_fast: est.keys.fast, label: label.clone() });
        st.fast_state = FastState::Idle;
        st.backoff = self.0.cfg.backoff_min;
        st.next_try = None;
        st.last_activity = Instant::now();
        // unfinished outgoing files: offer again, the receiver answers with its offset
        let offers: Vec<Vec<u8>> = st.sends.iter_mut().filter(|j| j.done.is_none()).map(|j| { j.accepted = false; j.offer.encode() }).collect();
        for o in offers {
            if !st.queue[1].iter().any(|q| q.m.kind == kind::FILE_OFFER && q.m.body == o) {
                st.queue[1].push_back(Out { m: Message::new(kind::FILE_OFFER, o), deadline: Instant::now() + Duration::from_secs(3600), sent_at: None });
            }
        }
        drop(st);
        self.log(&format!("session #{gen} up over {label} ({} {}, v{}, peer caps {:#x})", if est.initiator { "to" } else { "from" }, est.peer.name, est.version, est.peer.caps));
        let me = self.clone();
        let opener = est.opener();
        std::thread::Builder::new().name("aura-link-read".into()).spawn(move || me.reader(reader, opener, gen, false)).expect("spawn");
        self.0.cv.notify_all();
        self.0.handler.on_session(self, true);
        true
    }

    fn teardown(&self, gen: u64, why: &str) {
        let mut st = self.st();
        let Some(s) = st.session.take_if(|s| s.gen == gen) else { return };
        (s.close)();
        if let Some((_, c)) = st.fast.take() {
            c();
        }
        st.fast_state = FastState::Idle;
        // sent-but-unacked messages go back to the front of their queues, oldest first
        let mut back: Vec<Out> = std::mem::take(&mut st.unacked);
        back.sort_by_key(|o| o.m.id);
        for mut o in back.into_iter().rev() {
            o.sent_at = None;
            let p = priority(o.m.kind);
            st.queue[p].push_front(o);
        }
        // queued chunks are useless without their session (they are re-read from the file)
        st.queue[2].clear();
        for j in st.sends.iter_mut() {
            j.accepted = false;
        }
        drop(st);
        self.log(&format!("session #{gen} down: {why}"));
        self.0.cv.notify_all();
        self.0.handler.on_session(self, false);
    }

    fn reader(&self, mut r: Box<dyn Read + Send>, mut opener: Opener, gen: u64, fast: bool) {
        let why = loop {
            let rec = match read_record(&mut r) {
                Ok(x) => x,
                Err(e) => break format!("read: {e}"),
            };
            let pt = match opener.open(&rec) {
                Ok(x) => x,
                Err(e) => break e,
            };
            match Message::decode(&pt) {
                Ok(m) => self.incoming(gen, m, fast),
                Err(e) => break format!("bad message: {e}"),
            }
        };
        if fast {
            let mut st = self.st();
            if st.session.as_ref().map(|s| s.gen) == Some(gen) && st.fast_state == FastState::Up {
                if let Some((_, c)) = st.fast.take() {
                    c();
                }
                st.fast_state = FastState::Idle;
                drop(st);
                self.log(&format!("fast path closed ({why})"));
                self.0.cv.notify_all();
            }
        } else {
            self.teardown(gen, &why);
        }
    }

    // ------------------------------------------------------------------ incoming

    fn incoming(&self, gen: u64, m: Message, via_fast: bool) {
        let mut st = self.st();
        if st.session.as_ref().map(|s| s.gen) != Some(gen) {
            return;
        }
        st.last_activity = Instant::now();
        if crate::msg::is_user_facing(m.kind) {
            st.warm_until = st.warm_until.max(Instant::now() + self.0.cfg.warm_hold);
        }
        if via_fast {
            st.fast_last_use = Instant::now();
        }
        let dup = m.wants_ack() && st.seen.contains(&m.id);
        if dup {
            st.queue[0].push_back(Out { m: Message::ack(m.id), deadline: Instant::now() + Duration::from_secs(60), sent_at: None });
            self.0.cv.notify_all();
            return;
        }
        if m.wants_ack() {
            st.seen.push_back(m.id);
            while st.seen.len() > 1024 {
                st.seen.pop_front();
            }
        }
        let mut deliver = false;
        let mut finished: Option<(PathBuf, FileOffer)> = None;
        match m.kind {
            kind::ACK | kind::ERROR => {
                let mut c = crate::wire::Cur::new(&m.body);
                if let Ok(id) = c.u32() {
                    let res = if m.kind == kind::ACK { Ok(()) } else { Err(String::from_utf8_lossy(c.rest()).into_owned()) };
                    if let Err(e) = &res {
                        self.log(&format!("peer refused message {id}: {e}"));
                    }
                    st.unacked.retain(|o| o.m.id != id);
                    st.outcomes.push_back((id, res));
                    while st.outcomes.len() > 256 {
                        st.outcomes.pop_front();
                    }
                }
            }
            kind::FILE_OFFER => match FileOffer::decode(&m.body) {
                Ok(o) => {
                    if let Some((_, s)) = st.done_recv.iter().find(|(i, _)| *i == o.id) {
                        let b = file::done_body(&o.id, *s);
                        st.queue[1].push_back(out(Message::new(kind::FILE_DONE, b)));
                    } else {
                        let inbox = Inbox { dir: self.0.cfg.inbox.clone() };
                        let off = match st.recvs.get(&o.id) {
                            Some(j) => Ok(j.received),
                            None => inbox.open(&o).map(|j| {
                                let r = j.received;
                                st.recvs.insert(o.id, j);
                                r
                            }),
                        };
                        match off {
                            Ok(off) => {
                                self.log(&format!("file offer {} ({} bytes), resuming at {off}", o.name, o.size));
                                if o.size == 0 {
                                    let j = st.recvs.remove(&o.id).unwrap();
                                    finished = self.complete(&mut st, j);
                                } else {
                                    st.queue[1].push_back(out(Message::new(kind::FILE_ACCEPT, file::id_offset_body(&o.id, off))));
                                }
                            }
                            Err(e) => {
                                self.log(&format!("file offer refused: {e}"));
                                st.queue[1].push_back(out(Message::new(kind::FILE_DONE, file::done_body(&o.id, file::DONE_REFUSED))));
                            }
                        }
                    }
                }
                Err(e) => self.log(&format!("bad FILE_OFFER: {e}")),
            },
            kind::FILE_ACCEPT => {
                if let Ok((id, off, _)) = file::parse_id_offset(&m.body) {
                    if let Some(j) = st.sends.iter_mut().find(|j| j.offer.id == id && j.done.is_none()) {
                        j.next = off.min(j.offer.size);
                        j.accepted = true;
                    }
                }
            }
            kind::FILE_CHUNK => {
                if let Ok((id, off, data)) = file::parse_id_offset(&m.body) {
                    let inbox = Inbox { dir: self.0.cfg.inbox.clone() };
                    if let Some(mut j) = st.recvs.remove(&id) {
                        if off < j.received {
                            st.recvs.insert(id, j); // duplicate after a resync: ignore
                        } else {
                            match inbox.write(&mut j, off, data) {
                                Ok(true) => finished = self.complete(&mut st, j),
                                Ok(false) => {
                                    st.recvs.insert(id, j);
                                }
                                Err(e) => {
                                    // gap (a fast-path socket died mid-file): ask the sender to rewind
                                    let r = j.received;
                                    st.recvs.insert(id, j);
                                    if !st.queue[1].iter().any(|q| q.m.kind == kind::FILE_ACCEPT && q.m.body == file::id_offset_body(&id, r)) {
                                        self.log(&format!("file resync: {e}"));
                                        st.queue[1].push_back(out(Message::new(kind::FILE_ACCEPT, file::id_offset_body(&id, r))));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            kind::FILE_DONE => {
                if let Ok((id, status)) = file::parse_done(&m.body) {
                    let mut reoffer = None;
                    if let Some(j) = st.sends.iter_mut().find(|j| j.offer.id == id && j.done.is_none()) {
                        match status {
                            file::DONE_OK => {
                                j.done = Some(Ok(()));
                                j.finished = Some(Instant::now());
                            }
                            file::DONE_BAD_HASH => {
                                j.next = 0;
                                j.accepted = false;
                                reoffer = Some(j.offer.encode());
                            }
                            _ => j.done = Some(Err("peer refused the file".into())),
                        }
                    }
                    if let Some(o) = reoffer {
                        self.log("file hash mismatch on the peer: sending again from 0");
                        st.queue[1].push_back(out(Message::new(kind::FILE_OFFER, o)));
                    }
                }
            }
            kind::FILE_CANCEL => {
                if m.body.len() >= 16 {
                    let id: [u8; 16] = m.body[..16].try_into().unwrap();
                    if let Some(j) = st.recvs.remove(&id) {
                        let _ = std::fs::remove_file(&j.part);
                    }
                }
            }
            kind::FAST_REQ => {
                if self.0.cfg.fast_role == FastRole::Server && matches!(st.fast_state, FastState::Idle | FastState::Failed) {
                    st.fast_state = FastState::Negotiating(Instant::now() + self.0.cfg.fast_wait);
                    let k = st.session.as_ref().unwrap().k_fast;
                    drop(st);
                    self.open_fast_server(gen, k);
                    st = self.st();
                } else if self.0.cfg.fast_role != FastRole::Server {
                    st.queue[1].push_back(out(Message::new(kind::FAST_FAIL, b"no fast path here".to_vec())));
                }
            }
            kind::FAST_OFFER => {
                if self.0.cfg.fast_role == FastRole::Client {
                    match FastOffer::decode(&m.body) {
                        Ok(offer) => {
                            st.fast_state = FastState::Negotiating(Instant::now() + self.0.cfg.fast_wait);
                            let k = st.session.as_ref().unwrap().k_fast;
                            let me = self.clone();
                            std::thread::spawn(move || {
                                let r = fast::connect(&offer, &k, Duration::from_millis(800));
                                me.fast_result(gen, r);
                            });
                        }
                        Err(e) => self.log(&format!("bad FAST_OFFER: {e}")),
                    }
                } else {
                    st.queue[1].push_back(out(Message::new(kind::FAST_FAIL, b"no fast client here".to_vec())));
                }
            }
            kind::FAST_FAIL => {
                self.log(&format!("peer: fast path failed ({})", String::from_utf8_lossy(&m.body)));
                st.fast_state = FastState::Failed;
            }
            kind::PING => {}
            _ => deliver = true,
        }
        // internally handled acked kinds are acked at once
        if m.wants_ack() && !deliver {
            st.queue[0].push_back(out(Message::ack(m.id)));
        }
        drop(st);
        self.0.cv.notify_all();
        if let Some((p, o)) = finished {
            self.log(&format!("file received: {} ({} bytes)", p.display(), o.size));
            self.0.handler.on_file(self, &p, &o);
        }
        if deliver {
            let r = self.0.handler.on_message(self, &m);
            if m.wants_ack() {
                let reply = match r {
                    Ok(()) => Message::ack(m.id),
                    Err(e) => Message::new(kind::ERROR, [&m.id.to_be_bytes()[..], e.as_bytes()].concat()),
                };
                self.st().queue[0].push_back(out(reply));
                self.0.cv.notify_all();
            }
        }
    }

    fn complete(&self, st: &mut State, j: RecvJob) -> Option<(PathBuf, FileOffer)> {
        let inbox = Inbox { dir: self.0.cfg.inbox.clone() };
        let offer = j.offer.clone();
        let (status, res) = match inbox.finish(j) {
            Ok(p) => (file::DONE_OK, Some((p, offer.clone()))),
            Err(e) => {
                self.log(&format!("file {} failed: {e}", offer.name));
                (file::DONE_BAD_HASH, None)
            }
        };
        if status == file::DONE_OK {
            st.done_recv.push_back((offer.id, status));
            while st.done_recv.len() > 64 {
                st.done_recv.pop_front();
            }
        }
        st.queue[1].push_back(out(Message::new(kind::FILE_DONE, file::done_body(&offer.id, status))));
        res
    }

    // ------------------------------------------------------------------ fast path

    fn open_fast_server(&self, gen: u64, k: [u8; 32]) {
        let addrs = (self.0.cfg.fast_addrs)();
        match FastListener::open(addrs) {
            Ok(l) if !l.offer.addrs.is_empty() => {
                let body = l.offer.encode();
                self.log(&format!("fast path offered on {:?}:{}", l.offer.addrs, l.offer.port));
                {
                    let mut st = self.st();
                    st.queue[1].push_back(out(Message::new(kind::FAST_OFFER, body)));
                }
                self.0.cv.notify_all();
                let me = self.clone();
                std::thread::spawn(move || {
                    let r = l.accept(&k);
                    me.fast_result(gen, r);
                });
            }
            Ok(_) => self.fast_result(gen, Err("no LAN address (Wi-Fi off?)".into())),
            Err(e) => self.fast_result(gen, Err(format!("listen: {e}"))),
        }
    }

    fn fast_result(&self, gen: u64, r: Result<FastConn, String>) {
        let mut st = self.st();
        if st.session.as_ref().map(|s| s.gen) != Some(gen) {
            if let Ok(c) = r {
                let _ = c.stream.shutdown(std::net::Shutdown::Both);
            }
            return;
        }
        match r {
            Ok(c) => {
                let FastConn { stream, sealer, opener, peer } = c;
                let (Ok(rd), Ok(cl)) = (stream.try_clone(), stream.try_clone()) else {
                    st.fast_state = FastState::Failed;
                    return;
                };
                let close: Closer = Arc::new(move || {
                    let _ = cl.shutdown(std::net::Shutdown::Both);
                });
                st.fast = Some((Arc::new(Mutex::new(Pipe { w: Box::new(stream), sealer })), close));
                st.fast_state = FastState::Up;
                st.fast_last_use = Instant::now();
                drop(st);
                self.log(&format!("fast path up ({peer})"));
                let me = self.clone();
                std::thread::spawn(move || me.reader(Box::new(rd), opener, gen, true));
            }
            Err(e) => {
                st.fast_state = FastState::Failed;
                if self.0.cfg.fast_role == FastRole::Client {
                    st.queue[1].push_back(out(Message::new(kind::FAST_FAIL, e.as_bytes().to_vec())));
                }
                drop(st);
                self.log(&format!("fast path not available: {e} (staying on Bluetooth)"));
            }
        }
        self.0.cv.notify_all();
    }

    fn wants_fast(&self, j: &SendJob, peer_caps: u32) -> bool {
        let c = &self.0.cfg;
        let ok = match c.fast_role {
            FastRole::Client => peer_caps & caps::FAST_SERVER != 0,
            FastRole::Server => peer_caps & caps::FAST_CLIENT != 0,
            FastRole::None => false,
        };
        ok && j.prefer_fast.unwrap_or(j.offer.size >= c.fast_min_size)
    }

    // ------------------------------------------------------------------ the driver (only writer)

    fn next_job(&self, st: &mut State) -> Job {
        let now = Instant::now();
        if st.stopped {
            return Job::Exit;
        }
        // expire what waited too long
        for q in st.queue.iter_mut() {
            let before = q.len();
            q.retain(|o| o.deadline > now);
            if q.len() != before {
                self.0.handler.log(&format!("[{}] {} message(s) expired unsent", role::name(self.0.cfg.peer_role), before - q.len()));
            }
        }
        let has_work = st.queue.iter().any(|q| !q.is_empty()) || !st.unacked.is_empty() || st.sends.iter().any(|j| j.done.is_none());
        let Some(s) = &st.session else {
            if !has_work || self.0.connector.is_none() {
                return Job::Wait(None);
            }
            return match st.next_try {
                Some(t) if t > now => Job::Wait(Some(t - now)),
                _ => Job::Connect,
            };
        };
        let (gen, pipe, peer_caps, k_fast) = (s.gen, s.pipe.clone(), s.peer.caps, s.k_fast);
        if let Some(o) = st.unacked.iter().filter_map(|o| o.sent_at).min() {
            if now.duration_since(o) > self.0.cfg.ack_timeout {
                return Job::Teardown(gen, "no ACK in time".into());
            }
        }
        for p in 0..2 {
            if let Some(o) = st.queue[p].pop_front() {
                let m = o.m.clone();
                if m.wants_ack() {
                    st.unacked.push(Out { sent_at: Some(now), ..o });
                }
                return Job::Write(pipe, m, gen, false);
            }
        }
        // bulk: one chunk of the first active outgoing file
        let fast_state = st.fast_state;
        if let FastState::Negotiating(until) = fast_state {
            if now > until {
                st.fast_state = FastState::Failed;
                self.0.handler.log(&format!("[{}] fast path negotiation timed out", role::name(self.0.cfg.peer_role)));
            }
        }
        let fast_pipe = st.fast.as_ref().map(|(p, _)| p.clone());
        let mut wait_fast = None;
        let mut chunk = None;
        let mut open_server = false;
        if let Some(i) = st.sends.iter().position(|j| j.accepted && j.done.is_none() && j.remaining() > 0) {
            let want_fast = self.wants_fast(&st.sends[i], peer_caps);
            let use_fast = want_fast && st.fast_state == FastState::Up;
            if want_fast && st.fast_state == FastState::Idle {
                st.fast_state = FastState::Negotiating(now + self.0.cfg.fast_wait);
                match self.0.cfg.fast_role {
                    FastRole::Client => st.queue[1].push_back(out(Message::new(kind::FAST_REQ, vec![]))),
                    _ => open_server = true,
                }
            } else if let (true, FastState::Negotiating(until)) = (want_fast, st.fast_state) {
                wait_fast = Some(until.saturating_duration_since(now));
            } else {
                let max = if use_fast { self.0.cfg.chunk_fast } else { self.0.cfg.chunk_bt }.min(MAX_PLAINTEXT - 40);
                let j = &mut st.sends[i];
                match j.next_chunk(max) {
                    Ok(Some((off, data))) => {
                        if use_fast {
                            j.bytes_fast += data.len() as u64;
                        } else {
                            j.bytes_bt += data.len() as u64;
                        }
                        let mut body = file::id_offset_body(&j.offer.id, off);
                        body.extend_from_slice(&data);
                        chunk = Some((Message::new(kind::FILE_CHUNK, body), use_fast));
                    }
                    Ok(None) => {}
                    Err(e) => j.done = Some(Err(format!("read {}: {e}", j.path.display()))),
                }
            }
        }
        if open_server {
            return Job::OpenFastServer(gen, k_fast);
        }
        if let Some((m, use_fast)) = chunk {
            if use_fast {
                st.fast_last_use = now;
                return Job::Write(fast_pipe.unwrap(), m, gen, true);
            }
            return Job::Write(pipe, m, gen, false);
        }
        // nothing to write
        let receiving = !st.recvs.is_empty() && now.duration_since(st.last_activity) < Duration::from_secs(60);
        let busy = !st.unacked.is_empty() || st.sends.iter().any(|j| j.done.is_none()) || receiving;
        if st.fast_state == FastState::Up && !st.sends.iter().any(|j| j.done.is_none()) && now.duration_since(st.fast_last_use) > Duration::from_secs(5) && st.recvs.is_empty() {
            return Job::CloseFast;
        }
        let idle_at = (st.last_activity + self.0.cfg.idle_close).max(st.warm_until);
        if !busy && now >= idle_at {
            return Job::Teardown(gen, "idle".into());
        }
        let mut wake = if busy { Duration::from_secs(1) } else { idle_at - now };
        if let Some(w) = wait_fast {
            wake = wake.min(w + Duration::from_millis(10));
        }
        if st.fast_state == FastState::Up {
            wake = wake.min(Duration::from_secs(5));
        }
        Job::Wait(Some(wake))
    }

    fn driver(&self) {
        loop {
            // decide and (if idle) wait under ONE lock acquisition, so no notify is lost
            let job = {
                let mut st = self.st();
                match self.next_job(&mut st) {
                    Job::Wait(None) => {
                        drop(self.0.cv.wait(st));
                        continue;
                    }
                    Job::Wait(Some(d)) => {
                        drop(self.0.cv.wait_timeout(st, d));
                        continue;
                    }
                    j => j,
                }
            };
            match job {
                Job::Exit => return,
                Job::Wait(_) => {}
                Job::Teardown(g, why) => self.teardown(g, &why),
                Job::CloseFast => {
                    let mut st = self.st();
                    if let Some((_, c)) = st.fast.take() {
                        c();
                    }
                    st.fast_state = FastState::Idle;
                    drop(st);
                    self.log("fast path idle: closed");
                }
                Job::OpenFastServer(g, k) => self.open_fast_server(g, k),
                Job::Write(pipe, m, gen, fast) => {
                    let r = {
                        let mut p = pipe.lock().unwrap_or_else(|e| e.into_inner());
                        let rec = p.sealer.seal(&m.encode());
                        write_record(&mut p.w, &rec)
                    };
                    match r {
                        Ok(()) => self.st().last_activity = Instant::now(),
                        Err(e) if fast => {
                            let mut st = self.st();
                            if let Some((_, c)) = st.fast.take() {
                                c();
                            }
                            st.fast_state = FastState::Failed;
                            drop(st);
                            self.log(&format!("fast path write failed ({e}); continuing over Bluetooth"));
                        }
                        Err(e) => self.teardown(gen, &format!("write: {e}")),
                    }
                }
                Job::Connect => {
                    let r = self.connect_once();
                    let mut st = self.st();
                    match r {
                        Ok(()) => {}
                        Err(e) => {
                            let b = st.backoff;
                            st.next_try = Some(Instant::now() + b);
                            st.backoff = (b * 2).min(self.0.cfg.backoff_max);
                            drop(st);
                            self.log(&format!("connect failed: {e}; retry in {} ms", b.as_millis()));
                        }
                    }
                }
            }
        }
    }

    fn connect_once(&self) -> Result<(), String> {
        let c = self.0.connector.as_ref().ok_or("no connector")?;
        let t0 = Instant::now();
        let mut conn = c.connect()?;
        let e = Ephemeral::generate();
        let cfg = &self.0.cfg;
        let me = Hello::new(cfg.my_role, cfg.pc_id, &e, cfg.caps, &cfg.name);
        let done = Arc::new(AtomicBool::new(false));
        watchdog(done.clone(), conn.close.clone(), cfg.handshake_timeout);
        let r = handshake::initiate(&mut conn.reader, &mut conn.writer, &me, &e, &cfg.cred);
        done.store(true, Ordering::SeqCst);
        match r {
            Ok(est) => {
                self.log(&format!("connected + authenticated in {} ms", t0.elapsed().as_millis()));
                self.install(conn, est);
                Ok(())
            }
            Err(x) => {
                (conn.close)();
                Err(x)
            }
        }
    }
}

fn out(m: Message) -> Out {
    Out { m, deadline: Instant::now() + Duration::from_secs(300), sent_at: None }
}

/// Close the pipe if the handshake has not finished in time (Bluetooth reads cannot time out).
pub fn watchdog(done: Arc<AtomicBool>, close: Closer, after: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(after);
        if !done.load(Ordering::SeqCst) {
            close();
        }
    });
}

/// Listener side: run the responder handshake on an incoming pipe and hand it to the link
/// `pick` chooses from the peer's HELLO (role, name). Blocking; run it on its own thread.
pub fn respond_and_attach(mut conn: Conn, timeout: Duration, pick: impl FnOnce(&Hello) -> Option<Link>) -> Result<Link, String> {
    let done = Arc::new(AtomicBool::new(false));
    watchdog(done.clone(), conn.close.clone(), timeout);
    let mut chosen = None;
    let r = handshake::respond(&mut conn.reader, &mut conn.writer, |h| {
        let l = pick(h).ok_or_else(|| format!("no link for a {} here", role::name(h.role)))?;
        let parts = l.responder_parts();
        chosen = Some(l);
        Ok(parts)
    });
    done.store(true, Ordering::SeqCst);
    match (r, chosen) {
        (Ok((est, _)), Some(l)) => {
            l.accept(conn, est);
            Ok(l)
        }
        (Ok(_), None) => {
            (conn.close)();
            Err("no link chosen".into())
        }
        (Err(e), _) => {
            (conn.close)();
            Err(e)
        }
    }
}
