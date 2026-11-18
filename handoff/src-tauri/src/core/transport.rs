//! The seam between "what to send" and "how it travels".
//!
//! v1 has one backend (KDE Connect over LAN, via kdeconnect-cli + its D-Bus).
//! A Bluetooth backend (RFCOMM / BLE GATT to a companion Android app) implements the
//! same two traits and is selected in settings - nothing above this file changes.

use super::payload::Payload;
use serde::Serialize;

/// A phone we can talk to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// The phone is not reachable right now - worth queueing and retrying.
    #[error("phone not reachable: {0}")]
    Unreachable(String),
    /// Something is wrong with the item or the backend - retrying will not help.
    #[error("send failed: {0}")]
    Failed(String),
}

impl SendError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, SendError::Unreachable(_))
    }
}

/// Outgoing direction. Implementations must be cheap to call from a worker thread and
/// must never block for long (use timeouts on child processes / sockets).
pub trait Transport: Send + Sync {
    /// Short backend name for logs and the settings window, e.g. "KDE Connect".
    fn name(&self) -> &'static str;
    /// The paired phone if it is reachable right now, `Ok(None)` if it is not.
    fn reachable_peer(&self) -> Result<Option<Peer>, String>;
    /// All paired phones (reachable or not) - for the device picker.
    fn known_peers(&self) -> Result<Vec<Peer>, String>;
    /// Deliver one payload to `peer`.
    fn send(&self, peer: &Peer, payload: &Payload) -> Result<(), SendError>;
}

/// Events an incoming-direction backend reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IncomingEvent {
    /// The phone shared something with the PC. The string is what the backend reported
    /// (for KDE Connect: an http(s) URL, or a file:/// URL of a received file / text).
    Shared(String),
    /// The phone shared plain text (KDE Connect 26.08 on Windows puts shared text straight
    /// on the PC clipboard instead of emitting `shareReceived`).
    SharedText(String),
    /// The phone's reachability changed (lets the queue retry immediately).
    ReachableChanged(bool),
    /// The incoming channel itself came up (true) or dropped (false).
    Connected(bool),
    /// Aura Link: the phone sent a link to open now ("Continue on PC"), with the playback position.
    OpenUrl { url: String, pos_ms: Option<u64> },
    /// Aura Link: phone clipboard (text, or an image file the service saved in its clip folder).
    ClipboardText(String),
    ClipboardImage(std::path::PathBuf),
    /// Aura Link: a file the phone sent, already saved in Downloads\Aura.
    ReceivedFile(std::path::PathBuf),
    /// Aura Link: what the phone is showing now (passive "Continue on PC" suggestion), raw JSON.
    Activity(serde_json::Value),
}

/// Incoming direction. `run` blocks on its own thread and calls `sink` for each event;
/// it should reconnect on its own if the underlying service restarts.
pub trait IncomingSource: Send {
    fn name(&self) -> &'static str;
    fn run(self: Box<Self>, sink: Box<dyn Fn(IncomingEvent) + Send>);
}
