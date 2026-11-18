//! Typed messages inside DATA records: `kind(1) | flags(1) | id(u32 BE) | body`.
//! flags bit0 = ACK requested (the receiver answers ACK(id); the sender keeps the message until
//! then and resends it on the next session; the receiver drops duplicate ids).
//! Unknown kinds are acked (if asked) and handed to the application, never an error, so a newer
//! peer can add kinds without breaking an older one.

use crate::wire::Cur;

pub const FLAG_ACK: u8 = 1;

pub mod kind {
    pub const ACK: u8 = 0x01;
    pub const ERROR: u8 = 0x03;
    pub const PING: u8 = 0x04;
    /// device -> PC: an existing signed lock event frame, byte for byte (`02 07 | CU1E..` or `02 07 | CUW1..`)
    pub const LOCK: u8 = 0x10;
    /// watch -> PC: the existing signed "Unlock PC" push frame (`02 07 | CUW1 COMMAND unlock_pc||pc_id`)
    pub const UNLOCK_REQUEST: u8 = 0x11;
    /// PC -> devices: session state, body u8 (0 unlocked, 1 locked)
    pub const PC_STATE: u8 = 0x12;
    /// PC -> phone (Aura Anywhere, item 17): what became of a remote "Unlock PC". utf-8 JSON
    /// `{"ok":bool,"why":"..","at_ms":u64}`; ok = the session really unlocked inside the approval window.
    pub const UNLOCK_RESULT: u8 = 0x13;
    /// phone/watch -> PC: remote power. utf-8 JSON `{"action":"restart"|"shutdown"|"sleep","from":"<device name>"}`. Acked;
    /// short ttl so a power command never runs late after a reconnect.
    pub const PC_POWER: u8 = 0x14;
    pub const TEXT: u8 = 0x20;
    /// body: position_ms u64 (u64::MAX = none) | utf-8 url
    pub const URL: u8 = 0x21;
    /// body: mime_len u8 | mime | bytes
    pub const CLIPBOARD: u8 = 0x22;
    /// Handoff suggestion (item 12): utf-8 JSON `{"app","title","url","pos_ms","playing","at_ms"}`;
    /// an empty object = "nothing to continue any more". Never acked (stale suggestions are worthless).
    pub const ACTIVITY: u8 = 0x23;
    /// phone -> PC (item 17): "what was I last doing on the PC?" Empty body, acked; the PC answers
    /// with [LAST_ACTIVITY].
    pub const LAST_ACTIVITY_GET: u8 = 0x24;
    /// PC -> phone (item 17): the PC's last continuable activity, the ACTIVITY JSON plus
    /// `"last":true`; `{}` = nothing. Never acked (a fresh GET asks again).
    pub const LAST_ACTIVITY: u8 = 0x25;
    /// both ways (item 19, zero-tap continue): "what you were playing now continues on me".
    /// utf-8 JSON `{"op":"pause"|"mute"|"leave"|"play","title":"..","url":"..","app":".."}`; the
    /// receiver pauses (or mutes / leaves) the matching media; `play` = Undo. Acked.
    pub const CONTINUED: u8 = 0x26;
    pub const FILE_OFFER: u8 = 0x30;
    pub const FILE_ACCEPT: u8 = 0x31;
    pub const FILE_CHUNK: u8 = 0x32;
    pub const FILE_DONE: u8 = 0x33;
    pub const FILE_CANCEL: u8 = 0x34;
    /// body: playing u8 | position_ms u64 | duration_ms u64 | u16 len title | u16 len artist
    pub const MEDIA_STATE: u8 = 0x40;
    /// body: u8 command, see [media_cmd] (1 play/pause, 2 next, 3 previous, 4 volume up, 5 volume down,
    /// 6 mute, 7 refresh = "send me the state and keep me posted for 2 minutes", 8 stop updates)
    pub const MEDIA_CMD: u8 = 0x41;
    /// body: u8 (1 ring, 0 stop). Either direction: phone/watch -> PC rings the laptop, PC -> phone rings the phone.
    pub const RING: u8 = 0x42;
    /// PC -> phone/watch (item 15): the artwork of the track in the last MEDIA_STATE, a small JPEG
    /// (at most [MEDIA_ART_MAX] bytes; empty = no artwork). Never acked.
    pub const MEDIA_ART: u8 = 0x43;
    /// phone/watch -> PC (item 15): presentation remote, body u8 command, see [present_cmd]. Acked, short lifetime.
    pub const PRESENT: u8 = 0x44;
    /// phone/watch -> PC: remote mouse / keyboard, JSON body {"t":"move"|"click"|"scroll"|"text"|"key",..}. Never acked, ttl 2 s.
    pub const INPUT: u8 = 0x45;
    /// chat-head tree: phone -> PC = "send me your recent apps" (empty body or `{"ask":1}`); PC -> phone = the laptop's 3-4 most
    /// recent top-level windows as JSON `{"v":1,"at_ms":..,"apps":[{"id","name","exe","title","browser":bool,"icon":b64png?,"url"?,"pos_ms"?,"tabs":[{"title","url"?,"active":bool}]}]}`.
    /// Icons ride only the first time a link sees them. Never acked, never queued long.
    pub const LAPTOP_APPS: u8 = 0x48;
    /// "Now playing everywhere": phone -> PC, every media session on the phone as JSON
    /// `{"v":1,"at":ms,"sessions":[{"id","app","pkg","title","artist","playing","pos","dur","art"?:b64 jpeg}]}`.
    /// Sent on change only (art only when the track changed). Never acked.
    pub const PHONE_MEDIA: u8 = 0x49;
    /// PC -> phone: control one phone session `{"id","cmd":"toggle"|"play"|"pause"|"next"|"prev"|"seek","pos"?:ms}`. Acked, short ttl.
    pub const PHONE_MEDIA_CMD: u8 = 0x4A;
    /// Notification mirror, phone -> PC: `{"op":"post"|"remove","key","pkg","app","title","text","when","reply":bool,"icon"?:b64 png}`.
    /// Event-driven, batched by the phone. Never acked (a stale toast is worthless: ttl 30 s).
    pub const NOTIF: u8 = 0x4B;
    /// PC -> phone: act on a mirrored notification `{"key","act":"dismiss"|"reply","text"?}`. Acked, ttl 20 s.
    pub const NOTIF_ACT: u8 = 0x4C;
    pub const FAST_REQ: u8 = 0x50;
    pub const FAST_OFFER: u8 = 0x51;
    pub const FAST_FAIL: u8 = 0x52;
    /// both ways (item 17): `{"ip":"100.x.y.z","port":u16}` = where this device listens on the
    /// owner's tailnet. Sent at every session start; never acked.
    pub const TAILNET: u8 = 0x60;
    /// PC -> phone (remote control): where this PC's Sunshine game-stream host can be reached, so the phone never types an
    /// address: `{"v":1,"host":"PC name","lan":["192.168.x.y"],"tailnet":"100.x.y.z"|null,"port":47989,"installed":bool,"running":bool}`.
    /// Sent at every phone session start and whenever it changes; never acked.
    pub const LAPTOP_INFO: u8 = 0x61;

