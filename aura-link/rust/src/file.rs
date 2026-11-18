//! Resumable file transfer (idea from LocalSend's prepare/accept/upload and Syncthing's
//! "resume from what is on disk, verify the hash at the end"; no code copied).
//!
//! sender   FILE_OFFER  id(16) | size u64 | sha256(32) | u16 len name | u8 len mime | mime
//! receiver FILE_ACCEPT id | offset u64        offset = bytes already in its .part file
//! sender   FILE_CHUNK  id | offset u64 | data (from offset, in order, over RFCOMM or the fast path)
//! receiver FILE_DONE   id | status u8          0 ok (sha256 matched, file renamed into place),
//!                                              1 hash mismatch (part deleted; sender re-offers from 0),
//!                                              2 refused
//! After a dropped session the sender re-sends FILE_OFFER on the next one and continues from the
//! receiver's offset. The .part file survives a restart of either side.

use crate::wire::Cur;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const DONE_OK: u8 = 0;
pub const DONE_BAD_HASH: u8 = 1;
pub const DONE_REFUSED: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOffer {
    pub id: [u8; 16],
    pub size: u64,
    pub sha256: [u8; 32],
    pub name: String,
    pub mime: String,
}

impl FileOffer {
    pub fn encode(&self) -> Vec<u8> {
        let name = &self.name.as_bytes()[..self.name.len().min(1024)];
        let mime = &self.mime.as_bytes()[..self.mime.len().min(255)];
        let mut b = self.id.to_vec();
        b.extend_from_slice(&self.size.to_be_bytes());
        b.extend_from_slice(&self.sha256);
        b.extend_from_slice(&(name.len() as u16).to_be_bytes());
        b.extend_from_slice(name);
        b.push(mime.len() as u8);
        b.extend_from_slice(mime);
        b
    }
    pub fn decode(body: &[u8]) -> Result<Self, String> {
        let mut c = Cur::new(body);
        let id = c.arr::<16>()?;
        let size = c.u64()?;
        let sha256 = c.arr::<32>()?;
        let n = c.u16()? as usize;
        let name = String::from_utf8_lossy(c.take(n)?).into_owned();
        let n = c.u8()? as usize;
        let mime = String::from_utf8_lossy(c.take(n)?).into_owned();
        Ok(Self { id, size, sha256, name, mime })
    }
}

pub fn id_offset_body(id: &[u8; 16], offset: u64) -> Vec<u8> {
    [&id[..], &offset.to_be_bytes()].concat()
}

pub fn parse_id_offset(body: &[u8]) -> Result<([u8; 16], u64, &[u8]), String> {
    let mut c = Cur::new(body);
    let id = c.arr::<16>()?;
    let off = c.u64()?;
    Ok((id, off, c.rest()))
}

pub fn done_body(id: &[u8; 16], status: u8) -> Vec<u8> {
    [&id[..], &[status]].concat()
}

pub fn parse_done(body: &[u8]) -> Result<([u8; 16], u8), String> {
    let mut c = Cur::new(body);
    Ok((c.arr::<16>()?, c.u8()?))
}

