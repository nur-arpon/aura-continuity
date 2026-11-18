//! Searchable clipboard history with pins. The sync itself is KDE Connect's; this only keeps
//! a history on the PC and marks what came from the phone (clipboard owner `kdeconnectd.exe`).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Pc,
    Phone,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    Text { text: String },
    /// PNG stored in the history folder; `hash` identifies the pixels for de-duplication.
    Image { file: String, width: u32, height: u32, hash: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub content: Content,
    pub origin: Origin,
    pub at_ms: u64,
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    /// newest first
    pub items: Vec<Item>,
    next_id: u64,
    cap: usize,
}

pub const MAX_TEXT: usize = 100_000;

impl History {
    pub fn new(cap: usize) -> Self {
        Self { items: Vec::new(), next_id: 1, cap: cap.max(1) }
    }

    pub fn set_cap(&mut self, cap: usize) -> Vec<String> {
        self.cap = cap.max(1);
        self.trim()
    }

    fn same(a: &Content, b: &Content) -> bool {
        match (a, b) {
            (Content::Text { text: x }, Content::Text { text: y }) => x == y,
            (Content::Image { hash: x, .. }, Content::Image { hash: y, .. }) => x == y,
            _ => false,
        }
    }

    /// Add a copy. Returns (id, image files that fell off the end and can be deleted).
    /// Empty / whitespace-only text is ignored (id 0). Text over MAX_TEXT is cut.
    pub fn add(&mut self, mut content: Content, origin: Origin, now_ms: u64) -> (u64, Vec<String>) {
        if let Content::Text { text } = &mut content {
            if text.trim().is_empty() {
                return (0, vec![]);
            }
            if text.len() > MAX_TEXT {
                let mut cut = MAX_TEXT;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
            }
        }
        let mut dropped = Vec::new();
        if let Some(pos) = self.items.iter().position(|i| Self::same(&i.content, &content)) {
            let mut it = self.items.remove(pos);
            it.at_ms = now_ms;
            // "came from the phone" wins: the PC copy is usually KDE Connect pasting it
            if origin == Origin::Phone {
                it.origin = Origin::Phone;
            }
            // a re-copied image keeps the stored file; the new one is redundant
            if let (Content::Image { file: new, .. }, Content::Image { file: old, .. }) = (&content, &it.content) {
                if new != old {
                    dropped.push(new.clone());
                }
            }
            let id = it.id;
            self.items.insert(0, it);
            return (id, dropped);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.insert(0, Item { id, content, origin, at_ms: now_ms, pinned: false });
        dropped.extend(self.trim());
        (id, dropped)
    }

    /// Drop the oldest unpinned items beyond the cap; returns image files to delete.
    fn trim(&mut self) -> Vec<String> {
        let mut files = Vec::new();
        while self.items.iter().filter(|i| !i.pinned).count() > self.cap {
            if let Some(pos) = self.items.iter().rposition(|i| !i.pinned) {
                if let Content::Image { file, .. } = self.items.remove(pos).content {
                    files.push(file);
                }
            }
        }
        files
    }

    pub fn pin(&mut self, id: u64, on: bool) -> bool {
        match self.items.iter_mut().find(|i| i.id == id) {
            Some(i) => {
                i.pinned = on;
                true
            }
            None => false,
        }
    }

    pub fn remove(&mut self, id: u64) -> Option<Item> {
        let pos = self.items.iter().position(|i| i.id == id)?;
        Some(self.items.remove(pos))
    }

    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// Search: every whitespace-separated term must match (case-insensitive substring).
    /// Special terms: `from:phone`, `from:pc`, `is:pinned`, `is:image`, `is:text`.
    /// Pinned items come first, then newest first.
    pub fn search(&self, query: &str) -> Vec<&Item> {
        let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
        let mut out: Vec<&Item> = self
            .items
            .iter()
            .filter(|it| {
                terms.iter().all(|t| match t.as_str() {
                    "from:phone" => it.origin == Origin::Phone,
                    "from:pc" => it.origin == Origin::Pc,
                    "is:pinned" => it.pinned,
                    "is:image" => matches!(it.content, Content::Image { .. }),
                    "is:text" => matches!(it.content, Content::Text { .. }),
                    t => match &it.content {
                        Content::Text { text } => text.to_lowercase().contains(t),
                        Content::Image { .. } => "image picture screenshot".contains(t),
                    },
                })
            })
            .collect();
        out.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.at_ms.cmp(&a.at_ms)));
        out
    }
}