    pub fn name(k: u8) -> &'static str {
        match k {
            ACK => "ACK",
            ERROR => "ERROR",
            PING => "PING",
            LOCK => "LOCK",
            UNLOCK_REQUEST => "UNLOCK_REQUEST",
            PC_STATE => "PC_STATE",
            UNLOCK_RESULT => "UNLOCK_RESULT",
            PC_POWER => "PC_POWER",
            LAST_ACTIVITY_GET => "LAST_ACTIVITY_GET",
            LAST_ACTIVITY => "LAST_ACTIVITY",
            CONTINUED => "CONTINUED",
            TAILNET => "TAILNET",
            LAPTOP_INFO => "LAPTOP_INFO",
            TEXT => "TEXT",
            URL => "URL",
            CLIPBOARD => "CLIPBOARD",
            ACTIVITY => "ACTIVITY",
            FILE_OFFER => "FILE_OFFER",
            FILE_ACCEPT => "FILE_ACCEPT",
            FILE_CHUNK => "FILE_CHUNK",
            FILE_DONE => "FILE_DONE",
            FILE_CANCEL => "FILE_CANCEL",
            MEDIA_STATE => "MEDIA_STATE",
            MEDIA_CMD => "MEDIA_CMD",
            MEDIA_ART => "MEDIA_ART",
            PRESENT => "PRESENT",
            INPUT => "INPUT",
            LAPTOP_APPS => "LAPTOP_APPS",
            PHONE_MEDIA => "PHONE_MEDIA",
            PHONE_MEDIA_CMD => "PHONE_MEDIA_CMD",
            NOTIF => "NOTIF",
            NOTIF_ACT => "NOTIF_ACT",
            RING => "RING",
            FAST_REQ => "FAST_REQ",
            FAST_OFFER => "FAST_OFFER",
            FAST_FAIL => "FAST_FAIL",
            _ => "UNKNOWN",
        }
    }
}

/// Largest artwork sent with [kind::MEDIA_ART]; one record holds it.
pub const MEDIA_ART_MAX: usize = 40 * 1024;

