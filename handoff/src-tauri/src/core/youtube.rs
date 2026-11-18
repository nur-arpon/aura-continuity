//! YouTube "continue where I was" handling: put the current playback time on the link
//! as `t=<seconds>s` so the phone's YouTube app starts there.

use super::classify::{classify_url, UrlKind};
use url::Url;

/// Parse YouTube's `t` parameter: "90", "90s", "1m30s", "1h2m3s".
pub fn parse_t_param(v: &str) -> Option<u64> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    if let Ok(n) = v.parse::<u64>() {
        return Some(n);
    }
    let mut total = 0u64;
    let mut num = String::new();
    let mut saw_unit = false;
    for c in v.chars() {
        if c.is_ascii_digit() {
            num.push(c);
        } else {
            let n: u64 = num.parse().ok()?;
            num.clear();
            total += match c {
                'h' | 'H' => n * 3600,
                'm' | 'M' => n * 60,
                's' | 'S' => n,
                _ => return None,
            };
            saw_unit = true;
        }
    }
    if !num.is_empty() {
        return None; // trailing digits without a unit, e.g. "1m30"
    }
    saw_unit.then_some(total)
}

/// Parse a player clock like "4:05", "1:02:03", or the "4:05 / 10:00" pair (takes the first).
pub fn parse_clock(s: &str) -> Option<u64> {
    let first = s.split('/').next()?.trim();
    if first.is_empty() {
        return None;
    }
    let parts: Vec<&str> = first.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut total = 0u64;
    for (i, p) in parts.iter().enumerate() {
        let n: u64 = p.trim().parse().ok()?;
        if i > 0 && n >= 60 {
            return None;
        }
        total = total * 60 + n;
    }
    Some(total)
}

/// Existing `t` on a YouTube link, if any.
pub fn existing_timestamp(raw: &str) -> Option<u64> {
    let u = Url::parse(raw).ok()?;
    let t = u.query_pairs().find(|(k, _)| k == "t" || k == "start")?.1.into_owned();
    parse_t_param(&t)
}

/// Return the link to send for a YouTube video at `seconds` (None = unknown).
///
/// * Not a YouTube video (or a Short): returned unchanged.
/// * `seconds` is `Some(n)` with n >= 5: any old `t`/`start` is replaced by `t=<n>s`.
///   Under 5 s we drop the timestamp - starting from the top is what the user expects.
/// * `seconds` is `None`: the link is returned unchanged (an existing `t` is kept).
pub fn with_timestamp(raw: &str, seconds: Option<u64>) -> String {
    match classify_url(raw) {
        UrlKind::YouTubeVideo { .. } => {}
        _ => return raw.to_string(),
    }
    let Some(secs) = seconds else {
        return raw.to_string();
    };
    let Ok(mut u) = Url::parse(raw) else {
        return raw.to_string();
    };
    let kept: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| k != "t" && k != "start")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    {
        let mut q = u.query_pairs_mut();
        q.clear();
        for (k, v) in &kept {
            q.append_pair(k, v);
        }
        if secs >= 5 {
            q.append_pair("t", &format!("{secs}s"));
        }
    }
    if u.query() == Some("") {
        u.set_query(None);
    }
    u.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_t_param_forms() {
        assert_eq!(parse_t_param("90"), Some(90));
        assert_eq!(parse_t_param("90s"), Some(90));
        assert_eq!(parse_t_param("1m30s"), Some(90));
        assert_eq!(parse_t_param("1h2m3s"), Some(3723));
        assert_eq!(parse_t_param("1m30"), None);
        assert_eq!(parse_t_param("abc"), None);
        assert_eq!(parse_t_param(""), None);
    }

    #[test]
    fn parses_player_clock() {
        assert_eq!(parse_clock("4:05"), Some(245));
        assert_eq!(parse_clock("1:02:03"), Some(3723));
        assert_eq!(parse_clock("4:05 / 10:00"), Some(245));
        assert_eq!(parse_clock("0:00"), Some(0));
        assert_eq!(parse_clock("4:75"), None);
        assert_eq!(parse_clock("405"), None);
        assert_eq!(parse_clock("LIVE"), None);
    }

    #[test]
    fn adds_timestamp_to_watch_links() {
        assert_eq!(
            with_timestamp("https://www.youtube.com/watch?v=abc", Some(245)),
            "https://www.youtube.com/watch?v=abc&t=245s"
        );
        // replaces an older t, keeps playlist
        assert_eq!(
            with_timestamp("https://www.youtube.com/watch?v=abc&t=10s&list=PL1", Some(60)),
            "https://www.youtube.com/watch?v=abc&list=PL1&t=60s"
        );
        assert_eq!(with_timestamp("https://youtu.be/abc", Some(61)), "https://youtu.be/abc?t=61s");
    }

    #[test]
    fn small_or_unknown_time_behaviour() {
        // < 5 s: strip, start from the top
        assert_eq!(
            with_timestamp("https://www.youtube.com/watch?v=abc&t=100", Some(2)),
            "https://www.youtube.com/watch?v=abc"
        );
        assert_eq!(with_timestamp("https://youtu.be/abc?t=100", Some(0)), "https://youtu.be/abc");
        // unknown: unchanged, existing t kept
        assert_eq!(
            with_timestamp("https://www.youtube.com/watch?v=abc&t=100", None),
            "https://www.youtube.com/watch?v=abc&t=100"
        );
        assert_eq!(existing_timestamp("https://www.youtube.com/watch?v=abc&t=1m40s"), Some(100));
    }

    #[test]
    fn leaves_non_video_links_alone() {
        assert_eq!(with_timestamp("https://example.com/?t=5", Some(99)), "https://example.com/?t=5");
        assert_eq!(
            with_timestamp("https://youtube.com/shorts/abc", Some(99)),
            "https://youtube.com/shorts/abc"
        );
        assert_eq!(with_timestamp("https://www.youtube.com/", Some(99)), "https://www.youtube.com/");
    }
}
