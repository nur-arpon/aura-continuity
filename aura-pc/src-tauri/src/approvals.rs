//! "Last approvals" for the Unlock page: the sign-in tile's recent requests, read from the helper
//! log (`ContinuityUnlock\helper.log`, lines like `22:23:46.460Z request <32 hex>: no_device in 8 ms`).
//! The log carries only the UTC time of day, so the date is rebuilt backwards from the file's
//! modification time (the last line is "now" for the file).

use serde::Serialize;

const DAY_MS: u64 = 86_400_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    /// Unix milliseconds (UTC); the window formats it in local time.
    pub at_ms: u64,
    pub ok: bool,
    /// Plain-language result.
    pub result: String,
    pub took_ms: Option<u64>,
}

fn tod_ms(line: &str) -> Option<u64> {
    let b = line.as_bytes();
    if b.len() < 14 || b[2] != b':' || b[5] != b':' || b[8] != b'.' || b[12] != b'Z' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| line.get(r)?.parse::<u64>().ok();
    let (h, m, s, ms) = (n(0..2)?, n(3..5)?, n(6..8)?, n(9..12)?);
    if h > 23 || m > 59 || s > 59 {
        return None;
    }
    Some(((h * 60 + m) * 60 + s) * 1000 + ms)
}

fn describe(reason: &str) -> (bool, String) {
    match reason {
        "APPROVED" => (true, "Approved".into()),
        "timeout" => (false, "No answer in time - PIN used".into()),
        "no_device" => (false, "No phone or watch reached".into()),
        "denied" => (false, "Denied on the phone".into()),
        "not_now" => (false, "Phone said not now - PIN used".into()),
        "not_paired" => (false, "Not paired yet".into()),
        "approval_already_used" => (false, "Approval already used".into()),
        "replayed_nonce" => (false, "Repeated request refused".into()),
        other => (false, other.replace('_', " ")),
    }
}

/// Newest first, at most `max`. `mtime_ms` = modification time of the log (Unix ms).
pub fn parse(log: &str, mtime_ms: u64, max: usize) -> Vec<Approval> {
    let lines: Vec<(u64, &str)> = log.lines().filter_map(|l| tod_ms(l).map(|t| (t, &l[14..]))).collect();
    let Some(&(last_tod, _)) = lines.last() else { return vec![] };
    // The last line is as old as the file; walk backwards, a drop in the time of day = a day boundary.
    let mut at = vec![0u64; lines.len()];
    let n = lines.len();
    at[n - 1] = mtime_ms.saturating_sub((mtime_ms % DAY_MS + DAY_MS - last_tod) % DAY_MS);
    for i in (0..n - 1).rev() {
        let d = (lines[i + 1].0 + DAY_MS - lines[i].0) % DAY_MS;
        // writers can interleave a few ms out of order: a "negative" step is a tiny move forward
        at[i] = if d > DAY_MS / 2 { at[i + 1] + (DAY_MS - d) } else { at[i + 1].saturating_sub(d) };
    }
    let mut out = Vec::new();
    for (i, (_, rest)) in lines.iter().enumerate().rev() {
        let Some(r) = rest.strip_prefix("request ") else { continue };
        let Some((_, tail)) = r.split_once(": ") else { continue };
        let (reason, took) = match tail.split_once(" in ") {
            Some((a, b)) => (a.trim(), b.trim().trim_end_matches(" ms").parse::<u64>().ok()),
            None => (tail.trim(), None),
        };
        let (ok, result) = describe(reason);
        out.push(Approval { at_ms: at[i], ok, result, took_ms: took });
        if out.len() >= max {
            break;
        }
    }
    out
}

/// Read the tail of the helper log (the file is capped at 1 MB; 128 KB is plenty for the last requests).
pub fn recent(max: usize) -> Vec<Approval> {
    use std::io::{Read, Seek, SeekFrom};
    let path = unlock_pc::pairing::store::dir().join("helper.log");
    let Ok(mut f) = std::fs::File::open(&path) else { return vec![] };
    let mtime = f
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(128 * 1024)));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    parse(&String::from_utf8_lossy(&buf), mtime, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
22:23:46.459Z race winner=- total=7ms health=\"x\"
22:23:46.460Z request 99033b4696d2d39b628aa9322e158ac2: no_device in 8 ms
22:23:49.473Z request 48224a3d520d108f3689b7de042ac661: timeout in 1811 ms
this line has no time stamp
23:59:58.000Z request aaaa: APPROVED in 120 ms
00:00:03.500Z request bbbb: denied in 900 ms
00:00:04.000Z decision NotApproved(\"denied\") after 900 ms
";

    #[test]
    fn newest_first_with_dates_rebuilt_across_midnight() {
        // the file was last written at 2026-10-22 00:00:04 UTC
        let mtime = 1_792_627_204_000u64; // 2026-10-22T00:00:04Z
        assert_eq!(mtime % DAY_MS, 4_000);
        let a = parse(LOG, mtime, 10);
        assert_eq!(a.len(), 4);
        assert_eq!(a[0].result, "Denied on the phone");
        assert_eq!(a[0].took_ms, Some(900));
        assert_eq!(a[0].at_ms, mtime - 500, "00:00:03.500 is 500 ms before the last line");
        assert!(a[1].ok && a[1].result == "Approved");
        assert_eq!(a[1].at_ms, mtime - 6_000, "23:59:58 is the day before");
        assert!(a[1].at_ms / DAY_MS < a[0].at_ms / DAY_MS, "different UTC days");
        assert_eq!(a[2].result, "No answer in time - PIN used");
        assert_eq!(a[3].result, "No phone or watch reached");
        assert_eq!(parse(LOG, mtime, 2).len(), 2);
    }

    #[test]
    fn junk_and_empty() {
        assert!(parse("", 5, 5).is_empty());
        assert!(parse("garbage\nmore", 5, 5).is_empty());
        assert!(tod_ms("25:00:00.000Z x").is_none());
        assert_eq!(describe("something_new").1, "something new");
        // a few ms out of order (two writers) must not jump a day
        let l = "10:00:00.100Z request a: timeout in 1 ms\n10:00:00.090Z request b: timeout in 1 ms\n";
        let a = parse(l, 36_000_000 + 100, 5);
        assert!(a[0].at_ms.abs_diff(a[1].at_ms) < 1000);
    }
}