/// Bodies of [kind::MEDIA_CMD].
pub mod media_cmd {
    pub const PLAY_PAUSE: u8 = 1;
    pub const NEXT: u8 = 2;
    pub const PREV: u8 = 3;
    pub const VOL_UP: u8 = 4;
    pub const VOL_DOWN: u8 = 5;
    pub const MUTE: u8 = 6;
    pub const REFRESH: u8 = 7;
    pub const STOP_UPDATES: u8 = 8;
    pub fn name(c: u8) -> &'static str {
        match c {
            PLAY_PAUSE => "play/pause",
            NEXT => "next",
            PREV => "previous",
            VOL_UP => "volume up",
            VOL_DOWN => "volume down",
            MUTE => "mute",
            REFRESH => "refresh",
            STOP_UPDATES => "stop updates",
            _ => "unknown",
        }
    }
}

/// Bodies of [kind::PRESENT].
pub mod present_cmd {
    pub const NEXT: u8 = 1;
    pub const PREV: u8 = 2;
    /// black screen toggle (B)
    pub const BLACK: u8 = 3;
    /// white screen toggle (W)
    pub const WHITE: u8 = 4;
    /// start from the beginning (F5)
    pub const START: u8 = 5;
    /// end the show (Esc)
    pub const END: u8 = 6;
    pub fn name(c: u8) -> &'static str {
        match c {
            NEXT => "next slide",
            PREV => "previous slide",
            BLACK => "blank screen",
            WHITE => "white screen",
            START => "start show",
            END => "end show",
            _ => "unknown",
        }
    }
}

/// How long an unsent message of this kind may wait for a session (live controls must not fire minutes later).
pub fn ttl_secs(k: u8, default_secs: u64) -> u64 {
    match k {
        kind::LOCK | kind::UNLOCK_REQUEST => 110,
        kind::PC_POWER => 20,
        kind::MEDIA_CMD | kind::PRESENT => 8,
        kind::INPUT => 2,
        kind::LAPTOP_APPS => 20,
        kind::PHONE_MEDIA => 10,
        kind::PHONE_MEDIA_CMD => 8,
        kind::NOTIF => 30,
        kind::NOTIF_ACT => 20,
        kind::RING => 60,
        _ => default_secs,
    }
}

/// Kinds that mean a person just did something (opened an app, sent a link, pressed a control).
/// Seeing one, sent or received, keeps the session warm for `Config::warm_hold` so the next tap
/// does not pay a reconnect. Background pushes (media state, pings, tailnet, acks) do not count.
pub fn is_user_facing(k: u8) -> bool {
    matches!(k, kind::LOCK | kind::UNLOCK_REQUEST | kind::PC_POWER | kind::TEXT | kind::URL | kind::CLIPBOARD | kind::CONTINUED | kind::LAST_ACTIVITY_GET | kind::FILE_OFFER | kind::MEDIA_CMD | kind::PRESENT | kind::INPUT | kind::LAPTOP_APPS | kind::PHONE_MEDIA_CMD | kind::NOTIF_ACT)
}