/// FNV-1a over raw pixels: cheap, stable, good enough to spot "same image copied again".
pub fn hash_pixels(width: u32, height: u32, rgba: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in width.to_le_bytes().iter().chain(height.to_le_bytes().iter()).chain(rgba.iter()) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Content {
        Content::Text { text: s.into() }
    }

    #[test]
    fn adds_newest_first_and_dedupes() {
        let mut h = History::new(10);
        let (a, _) = h.add(t("alpha"), Origin::Pc, 1);
        h.add(t("beta"), Origin::Pc, 2);
        let (a2, _) = h.add(t("alpha"), Origin::Phone, 3);
        assert_eq!(a, a2);
        assert_eq!(h.items.len(), 2);
        assert_eq!(h.items[0].id, a);
        assert_eq!(h.items[0].origin, Origin::Phone);
        assert_eq!(h.items[0].at_ms, 3);
        // phone origin is not downgraded by the PC echo
        h.add(t("alpha"), Origin::Pc, 4);
        assert_eq!(h.items[0].origin, Origin::Phone);
    }

    #[test]
    fn blank_ignored_and_long_cut_on_char_boundary() {
        let mut h = History::new(10);
        assert_eq!(h.add(t("  \n"), Origin::Pc, 1).0, 0);
        assert!(h.items.is_empty());
        let long = "é".repeat(MAX_TEXT); // 2 bytes each
        h.add(Content::Text { text: long }, Origin::Pc, 2);
        let Content::Text { text } = &h.items[0].content else { panic!() };
        assert!(text.len() <= MAX_TEXT);
    }

    #[test]
    fn cap_keeps_pins_and_returns_image_files() {
        let mut h = History::new(2);
        let (p, _) = h.add(t("keep me"), Origin::Pc, 1);
        h.pin(p, true);
        h.add(Content::Image { file: "1.png".into(), width: 1, height: 1, hash: 1 }, Origin::Pc, 2);
        h.add(t("b"), Origin::Pc, 3);
        let (_, dropped) = h.add(t("c"), Origin::Pc, 4);
        assert_eq!(dropped, vec!["1.png".to_string()]);
        assert_eq!(h.items.len(), 3);
        assert!(h.get(p).is_some());
        assert_eq!(h.set_cap(1).len(), 0);
        assert_eq!(h.items.len(), 2);
    }

    #[test]
    fn same_image_again_drops_new_file() {
        let mut h = History::new(5);
        h.add(Content::Image { file: "a.png".into(), width: 2, height: 2, hash: 9 }, Origin::Pc, 1);
        let (_, d) = h.add(Content::Image { file: "b.png".into(), width: 2, height: 2, hash: 9 }, Origin::Pc, 2);
        assert_eq!(d, vec!["b.png".to_string()]);
        assert_eq!(h.items.len(), 1);
    }

    #[test]
    fn search_terms_filters_and_pin_order() {
        let mut h = History::new(10);
        h.add(t("Meeting link https://meet.example"), Origin::Phone, 1);
        let (b, _) = h.add(t("grocery list: milk"), Origin::Pc, 2);
        h.add(t("Another MEETING note"), Origin::Pc, 3);
        h.add(Content::Image { file: "x.png".into(), width: 1, height: 1, hash: 5 }, Origin::Phone, 4);
        assert_eq!(h.search("meeting").len(), 2);
        assert_eq!(h.search("meeting from:phone").len(), 1);
        assert_eq!(h.search("is:image").len(), 1);
        assert_eq!(h.search("screenshot").len(), 1);
        assert_eq!(h.search("").len(), 4);
        h.pin(b, true);
        assert_eq!(h.search("")[0].id, b);
        assert_eq!(h.search("is:pinned").len(), 1);
        assert!(h.search("nothing-matches").is_empty());
    }

    #[test]
    fn remove_and_persist_roundtrip() {
        let mut h = History::new(10);
        let (a, _) = h.add(t("x"), Origin::Pc, 1);
        let j = serde_json::to_string(&h).unwrap();
        let back: History = serde_json::from_str(&j).unwrap();
        assert_eq!(back, h);
        assert!(h.remove(a).is_some() && h.remove(a).is_none());
        assert!(!h.pin(a, true));
    }

    #[test]
    fn pixel_hash_differs() {
        assert_eq!(hash_pixels(1, 1, &[1, 2, 3, 4]), hash_pixels(1, 1, &[1, 2, 3, 4]));
        assert_ne!(hash_pixels(1, 1, &[1, 2, 3, 4]), hash_pixels(1, 1, &[1, 2, 3, 5]));
        assert_ne!(hash_pixels(2, 1, &[0; 8]), hash_pixels(1, 2, &[0; 8]));
    }
}
