//! Screenshot folder watching (polling, no file-system hooks): report each new image once,
//! only after its size stopped changing (Snipping Tool, Syncthing and KDE Connect all write
//! files in more than one step). Also the "Save" naming rule.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
}

const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "heic", "gif", "bmp"];

pub fn is_image(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Syncthing temp files: ".syncthing.NAME.tmp" and "~syncthing~NAME.tmp"
    if lower.starts_with(".syncthing.") || lower.starts_with("~syncthing~") || lower.ends_with(".tmp") || lower.ends_with(".part") {
        return false;
    }
    lower.rsplit_once('.').map(|(_, e)| IMAGE_EXT.contains(&e)).unwrap_or(false)
}

/// Names that look like a screenshot (used for the Downloads folder, where KDE Connect puts
/// everything it receives). Samsung: "Screenshot_20261019_101500_Brave.jpg".
pub fn looks_like_screenshot(name: &str) -> bool {
    let l = name.to_lowercase();
    is_image(name) && (l.starts_with("screenshot") || l.starts_with("screen_shot") || l.starts_with("capture"))
}

#[derive(Debug, Default)]
pub struct FolderWatch {
    primed: bool,
    known: HashSet<String>,
    pending: HashMap<String, u64>,
    /// only report names this filter accepts
    screenshots_only: bool,
}

impl FolderWatch {
    pub fn new(screenshots_only: bool) -> Self {
        Self { screenshots_only, ..Default::default() }
    }

    fn wanted(&self, name: &str) -> bool {
        if self.screenshots_only {
            looks_like_screenshot(name)
        } else {
            is_image(name)
        }
    }

    /// Feed the current listing; returns names that just became new and stable.
    /// The first call only records what is already there.
    pub fn poll(&mut self, listing: &[Entry]) -> Vec<String> {
        let mut out = Vec::new();
        if !self.primed {
            self.primed = true;
            self.known = listing.iter().map(|e| e.name.clone()).collect();
            return out;
        }
        let present: HashSet<&str> = listing.iter().map(|e| e.name.as_str()).collect();
        // forget files that went away, so a re-created name is reported again
        self.known.retain(|n| present.contains(n.as_str()));
        self.pending.retain(|n, _| present.contains(n.as_str()));
        for e in listing {
            if self.known.contains(&e.name) || !self.wanted(&e.name) {
                continue;
            }
            match self.pending.get(&e.name) {
                Some(&prev) if prev == e.size && e.size > 0 => {
                    self.pending.remove(&e.name);
                    self.known.insert(e.name.clone());
                    out.push(e.name.clone());
                }
                _ => {
                    self.pending.insert(e.name.clone(), e.size);
                }
            }
        }
        out.sort();
        out
    }
}

/// "a.png" -> "a.png", or "a (2).png", "a (3).png" ... if taken.
pub fn unique_name(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    (2..10_000).map(|i| format!("{stem} ({i}){ext}")).find(|n| !taken(n)).unwrap_or_else(|| format!("{stem} (copy){ext}"))
}

/// Newest image in a listing that came after `after_ms` (for the snip hotkey: the file
/// Snipping Tool saved after we opened it). `entries` carry modified time in ms.
pub fn newest_after(entries: &[(String, u64, u64)], after_ms: u64) -> Option<String> {
    entries
        .iter()
        .filter(|(n, size, m)| is_image(n) && *size > 0 && *m >= after_ms)
        .max_by_key(|(_, _, m)| *m)
        .map(|(n, _, _)| n.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(n: &str, s: u64) -> Entry {
        Entry { name: n.into(), size: s }
    }

    #[test]
    fn image_and_screenshot_names() {
        assert!(is_image("Screenshot (1).png") && is_image("a.JPG"));
        assert!(!is_image("notes.txt") && !is_image(".syncthing.x.png.tmp") && !is_image("~syncthing~x.png.tmp"));
        assert!(!is_image("noext"));
        assert!(looks_like_screenshot("Screenshot_20261019_101500_Brave.jpg"));
        assert!(!looks_like_screenshot("holiday.jpg"));
    }

    #[test]
    fn first_poll_primes_then_reports_once_stable() {
        let mut w = FolderWatch::new(false);
        assert!(w.poll(&[e("old.png", 10)]).is_empty());
        // new file appears, still growing
        assert!(w.poll(&[e("old.png", 10), e("new.png", 100)]).is_empty());
        assert!(w.poll(&[e("old.png", 10), e("new.png", 200)]).is_empty());
        // same size twice: stable
        assert_eq!(w.poll(&[e("old.png", 10), e("new.png", 200)]), vec!["new.png"]);
        // never again
        assert!(w.poll(&[e("old.png", 10), e("new.png", 200)]).is_empty());
    }

    #[test]
    fn zero_size_and_non_images_ignored() {
        let mut w = FolderWatch::new(false);
        w.poll(&[]);
        w.poll(&[e("z.png", 0), e("doc.pdf", 5)]);
        assert!(w.poll(&[e("z.png", 0), e("doc.pdf", 5)]).is_empty());
    }

    #[test]
    fn deleted_and_recreated_is_reported_again() {
        let mut w = FolderWatch::new(false);
        w.poll(&[e("a.png", 1)]);
        w.poll(&[]);
        w.poll(&[e("a.png", 2)]);
        assert_eq!(w.poll(&[e("a.png", 2)]), vec!["a.png"]);
    }

    #[test]
    fn downloads_mode_only_screenshots() {
        let mut w = FolderWatch::new(true);
        w.poll(&[]);
        let l = [e("Screenshot_1.jpg", 5), e("cat.jpg", 5)];
        w.poll(&l);
        assert_eq!(w.poll(&l), vec!["Screenshot_1.jpg"]);
    }

    #[test]
    fn unique_names() {
        let taken = |n: &str| n == "a.png" || n == "a (2).png";
        assert_eq!(unique_name("b.png", &taken), "b.png");
        assert_eq!(unique_name("a.png", &taken), "a (3).png");
        let t2 = |n: &str| n == "README";
        assert_eq!(unique_name("README", &t2), "README (2)");
    }

    #[test]
    fn newest_after_picks_latest_image() {
        let l = vec![("old.png".to_string(), 5, 100), ("a.png".to_string(), 5, 300), ("b.png".to_string(), 5, 200), ("x.txt".to_string(), 5, 400)];
        assert_eq!(newest_after(&l, 150).as_deref(), Some("a.png"));
        assert_eq!(newest_after(&l, 301), None);
    }
}
