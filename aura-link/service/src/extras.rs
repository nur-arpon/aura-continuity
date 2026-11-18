//! Item 15 extras over Aura Link: ring the laptop, PC media controls, presentation remote.
//! The logic lives here behind a small [Backend] (Windows in production, a mock in tests) and a
//! [Peer] (a `Link` in production), so every rule is unit-tested without Bluetooth or speakers.
//!
//! Battery / privacy rules: nothing polls unless a phone or watch screen asked for media updates
//! (`MEDIA_CMD refresh`, valid 2 minutes, only while its session is connected); presenter keys are
//! refused while the PC is locked; each feature has an owner switch.

use crate::media::Snapshot;
use aura_link::link::Link;
use aura_link::msg::{kind, media_cmd, present_cmd, Message};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Whoever sent us something (a `Link` in production).
pub trait Peer: Send + Sync {
    fn send(&self, m: Message) -> u32;
    fn connected(&self) -> bool;
    fn name(&self) -> String;
    /// 1 = phone, 2 = watch (see handshake::role); only used to keep one media watcher per device.
    fn role(&self) -> u8;
}

impl Peer for Link {
    fn send(&self, m: Message) -> u32 {
        Link::send(self, m)
    }
    fn connected(&self) -> bool {
        Link::connected(self)
    }
    fn name(&self) -> String {
        self.peer().map(|h| h.name).filter(|n| !n.is_empty()).unwrap_or_else(|| "your device".into())
    }
    fn role(&self) -> u8 {
        self.config().peer_role
    }
}

/// The things that touch the machine.
pub trait Backend: Send + Sync {
    fn media_snapshot(&self, want_art: bool) -> Option<Snapshot>;
    fn media_cmd(&self, cmd: u8) -> Result<(), String>;
    fn present(&self, cmd: u8) -> Result<(), String>;
    fn ring_start(&self, full_volume: bool) -> Result<(), String>;
    fn ring_stop(&self);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingEvent {
    pub on: bool,
    pub device: String,
}

pub type RingHook = Arc<dyn Fn(RingEvent) + Send + Sync>;

/// Owner switches and hooks (the tray app wires the toast window to `on_ring`).
#[derive(Clone)]
pub struct ExtrasOptions {
    pub ring: Arc<AtomicBool>,
    pub media: Arc<AtomicBool>,
    pub presenter: Arc<AtomicBool>,
    pub ring_full_volume: Arc<AtomicBool>,
    pub on_ring: Option<RingHook>,
}

impl Default for ExtrasOptions {
    fn default() -> Self {
        let on = || Arc::new(AtomicBool::new(true));
        Self { ring: on(), media: on(), presenter: on(), ring_full_volume: on(), on_ring: None }
    }
}

#[derive(Default)]
struct Watcher {
    running: AtomicBool,
    /// unix ms until which updates are wanted
    until: AtomicU64,
    /// send everything now, even if unchanged
    force: AtomicBool,
    /// quick polling until this unix ms (just after a command)
    fast_until: AtomicU64,
}

pub struct Extras {
    backend: Arc<dyn Backend>,
    pub opts: ExtrasOptions,
    log: crate::Log,
    watchers: [Arc<Watcher>; 3],
    ringing: Mutex<Option<(Arc<dyn Peer>, Instant)>>,
    pub watch_for: Duration,
    pub poll: Duration,
    /// a ring stops by itself after this long
    pub max_ring: Duration,
}

fn now_ms() -> u64 {
    aura_link::unix_ms()
}

impl Extras {
    pub fn new(backend: Arc<dyn Backend>, opts: ExtrasOptions, log: crate::Log) -> Arc<Self> {
        Arc::new(Self { backend, opts, log, watchers: Default::default(), ringing: Mutex::new(None), watch_for: Duration::from_secs(120), poll: Duration::from_millis(1500), max_ring: Duration::from_secs(180) })
    }

    pub fn is_ringing(&self) -> bool {
        self.ringing.lock().unwrap().is_some()
    }

