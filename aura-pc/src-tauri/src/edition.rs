//! Which edition this build is. The default build is the "direct" (full) edition. `--features store` builds the
//! Microsoft Store edition: no sign-in-screen unlock (tile / SYSTEM helper) and no add-ons (Apollo streaming,
//! scrcpy phone mirroring), because Store policy rules out the downloader and the SYSTEM-level pieces. Features
//! that need an add-on answer with [`FULL_EDITION_NOTE`] instead of failing in a confusing way.

pub const STORE: bool = cfg!(feature = "store");

pub const FULL_EDITION_NOTE: &str = "Get the full edition: this needs add-ons (sign-in screen unlock, phone mirroring, laptop streaming) that the Microsoft Store edition does not include. Download it from the Aura website.";

/// Err(note) in the Store edition, Ok otherwise. For commands that only the full edition can run.
pub fn require_full() -> Result<(), String> {
    if STORE { Err(FULL_EDITION_NOTE.to_string()) } else { Ok(()) }
}