pub fn sha256_file(p: &Path) -> std::io::Result<[u8; 32]> {
    let mut f = File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

pub fn guess_mime(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Sender side of one file.
pub struct SendJob {
    pub offer: FileOffer,
    pub path: PathBuf,
    /// next byte to send; set by FILE_ACCEPT
    pub next: u64,
    pub accepted: bool,
    /// None = in progress
    pub done: Option<Result<(), String>>,
    pub prefer_fast: Option<bool>,
    pub created: Instant,
    pub first_chunk: Option<Instant>,
    pub finished: Option<Instant>,
    pub bytes_fast: u64,
    pub bytes_bt: u64,
    file: Option<File>,
}

impl SendJob {
    pub fn new(path: &Path, prefer_fast: Option<bool>) -> std::io::Result<Self> {
        let size = std::fs::metadata(path)?.len();
        let sha256 = sha256_file(path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        let mime = guess_mime(&name).to_string();
        Ok(Self {
            offer: FileOffer { id: crate::random(), size, sha256, name, mime },
            path: path.to_path_buf(),
            next: 0,
            accepted: false,
            done: None,
            prefer_fast,
            created: Instant::now(),
            first_chunk: None,
            finished: None,
            bytes_fast: 0,
            bytes_bt: 0,
            file: None,
        })
    }

    pub fn remaining(&self) -> u64 {
        self.offer.size.saturating_sub(self.next)
    }

    /// Next chunk (offset, data) of at most `max` bytes, or None when everything was sent.
    pub fn next_chunk(&mut self, max: usize) -> std::io::Result<Option<(u64, Vec<u8>)>> {
        if !self.accepted || self.done.is_some() || self.next >= self.offer.size {
            return Ok(None);
        }
        if self.file.is_none() {
            self.file = Some(File::open(&self.path)?);
        }
        let n = (self.remaining() as usize).min(max);
        let f = self.file.as_mut().unwrap();
        f.seek(SeekFrom::Start(self.next))?;
        let mut buf = vec![0u8; n];
        f.read_exact(&mut buf)?;
        let off = self.next;
        self.next += n as u64;
        self.first_chunk.get_or_insert_with(Instant::now);
        Ok(Some((off, buf)))
    }
}

/// Receiver side of one file.
pub struct RecvJob {
    pub offer: FileOffer,
    pub part: PathBuf,
    pub received: u64,
    pub started: Instant,
    file: File,
}

pub fn sanitize(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let s: String = base.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() {
        "file".into()
    } else {
        s.chars().take(200).collect()
    }
}

pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    if !p.exists() {
        return p;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for i in 1.. {
        let p = dir.join(format!("{stem} ({i}){ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!()
}

pub struct Inbox {
    pub dir: PathBuf,
}

impl Inbox {
    pub fn part_path(&self, id: &[u8; 16]) -> PathBuf {
        self.dir.join(".aura-partial").join(format!("{}.part", hex::encode(id)))
    }

    /// Opens (or resumes) the .part file for an offer. `received` = bytes already there.
    pub fn open(&self, offer: &FileOffer) -> std::io::Result<RecvJob> {
        let part = self.part_path(&offer.id);
        std::fs::create_dir_all(part.parent().unwrap())?;
        let file = OpenOptions::new().create(true).read(true).write(true).truncate(false).open(&part)?;
        let mut len = file.metadata()?.len();
        if len > offer.size {
            file.set_len(0)?;
            len = 0;
        }
        Ok(RecvJob { offer: offer.clone(), part, received: len, started: Instant::now(), file })
    }

    /// Appends a chunk if it is the next one. Ok(true) when the file is complete.
    pub fn write(&self, job: &mut RecvJob, offset: u64, data: &[u8]) -> Result<bool, String> {
        if offset != job.received {
            return Err(format!("chunk at {offset}, expected {}", job.received));
        }
        if job.received + data.len() as u64 > job.offer.size {
            return Err("chunk past the end of the file".into());
        }
        job.file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        job.file.write_all(data).map_err(|e| e.to_string())?;
        job.received += data.len() as u64;
        Ok(job.received == job.offer.size)
    }

    /// Hash check, then move into the inbox under a safe, unique name.
    pub fn finish(&self, job: RecvJob) -> Result<PathBuf, String> {
        let RecvJob { offer, part, file, .. } = job;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        let got = sha256_file(&part).map_err(|e| e.to_string())?;
        if got != offer.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err("sha256 mismatch".into());
        }
        let dest = unique_path(&self.dir, &sanitize(&offer.name));
        std::fs::rename(&part, &dest).map_err(|e| format!("move into inbox: {e}"))?;
        Ok(dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aura-link-test-{tag}-{}-{}", std::process::id(), hex::encode(crate::random::<4>())));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn offer_roundtrip_and_sanitize() {
        let o = FileOffer { id: [1; 16], size: 9, sha256: [2; 32], name: "a b.txt".into(), mime: "text/plain".into() };
        assert_eq!(FileOffer::decode(&o.encode()).unwrap(), o);
        assert_eq!(sanitize("..\\..\\evil:name?.exe"), "evil_name_.exe");
        assert_eq!(sanitize("../.."), "file");
        assert_eq!(sanitize("/etc/passwd"), "passwd");
        assert_eq!(sanitize(""), "file");
    }

    #[test]
    fn resume_from_partial_and_hash_check() {
        let d = tmp("file");
        let src = d.join("src.bin");
        let data: Vec<u8> = (0..100_000u32).map(|i| (i * 7) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let mut s = SendJob::new(&src, None).unwrap();
        let inbox = Inbox { dir: d.join("in") };
        let mut r = inbox.open(&s.offer).unwrap();
        s.accepted = true;
        let (o, c) = s.next_chunk(30_000).unwrap().unwrap();
        assert!(!inbox.write(&mut r, o, &c).unwrap());
        drop(r);
        // "reconnect": the receiver reopens and reports what it has
        let mut r = inbox.open(&s.offer).unwrap();
        assert_eq!(r.received, 30_000);
        s.next = r.received;
        assert!(inbox.write(&mut r, 0, b"x").is_err(), "out of order chunk refused");
        loop {
            let (o, c) = s.next_chunk(30_000).unwrap().unwrap();
            if inbox.write(&mut r, o, &c).unwrap() {
                break;
            }
        }
        let p = inbox.finish(r).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), data);
        assert_eq!(p.file_name().unwrap(), "src.bin");
        // a second copy gets a unique name, a corrupted one is refused
        let mut bad = s.offer.clone();
        bad.id = [9; 16];
        bad.sha256 = [0; 32];
        let mut r = inbox.open(&bad).unwrap();
        assert!(inbox.write(&mut r, 0, &data).unwrap());
        assert!(inbox.finish(r).is_err());
        assert!(!inbox.part_path(&bad.id).exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