    /// Handles RING / MEDIA_CMD / PRESENT. `locked` = the PC session is locked. Err = refused (the peer gets an ERROR).
    pub fn on_message(self: &Arc<Self>, peer: Arc<dyn Peer>, m: &Message, locked: bool) -> Result<(), String> {
        let cmd = m.body.first().copied().unwrap_or(0);
        match m.kind {
            kind::RING => {
                if cmd == 0 {
                    self.stop_ring(false);
                    return Ok(());
                }
                if !self.opts.ring.load(Ordering::SeqCst) {
                    (self.log)("RING refused: switched off in Aura > Remote");
                    return Err("Ring my laptop is switched off on the PC (Aura > Remote)".into());
                }
                self.start_ring(peer)
            }
            kind::MEDIA_CMD => {
                if !self.opts.media.load(Ordering::SeqCst) {
                    return Err("Media controls are switched off on the PC (Aura > Remote)".into());
                }
                self.media(peer, cmd)
            }
            kind::PRESENT => {
                if !self.opts.presenter.load(Ordering::SeqCst) {
                    return Err("The presentation remote is switched off on the PC (Aura > Remote)".into());
                }
                if locked {
                    (self.log)(&format!("PRESENT {} refused: the PC is locked", present_cmd::name(cmd)));
                    return Err("The PC is locked, so the remote does nothing".into());
                }
                (self.log)(&format!("PRESENT {} from {}", present_cmd::name(cmd), peer.name()));
                self.backend.present(cmd)
            }
            _ => Ok(()),
        }
    }

    pub fn start_ring(self: &Arc<Self>, origin: Arc<dyn Peer>) -> Result<(), String> {
        let name = origin.name();
        self.backend.ring_start(self.opts.ring_full_volume.load(Ordering::SeqCst))?;
        let since = Instant::now();
        *self.ringing.lock().unwrap() = Some((origin, since));
        let me = self.clone();
        let _ = std::thread::Builder::new().name("aura-ring-timer".into()).spawn(move || {
            std::thread::sleep(me.max_ring);
            let still = me.ringing.lock().unwrap().as_ref().map(|(_, t)| *t == since).unwrap_or(false);
            if still {
                (me.log)("RING timed out");
                me.stop_ring(true);
            }
        });
        (self.log)(&format!("RING from {name}: chime on"));
        if let Some(h) = &self.opts.on_ring {
            h(RingEvent { on: true, device: name });
        }
        Ok(())
    }

    /// The owner dismissed it on the PC (`from_pc`) or the device sent RING 0. Tells the origin to stop its own UI.
    pub fn stop_ring(&self, from_pc: bool) {
        let was = self.ringing.lock().unwrap().take();
        self.backend.ring_stop();
        if let Some((origin, since)) = was {
            (self.log)(&format!("RING stopped after {} s ({})", since.elapsed().as_secs(), if from_pc { "dismissed on the PC" } else { "stopped from the device" }));
            if from_pc {
                origin.send(Message::ring(false));
            }
            if let Some(h) = &self.opts.on_ring {
                h(RingEvent { on: false, device: origin.name() });
            }
        }
    }

    /// A session came (back) up while a screen on that device still wants updates: the watcher stopped
    /// when the idle session closed, so start it again (the watch Music screen was stuck on "Nothing playing").
    pub fn peer_up(self: &Arc<Self>, peer: Arc<dyn Peer>) {
        let w = self.watchers[(peer.role() as usize).min(2)].clone();
        if now_ms() < w.until.load(Ordering::SeqCst) {
            w.force.store(true, Ordering::SeqCst);
            self.ensure_watcher(peer, w);
        }
    }

    fn media(self: &Arc<Self>, peer: Arc<dyn Peer>, cmd: u8) -> Result<(), String> {
        let w = self.watchers[(peer.role() as usize).min(2)].clone();
        match cmd {
            media_cmd::REFRESH => {
                w.until.store(now_ms() + self.watch_for.as_millis() as u64, Ordering::SeqCst);
                w.force.store(true, Ordering::SeqCst);
                self.ensure_watcher(peer, w);
                Ok(())
            }
            media_cmd::STOP_UPDATES => {
                w.until.store(0, Ordering::SeqCst);
                Ok(())
            }
            c => {
                (self.log)(&format!("MEDIA {} from {}", media_cmd::name(c), peer.name()));
                self.backend.media_cmd(c)?;
                // show the effect fast to a screen that is watching
                w.fast_until.store(now_ms() + 3000, Ordering::SeqCst);
                if now_ms() < w.until.load(Ordering::SeqCst) {
                    w.force.store(true, Ordering::SeqCst);
                    self.ensure_watcher(peer, w);
                }
                Ok(())
            }
        }
    }