/// 0 = lock / unlock / acks (always first), 1 = control and small payloads, 2 = bulk file data.
pub fn priority(k: u8) -> usize {
    match k {
        kind::ACK | kind::LOCK | kind::PC_POWER | kind::UNLOCK_REQUEST | kind::UNLOCK_RESULT | kind::PC_STATE | kind::ERROR => 0,
        kind::FILE_CHUNK => 2,
        _ => 1,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub kind: u8,
    pub flags: u8,
    pub id: u32,
    pub body: Vec<u8>,
}

impl Message {
    pub fn new(kind: u8, body: Vec<u8>) -> Self {
        // control messages that matter are acked by default; chunks, acks and fast-path signalling are not
        let flags = match kind {
            kind::ACK | kind::FILE_CHUNK | kind::FILE_ACCEPT | kind::FILE_OFFER | kind::FILE_DONE | kind::FAST_REQ | kind::FAST_OFFER | kind::FAST_FAIL | kind::ERROR | kind::MEDIA_STATE | kind::MEDIA_ART | kind::ACTIVITY | kind::LAST_ACTIVITY | kind::TAILNET | kind::LAPTOP_INFO | kind::INPUT | kind::LAPTOP_APPS | kind::PHONE_MEDIA | kind::NOTIF => 0,
            _ => FLAG_ACK,
        };
        Self { kind, flags, id: 0, body }
    }
    pub fn wants_ack(&self) -> bool {
        self.flags & FLAG_ACK != 0
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(6 + self.body.len());
        b.push(self.kind);
        b.push(self.flags);
        b.extend_from_slice(&self.id.to_be_bytes());
        b.extend_from_slice(&self.body);
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, String> {
        let mut c = Cur::new(b);
        Ok(Self { kind: c.u8()?, flags: c.u8()?, id: c.u32()?, body: c.rest().to_vec() })
    }

    pub fn text(s: &str) -> Self {
        Self::new(kind::TEXT, s.as_bytes().to_vec())
    }
    pub fn ack(id: u32) -> Self {
        Self::new(kind::ACK, id.to_be_bytes().to_vec())
    }
    pub fn url(url: &str, position_ms: Option<u64>) -> Self {
        Self::new(kind::URL, [&position_ms.unwrap_or(u64::MAX).to_be_bytes()[..], url.as_bytes()].concat())
    }
    /// body: mime_len u8 | mime | bytes (must fit one record: see [CLIPBOARD_MAX])
    pub fn clipboard(mime: &str, data: &[u8]) -> Self {
        let m = &mime.as_bytes()[..mime.len().min(255)];
        Self::new(kind::CLIPBOARD, [&[m.len() as u8][..], m, data].concat())
    }
    pub fn activity(json: &str) -> Self {
        Self::new(kind::ACTIVITY, json.as_bytes().to_vec())
    }
    pub fn ring(on: bool) -> Self {
        Self::new(kind::RING, vec![on as u8])
    }
    pub fn media_cmd(cmd: u8) -> Self {
        Self::new(kind::MEDIA_CMD, vec![cmd])
    }
    pub fn present(cmd: u8) -> Self {
        Self::new(kind::PRESENT, vec![cmd])
    }
    pub fn media_art(jpeg: &[u8]) -> Self {
        Self::new(kind::MEDIA_ART, jpeg[..jpeg.len().min(MEDIA_ART_MAX)].to_vec())
    }
    pub fn pc_state(locked: bool) -> Self {
        Self::new(kind::PC_STATE, vec![locked as u8])
    }
    pub fn json(kind: u8, v: &str) -> Self {
        Self::new(kind, v.as_bytes().to_vec())
    }
}

/// Largest clipboard payload sent inline; bigger text / any image travels as a file named
/// [CLIPBOARD_FILE_PREFIX]* so the receiver puts it on the clipboard instead of in Downloads.
pub const CLIPBOARD_MAX: usize = 60 * 1024;
pub const CLIPBOARD_FILE_PREFIX: &str = "aura-clipboard";

pub fn parse_clipboard(body: &[u8]) -> Result<(String, &[u8]), String> {
    let mut c = Cur::new(body);
    let n = c.u8()? as usize;
    let mime = String::from_utf8_lossy(c.take(n)?).into_owned();
    Ok((mime, c.rest()))
}

pub fn parse_url(body: &[u8]) -> Result<(String, Option<u64>), String> {
    let mut c = Cur::new(body);
    let p = c.u64()?;
    Ok((String::from_utf8_lossy(c.rest()).into_owned(), (p != u64::MAX).then_some(p)))
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MediaState {
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub title: String,
    pub artist: String,
    /// PC master volume 0..=100 and mute (item 15). Appended after the artist only when known, so
    /// older decoders (which ignore trailing bytes) and the original test vector are unaffected.
    pub volume: Option<(u8, bool)>,
}

impl MediaState {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![self.playing as u8];
        b.extend_from_slice(&self.position_ms.to_be_bytes());
        b.extend_from_slice(&self.duration_ms.to_be_bytes());
        for s in [&self.title, &self.artist] {
            let s = &s.as_bytes()[..s.len().min(512)];
            b.extend_from_slice(&(s.len() as u16).to_be_bytes());
            b.extend_from_slice(s);
        }
        if let Some((v, m)) = self.volume {
            b.push(v.min(100));
            b.push(m as u8);
        }
        b
    }
    pub fn decode(body: &[u8]) -> Result<Self, String> {
        let mut c = Cur::new(body);
        let playing = c.u8()? != 0;
        let position_ms = c.u64()?;
        let duration_ms = c.u64()?;
        let n = c.u16()? as usize;
        let title = String::from_utf8_lossy(c.take(n)?).into_owned();
        let n = c.u16()? as usize;
        let artist = String::from_utf8_lossy(c.take(n)?).into_owned();
        let r = c.rest();
        let volume = if r.len() >= 2 { Some((r[0].min(100), r[1] != 0)) } else { None };
        Ok(Self { playing, position_ms, duration_ms, title, artist, volume })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FastOffer {
    pub port: u16,
    pub ticket: [u8; 16],
    pub addrs: Vec<std::net::Ipv4Addr>,
}

impl FastOffer {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = self.port.to_be_bytes().to_vec();
        b.extend_from_slice(&self.ticket);
        let n = self.addrs.len().min(16);
        b.push(n as u8);
        for a in &self.addrs[..n] {
            b.extend_from_slice(&a.octets());
        }
        b
    }
    pub fn decode(body: &[u8]) -> Result<Self, String> {
        let mut c = Cur::new(body);
        let port = c.u16()?;
        let ticket = c.arr::<16>()?;
        let n = c.u8()? as usize;
        let mut addrs = Vec::new();
        for _ in 0..n {
            addrs.push(std::net::Ipv4Addr::from(c.arr::<4>()?));
        }
        Ok(Self { port, ticket, addrs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extras_messages_and_lifetimes() {
        assert_eq!(Message::ring(true).encode(), vec![0x42, 1, 0, 0, 0, 0, 1]);
        assert_eq!(Message::media_cmd(media_cmd::NEXT).body, vec![2]);
        assert_eq!(Message::present(present_cmd::BLACK).body, vec![3]);
        // live controls are acked (a lost one is resent) but must never fire minutes later
        assert!(Message::present(1).wants_ack() && Message::media_cmd(1).wants_ack() && Message::ring(true).wants_ack());
        assert!(!Message::media_art(&[1, 2]).wants_ack());
        assert_eq!(ttl_secs(kind::PRESENT, 300), 8);
        assert_eq!(ttl_secs(kind::MEDIA_CMD, 300), 8);
        assert_eq!(ttl_secs(kind::RING, 300), 60);
        assert_eq!(ttl_secs(kind::TEXT, 300), 300);
        // artwork is capped so it always fits one record
        assert_eq!(Message::media_art(&vec![7u8; MEDIA_ART_MAX + 5]).body.len(), MEDIA_ART_MAX);
        assert_eq!(priority(kind::PRESENT), 1);
        // remote power: acked, front of the queue, expires after 20 s
        assert!(Message::new(kind::PC_POWER, vec![]).wants_ack());
        assert_eq!(ttl_secs(kind::PC_POWER, 300), 20);
        assert_eq!(priority(kind::PC_POWER), 0);
        assert_eq!(kind::name(kind::PC_POWER), "PC_POWER");
        assert_eq!(kind::name(kind::PRESENT), "PRESENT");
    }

    #[test]
    fn roundtrips() {
        let mut m = Message::text("hello aura");
        m.id = 7;
        assert_eq!(Message::decode(&m.encode()).unwrap(), m);
        assert!(m.wants_ack());
        assert!(!Message::ack(1).wants_ack());
        assert!(Message::decode(&[1, 2, 3]).is_err());
        let u = Message::url("https://youtu.be/x", Some(42_000));
        assert_eq!(parse_url(&u.body).unwrap(), ("https://youtu.be/x".into(), Some(42_000)));
        assert_eq!(parse_url(&Message::url("a", None).body).unwrap().1, None);
        let c = Message::clipboard("text/plain", b"hi");
        assert_eq!(parse_clipboard(&c.body).unwrap(), ("text/plain".to_string(), &b"hi"[..]));
        assert!(c.wants_ack());
        assert!(!Message::activity("{}").wants_ack());
        let s = MediaState { playing: true, position_ms: 5, duration_ms: 9, title: "Song".into(), artist: "Me".into(), volume: Some((30, false)) };
        assert_eq!(MediaState::decode(&s.encode()).unwrap(), s);
        let f = FastOffer { port: 4000, ticket: [3; 16], addrs: vec!["192.168.1.5".parse().unwrap()] };
        assert_eq!(FastOffer::decode(&f.encode()).unwrap(), f);
    }

    #[test]
    fn handoff_bodies_are_fixed() {
        // same bytes are asserted in the Kotlin AuraLinkTest (handoff_messages_encode_like_rust)
        assert_eq!(hex::encode(Message::clipboard("text/plain", b"hi").body), "0a746578742f706c61696e6869");
        assert_eq!(hex::encode(Message::url("https://youtu.be/x", Some(42_000)).body), format!("000000000000a410{}", hex::encode("https://youtu.be/x")));
        assert_eq!(Message::activity("{}").kind, 0x23);
    }

    #[test]
    fn lock_and_unlock_jump_the_queue() {
        assert_eq!(priority(kind::LOCK), 0);
        assert_eq!(priority(kind::UNLOCK_REQUEST), 0);
        assert_eq!(priority(kind::TEXT), 1);
        assert_eq!(priority(kind::FILE_CHUNK), 2);
        assert_eq!(priority(0x99), 1);
    }
}
