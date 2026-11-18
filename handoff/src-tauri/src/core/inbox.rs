//! Phone -> PC: turn what the backend reports into "From phone" list entries.

use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use url::Url;

pub const MAX_INBOX: usize = 20;
/// The same share reported twice within this window is one share.
const DEDUPE_WINDOW_MS: u64 = 3_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum InboxKind {
    /// A web link. `url` opens in Brave.
    Url { url: String },
    /// Text shared from the phone. KDE Connect stores it in a temp .txt file.
    Text { text: String, path: Option<PathBuf> },
    /// A file saved by the backend.
    File { path: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InboxItem {
    pub id: u64,
    pub received_ms: u64,
    #[serde(flatten)]
    pub kind: InboxKind,
}

impl InboxItem {
    /// What "Copy" puts on the clipboard.
    pub fn copy_value(&self) -> String {
        match &self.kind {
            InboxKind::Url { url } => url.clone(),
            InboxKind::Text { text, .. } => text.clone(),
            InboxKind::File { path } => path.display().to_string(),
        }
    }
    pub fn label(&self) -> String {
        use super::payload::truncate;
        match &self.kind {
            InboxKind::Url { url } => {
                truncate(url.trim_start_matches("https://").trim_start_matches("http://"), 48)
            }
            InboxKind::Text { text, .. } => {
                format!("\u{201C}{}\u{201D}", truncate(&text.replace(['\r', '\n'], " "), 40))
            }
            InboxKind::File { path } => truncate(
                &path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                48,
            ),
        }
    }
}

/// Is this a KDE Connect "shared text" temp file? On Windows the daemon writes received
/// text to `%TEMP%\kdeconnect-XXXXXX.txt` (QTemporaryFile template) and reports its file URL.
pub fn is_kde_text_temp(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    name.starts_with("kdeconnect-") && name.ends_with(".txt")
}

/// Map a raw share report (KDE Connect `shareReceived(url)`) to an inbox entry.
/// `read_text` reads a temp text file (injected so this stays testable).
pub fn classify_share(raw: &str, read_text: impl Fn(&Path) -> Option<String>) -> Option<InboxKind> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let u = Url::parse(raw).ok();
    match u.as_ref().map(|u| u.scheme()) {
        Some("file") => {
            let path = u.unwrap().to_file_path().ok()?;
            if is_kde_text_temp(&path) {
                let text = read_text(&path).unwrap_or_default();
                Some(InboxKind::Text { text, path: Some(path) })
            } else {
                Some(InboxKind::File { path })
            }
        }
        Some(_) => Some(InboxKind::Url { url: raw.to_string() }),
        // Not a URL at all: treat as text.
        None => Some(InboxKind::Text { text: raw.to_string(), path: None }),
    }
}

#[derive(Debug, Default)]
pub struct Inbox {
    items: VecDeque<InboxItem>,
    next_id: u64,
}

impl Inbox {
    pub fn new() -> Self {
        Self::default()
    }
    /// Newest first.
    pub fn items(&self) -> impl Iterator<Item = &InboxItem> {
        self.items.iter()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn get(&self, id: u64) -> Option<&InboxItem> {
        self.items.iter().find(|i| i.id == id)
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Add an entry; returns it, or `None` if it duplicates one just received.
    pub fn add(&mut self, kind: InboxKind, now_ms: u64) -> Option<InboxItem> {
        if let Some(front) = self.items.front() {
            if front.kind == kind && now_ms.saturating_sub(front.received_ms) < DEDUPE_WINDOW_MS {
                return None;
            }
        }
        self.next_id += 1;
        let item = InboxItem { id: self.next_id, received_ms: now_ms, kind };
        self.items.push_front(item.clone());
        self.items.truncate(MAX_INBOX);
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_read(_: &Path) -> Option<String> {
        None
    }

    #[test]
    fn classifies_url_share() {
        assert_eq!(
            classify_share("https://example.com/a", no_read),
            Some(InboxKind::Url { url: "https://example.com/a".into() })
        );
    }

    #[test]
    fn classifies_file_share() {
        let k = classify_share("file:///C:/Users/me/Downloads/photo%201.jpg", no_read).unwrap();
        assert_eq!(k, InboxKind::File { path: PathBuf::from(r"C:\Users\me\Downloads\photo 1.jpg") });
    }

    #[test]
    fn classifies_text_temp_file_and_reads_it() {
        let k = classify_share("file:///C:/Users/me/AppData/Local/Temp/kdeconnect-AbC123.txt", |p| {
            assert!(p.ends_with("kdeconnect-AbC123.txt"));
            Some("hello from phone".into())
        })
        .unwrap();
        match k {
            InboxKind::Text { text, path } => {
                assert_eq!(text, "hello from phone");
                assert!(path.is_some());
            }
            other => panic!("expected text, got {other:?}"),
        }
        // A normal .txt file the user sent is a file, not shared text.
        assert!(matches!(
            classify_share("file:///C:/d/notes.txt", no_read),
            Some(InboxKind::File { .. })
        ));
    }

    #[test]
    fn non_url_is_text_and_empty_is_none() {
        assert_eq!(
            classify_share("just words", no_read),
            Some(InboxKind::Text { text: "just words".into(), path: None })
        );
        assert_eq!(classify_share("  ", no_read), None);
    }

    #[test]
    fn inbox_orders_newest_first_dedupes_and_caps() {
        let mut ib = Inbox::new();
        let u = |s: &str| InboxKind::Url { url: s.into() };
        assert!(ib.add(u("https://a/"), 0).is_some());
        assert!(ib.add(u("https://a/"), 1_000).is_none(), "duplicate inside window");
        assert!(ib.add(u("https://a/"), 10_000).is_some(), "same link later is a new share");
        assert!(ib.add(u("https://b/"), 10_001).is_some());
        assert_eq!(ib.items().next().unwrap().copy_value(), "https://b/");
        for i in 0..MAX_INBOX + 3 {
            ib.add(u(&format!("https://n{i}/")), 20_000 + i as u64);
        }
        assert_eq!(ib.len(), MAX_INBOX);
        let id = ib.items().next().unwrap().id;
        assert!(ib.get(id).is_some());
    }
}