    fn ensure_watcher(self: &Arc<Self>, peer: Arc<dyn Peer>, w: Arc<Watcher>) {
        if w.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        let spawned = std::thread::Builder::new().name("aura-media-watch".into()).spawn(move || {
            let mut last: Option<Snapshot> = None;
            let mut last_key = String::new();
            let mut last_sent = Instant::now();
            loop {
                if now_ms() >= w.until.load(Ordering::SeqCst) || !peer.connected() {
                    break;
                }
                let force = w.force.swap(false, Ordering::SeqCst);
                if let Some(mut snap) = me.backend.media_snapshot(false) {
                    let key = snap.track_key();
                    let new_track = key != last_key;
                    let changed = match &last {
                        None => true,
                        Some(p) => {
                            let (a, b) = (&p.state, &snap.state);
                            let expect = if a.playing { a.position_ms + last_sent.elapsed().as_millis() as u64 } else { a.position_ms };
                            a.playing != b.playing || new_track || a.volume != b.volume || b.position_ms.abs_diff(expect) > 4000
                        }
                    };
                    if force || changed {
                        if force || new_track {
                            if let Some(full) = me.backend.media_snapshot(true) {
                                snap.art = full.art;
                            }
                            peer.send(Message::new(kind::MEDIA_STATE, snap.state.encode()));
                            peer.send(Message::media_art(snap.art.as_deref().unwrap_or(&[])));
                        } else {
                            peer.send(Message::new(kind::MEDIA_STATE, snap.state.encode()));
                        }
                        last_key = key;
                        last_sent = Instant::now();
                        last = Some(snap);
                    }
                }
                let step = if now_ms() < w.fast_until.load(Ordering::SeqCst) { Duration::from_millis(300) } else { me.poll };
                let end = Instant::now() + step;
                while Instant::now() < end && !w.force.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            w.running.store(false, Ordering::SeqCst);
        });
        if spawned.is_err() {
            self.watchers.iter().for_each(|w| w.running.store(false, Ordering::SeqCst));
        }
    }
}

// ------------------------------------------------------------------ Windows backend

#[cfg(windows)]
pub struct WinBackend {
    saved: Mutex<Option<crate::chime::Saved>>,
}

#[cfg(windows)]
impl WinBackend {
    pub fn new() -> Arc<Self> {
        let b = Arc::new(Self { saved: Mutex::new(None) });
        b.recover();
        b
    }

    fn restore_file() -> std::path::PathBuf {
        crate::data_dir().join("ring-restore.json")
    }

    /// A crash while ringing must not leave the speakers at full volume: put back what was saved.
    fn recover(&self) {
        if let Ok(t) = std::fs::read_to_string(Self::restore_file()) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
                let level = v["level"].as_f64().unwrap_or(0.3) as f32;
                let _ = crate::chime::endpoint_set(level, v["muted"].as_bool().unwrap_or(false));
            }
            let _ = std::fs::remove_file(Self::restore_file());
        }
    }
}

