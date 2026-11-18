//! Ctrl+Alt+S: wait for the picture Snipping Tool produces. Pure decision logic; the Win32
//! side (launching ms-screenclip, listing the folder, reading the clipboard) is in `win::snip`.
//!
//! The file watcher is the suite's `continuity_core::shots::FolderWatch` (reused by path,
//! not copied): it reports a new image once its size stopped changing.

use continuity_core::shots::{Entry, FolderWatch};

/// How long we wait for the user to finish drawing the snip rectangle.
pub const SNIP_TIMEOUT_MS: u64 = 90_000;
/// A clipboard image arrives first; wait this long for Snipping Tool's own file (the better
/// payload: real file name, already PNG) before falling back to the clipboard copy.
pub const CLIP_GRACE_MS: u64 = 3_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    /// A new image file in the Screenshots folder (name only).
    File(String),
    /// Only a clipboard image appeared; the caller saves it to a PNG and sends that.
    Clipboard,
    TimedOut,
}

pub struct SnipWait {
    started_ms: u64,
    clip_seen_ms: Option<u64>,
    watch: FolderWatch,
}

impl SnipWait {
    /// `before` is the folder listing taken right before Snipping Tool was launched.
    pub fn new(now_ms: u64, before: &[Entry]) -> Self {
        let mut watch = FolderWatch::new(false);
        watch.poll(before); // primes: what is already there is not a new snip
        Self { started_ms: now_ms, clip_seen_ms: None, watch }
    }

    /// Call every ~500 ms. `clip_image_new` is true when the clipboard changed to an image
    /// since the wait began.
    pub fn step(&mut self, now_ms: u64, listing: &[Entry], clip_image_new: bool) -> Step {
        if let Some(name) = self.watch.poll(listing).into_iter().next() {
            return Step::File(name);
        }
        if clip_image_new && self.clip_seen_ms.is_none() {
            self.clip_seen_ms = Some(now_ms);
        }
        if let Some(t) = self.clip_seen_ms {
            if now_ms.saturating_sub(t) >= CLIP_GRACE_MS {
                return Step::Clipboard;
            }
        }
        if now_ms.saturating_sub(self.started_ms) >= SNIP_TIMEOUT_MS {
            return Step::TimedOut;
        }
        Step::Wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(n: &str, s: u64) -> Entry {
        Entry { name: n.into(), size: s }
    }

    #[test]
    fn new_file_is_reported_once_stable() {
        let mut w = SnipWait::new(0, &[e("old.png", 5)]);
        assert_eq!(w.step(500, &[e("old.png", 5)], false), Step::Wait);
        assert_eq!(w.step(1000, &[e("old.png", 5), e("Screenshot 1.png", 100)], false), Step::Wait);
        assert_eq!(w.step(1500, &[e("old.png", 5), e("Screenshot 1.png", 900)], false), Step::Wait);
        assert_eq!(
            w.step(2000, &[e("old.png", 5), e("Screenshot 1.png", 900)], false),
            Step::File("Screenshot 1.png".into())
        );
    }

    #[test]
    fn clipboard_only_after_grace() {
        let mut w = SnipWait::new(0, &[]);
        assert_eq!(w.step(1000, &[], true), Step::Wait);
        assert_eq!(w.step(3999, &[], false), Step::Wait);
        assert_eq!(w.step(4000, &[], false), Step::Clipboard);
    }

    #[test]
    fn file_beats_clipboard() {
        let mut w = SnipWait::new(0, &[]);
        assert_eq!(w.step(1000, &[e("a.png", 10)], true), Step::Wait);
        assert_eq!(w.step(1500, &[e("a.png", 10)], true), Step::File("a.png".into()));
    }

    #[test]
    fn cancelled_snip_times_out() {
        let mut w = SnipWait::new(10, &[e("old.png", 1)]);
        assert_eq!(w.step(10 + SNIP_TIMEOUT_MS - 1, &[e("old.png", 1)], false), Step::Wait);
        assert_eq!(w.step(10 + SNIP_TIMEOUT_MS, &[e("old.png", 1)], false), Step::TimedOut);
    }
}
