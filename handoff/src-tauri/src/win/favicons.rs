//! Site icons from the browsers' own `Favicons` databases (read-only, on a COPY like History), so the phone's chat head
//! can show real favicons for the laptop's open tabs without the phone fetching anything from the internet.
//! One small PNG (<= 32 px) per host.

use base64::Engine;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SIZE: u32 = 32;

fn favicon_files() -> Vec<PathBuf> {
    super::history::history_files().into_iter().filter_map(|h| h.parent().map(|p| p.join("Favicons"))).filter(|p| p.is_file()).collect()
}

fn copy_to_temp(src: &Path, tag: usize) -> Option<PathBuf> {
    let dst = std::env::temp_dir().join(format!("aura-fav-{}-{tag}.db", std::process::id()));
    for _ in 0..2 {
        if std::fs::copy(src, &dst).is_ok() {
            return Some(dst);
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
    None
}

/// Shrink to at most 32 px and re-encode as PNG (a few hundred bytes to ~2 KB).
fn small_png(raw: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(raw).ok()?;
    let img = if img.width() > SIZE || img.height() > SIZE { img.resize(SIZE, SIZE, image::imageops::FilterType::Triangle) } else { img };
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    (out.len() <= 6_000).then_some(out)
}

fn like_escape(s: &str) -> String {
    s.replace('!', "!!").replace('%', "!%").replace('_', "!_")
}

/// The icon of one page host out of one (copied) Favicons database: the bitmap closest to 32 px of any page on that host.
fn one(db: &rusqlite::Connection, host: &str) -> Option<Vec<u8>> {
    let h = like_escape(host);
    let raw: Vec<u8> = db
        .query_row(
            "SELECT fb.image_data FROM icon_mapping im JOIN favicon_bitmaps fb ON fb.icon_id = im.icon_id \
             WHERE (im.page_url LIKE ?1 ESCAPE '!' OR im.page_url LIKE ?2 ESCAPE '!') AND length(fb.image_data) > 0 \
             ORDER BY ABS(fb.width - 32) LIMIT 1",
            rusqlite::params![format!("https://{h}/%"), format!("http://{h}/%")],
            |r| r.get(0),
        )
        .ok()?;
    small_png(&raw)
}

/// Icons for `hosts` (lower-case host[:port], as the phone derives it from a tab link) -> base64 PNG. Hosts without an icon are absent.
pub fn lookup(hosts: &[String]) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    if hosts.is_empty() {
        return out;
    }
    for (i, f) in favicon_files().iter().enumerate() {
        let Some(tmp) = copy_to_temp(f, i) else { continue };
        if let Ok(db) = rusqlite::Connection::open_with_flags(&tmp, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            let todo: Vec<&String> = hosts.iter().filter(|h| !out.contains_key(*h)).collect();
            for h in todo {
                if let Some(png) = one(&db, h) {
                    out.insert(h.clone(), base64::engine::general_purpose::STANDARD.encode(png));
                }
            }
        }
        let _ = std::fs::remove_file(&tmp);
        if out.len() == hosts.len() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(side: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(side, side, image::Rgba([200, 30, 30, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn finds_an_icon_by_host_and_shrinks_it() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Favicons");
        let db = rusqlite::Connection::open(&p).unwrap();
        db.execute("CREATE TABLE icon_mapping (id INTEGER PRIMARY KEY, page_url TEXT, icon_id INTEGER)", []).unwrap();
        db.execute("CREATE TABLE favicon_bitmaps (id INTEGER PRIMARY KEY, icon_id INTEGER, image_data BLOB, width INTEGER, height INTEGER)", []).unwrap();
        db.execute("INSERT INTO icon_mapping (page_url, icon_id) VALUES ('https://www.example.com/a/b', 1)", []).unwrap();
        db.execute("INSERT INTO favicon_bitmaps (icon_id, image_data, width, height) VALUES (1, ?1, 64, 64)", [png(64)]).unwrap();
        db.execute("INSERT INTO favicon_bitmaps (icon_id, image_data, width, height) VALUES (1, ?1, 16, 16)", [png(16)]).unwrap();
        let got = one(&db, "www.example.com").expect("icon");
        let img = image::load_from_memory(&got).unwrap();
        assert!(img.width() <= SIZE, "closest to 32 px wins (16), and nothing is bigger than 32");
        assert!(one(&db, "other.com").is_none());
        assert!(one(&db, "www_example.com").is_none(), "LIKE wildcards in a host never match other hosts");
    }
}