#[cfg(windows)]
impl Backend for WinBackend {
    fn media_snapshot(&self, want_art: bool) -> Option<Snapshot> {
        crate::media::snapshot(want_art)
    }
    fn media_cmd(&self, cmd: u8) -> Result<(), String> {
        crate::media::command(cmd)
    }
    fn present(&self, cmd: u8) -> Result<(), String> {
        crate::present::press(cmd)
    }
    fn ring_start(&self, full_volume: bool) -> Result<(), String> {
        let mut g = self.saved.lock().unwrap();
        if g.is_none() {
            if let Ok(now) = crate::chime::endpoint_get() {
                let _ = std::fs::create_dir_all(crate::data_dir());
                let _ = std::fs::write(Self::restore_file(), serde_json::json!({"level": now.level, "muted": now.muted}).to_string());
                if full_volume {
                    let _ = crate::chime::endpoint_set(crate::chime::ring_level(now.level, true), false);
                } else if now.muted {
                    let _ = crate::chime::endpoint_set(now.level, false);
                }
                *g = Some(now);
            }
        }
        crate::chime::play_loop()?;
        Ok(())
    }
    fn ring_stop(&self) {
        crate::chime::stop_playing();
        let mut g = self.saved.lock().unwrap();
        if let Some(s) = g.take() {
            let _ = crate::chime::endpoint_set(s.level, s.muted);
            let _ = std::fs::remove_file(Self::restore_file());
        }
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct MockPeer {
        sent: Mutex<Vec<Message>>,
        up: AtomicBool,
        role: u8,
    }
    impl Peer for MockPeer {
        fn send(&self, m: Message) -> u32 {
            self.sent.lock().unwrap().push(m);
            1
        }
        fn connected(&self) -> bool {
            self.up.load(Ordering::SeqCst)
        }
        fn name(&self) -> String {
            "Test phone".into()
        }
        fn role(&self) -> u8 {
            self.role
        }
    }
    fn peer() -> Arc<MockPeer> {
        Arc::new(MockPeer { up: AtomicBool::new(true), role: 1, ..Default::default() })
    }

    #[derive(Default)]
    struct MockBackend {
        cmds: Mutex<Vec<u8>>,
        keys: Mutex<Vec<u8>>,
        ring: AtomicUsize,
        ring_stops: AtomicUsize,
        snap: Mutex<Snapshot>,
    }
    impl Backend for MockBackend {
        fn media_snapshot(&self, art: bool) -> Option<Snapshot> {
            let mut s = self.snap.lock().unwrap().clone();
            if art {
                s.art = Some(vec![0xff, 0xd8, 0xff, 0xd9]);
            }
            Some(s)
        }
        fn media_cmd(&self, c: u8) -> Result<(), String> {
            self.cmds.lock().unwrap().push(c);
            Ok(())
        }
        fn present(&self, c: u8) -> Result<(), String> {
            self.keys.lock().unwrap().push(c);
            Ok(())
        }
        fn ring_start(&self, _full: bool) -> Result<(), String> {
            self.ring.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn ring_stop(&self) {
            self.ring_stops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn setup() -> (Arc<Extras>, Arc<MockBackend>, Arc<MockPeer>) {
        let b = Arc::new(MockBackend::default());
        b.snap.lock().unwrap().state.title = "Song".into();
        let mut x = Extras::new(b.clone(), ExtrasOptions::default(), Arc::new(|_| {}));
        Arc::get_mut(&mut x).unwrap().poll = Duration::from_millis(40);
        (x, b, peer())
    }

    fn wait(f: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn ring_starts_stops_and_tells_the_origin_when_dismissed_on_the_pc() {
        let (x, b, p) = setup();
        let events = Arc::new(Mutex::new(Vec::new()));
        let e2 = events.clone();
        let x2 = Extras::new(b.clone(), ExtrasOptions { on_ring: Some(Arc::new(move |e| e2.lock().unwrap().push(e))), ..Default::default() }, Arc::new(|_| {}));
        drop(x);
        x2.on_message(p.clone(), &Message::ring(true), true).unwrap(); // works while locked: that is the point
        assert!(x2.is_ringing());
        assert_eq!(b.ring.load(Ordering::SeqCst), 1);
        x2.stop_ring(true);
        assert!(!x2.is_ringing());
        assert_eq!(b.ring_stops.load(Ordering::SeqCst), 1);
        assert_eq!(p.sent.lock().unwrap().last().unwrap().body, vec![0], "origin told to stop");
        let ev = events.lock().unwrap().clone();
        assert_eq!(ev, vec![RingEvent { on: true, device: "Test phone".into() }, RingEvent { on: false, device: "Test phone".into() }]);
        // a device-side stop does not echo back
        let n = p.sent.lock().unwrap().len();
        x2.on_message(p.clone(), &Message::ring(true), false).unwrap();
        x2.on_message(p.clone(), &Message::ring(false), false).unwrap();
        assert_eq!(p.sent.lock().unwrap().len(), n);
    }

    #[test]
    fn a_ring_stops_by_itself() {
        let b = Arc::new(MockBackend::default());
        let mut x = Extras::new(b.clone(), ExtrasOptions::default(), Arc::new(|_| {}));
        Arc::get_mut(&mut x).unwrap().max_ring = Duration::from_millis(80);
        x.on_message(peer(), &Message::ring(true), false).unwrap();
        assert!(wait(|| !x.is_ringing()));
        assert_eq!(b.ring_stops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn switches_refuse_each_feature() {
        let (x, b, p) = setup();
        x.opts.ring.store(false, Ordering::SeqCst);
        x.opts.media.store(false, Ordering::SeqCst);
        x.opts.presenter.store(false, Ordering::SeqCst);
        assert!(x.on_message(p.clone(), &Message::ring(true), false).is_err());
        assert!(x.on_message(p.clone(), &Message::media_cmd(media_cmd::NEXT), false).is_err());
        assert!(x.on_message(p.clone(), &Message::present(present_cmd::NEXT), false).is_err());
        assert_eq!(b.ring.load(Ordering::SeqCst) + b.cmds.lock().unwrap().len() + b.keys.lock().unwrap().len(), 0);
    }

    #[test]
    fn presenter_keys_never_fire_on_a_locked_pc() {
        let (x, b, p) = setup();
        assert!(x.on_message(p.clone(), &Message::present(present_cmd::NEXT), true).is_err());
        assert!(b.keys.lock().unwrap().is_empty());
        x.on_message(p.clone(), &Message::present(present_cmd::NEXT), false).unwrap();
        x.on_message(p.clone(), &Message::present(present_cmd::BLACK), false).unwrap();
        assert_eq!(*b.keys.lock().unwrap(), vec![present_cmd::NEXT, present_cmd::BLACK]);
    }

    #[test]
    fn media_commands_run_even_when_locked() {
        let (x, b, p) = setup();
        for c in [media_cmd::PLAY_PAUSE, media_cmd::NEXT, media_cmd::PREV, media_cmd::VOL_UP, media_cmd::VOL_DOWN, media_cmd::MUTE] {
            x.on_message(p.clone(), &Message::media_cmd(c), true).unwrap();
        }
        assert_eq!(b.cmds.lock().unwrap().len(), 6);
    }

    #[test]
    fn refresh_sends_state_and_art_then_only_changes_then_stops_when_asked() {
        let (x, b, p) = setup();
        x.on_message(p.clone(), &Message::media_cmd(media_cmd::REFRESH), false).unwrap();
        let count = |k: u8| p.sent.lock().unwrap().iter().filter(|m| m.kind == k).count();
        assert!(wait(|| count(kind::MEDIA_STATE) == 1 && count(kind::MEDIA_ART) == 1));
        // unchanged: nothing more after several polls
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(count(kind::MEDIA_STATE), 1);
        // a new track: state + art again
        b.snap.lock().unwrap().state.title = "Next song".into();
        assert!(wait(|| count(kind::MEDIA_STATE) == 2 && count(kind::MEDIA_ART) == 2));
        // pause: state only (no new art)
        b.snap.lock().unwrap().state.playing = false;
        b.snap.lock().unwrap().state.volume = Some((10, false));
        assert!(wait(|| count(kind::MEDIA_STATE) == 3));
        assert_eq!(count(kind::MEDIA_ART), 2);
        let body = p.sent.lock().unwrap().iter().rfind(|m| m.kind == kind::MEDIA_STATE).unwrap().body.clone();
        assert_eq!(aura_link::msg::MediaState::decode(&body).unwrap().volume, Some((10, false)));
        // stop: the watcher ends
        x.on_message(p.clone(), &Message::media_cmd(media_cmd::STOP_UPDATES), false).unwrap();
        assert!(wait(|| !x.watchers[1].running.load(Ordering::SeqCst)));
        b.snap.lock().unwrap().state.title = "Ignored".into();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(count(kind::MEDIA_STATE), 3);
    }
    #[test]
    fn watcher_ends_when_the_session_drops() {
        let (x, _b, p) = setup();
        x.on_message(p.clone(), &Message::media_cmd(media_cmd::REFRESH), false).unwrap();
        assert!(wait(|| x.watchers[1].running.load(Ordering::SeqCst)));
        p.up.store(false, Ordering::SeqCst);
        assert!(wait(|| !x.watchers[1].running.load(Ordering::SeqCst)));
    }

    #[test]
    fn a_command_while_watching_pushes_the_new_state_quickly() {
        let (x, b, p) = setup();
        x.on_message(p.clone(), &Message::media_cmd(media_cmd::REFRESH), false).unwrap();
        let count = || p.sent.lock().unwrap().iter().filter(|m| m.kind == kind::MEDIA_STATE).count();
        assert!(wait(|| count() == 1));
        b.snap.lock().unwrap().state.playing = true;
        x.on_message(p.clone(), &Message::media_cmd(media_cmd::PLAY_PAUSE), false).unwrap();
        assert!(wait(|| count() == 2));
    }
}
