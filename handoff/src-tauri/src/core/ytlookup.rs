//! YouTube video identity from title + channel (item 19 fix, 2026-10-22). The phone's YouTube app
//! (Morphe) media session has title/channel/position but no video id. PC side, no API key:
//! YouTube's public search page (desktop page: the mobile page escapes its JSON as `\x22` and the
//! phone's old lookup found nothing in it), then a CONFIDENCE CHECK against oEmbed for each
//! candidate: exact title (case/space-insensitive) and the channel when both sides know it.
//! Pure functions here; the fetch is in `zt.rs`.

/// Search page URL for a title (+ channel).
pub fn search_url(title: &str, channel: Option<&str>) -> String {
    let q: String = [Some(title), channel].into_iter().flatten().collect::<Vec<_>>().join(" ");
    let enc: String = q
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else if b == b' ' { "+".into() } else { format!("%{b:02X}") })
        .collect();
    format!("https://www.youtube.com/results?search_query={enc}")
}

/// Video ids in a search results page, in order, no duplicates. Handles the desktop JSON
/// (`"videoId":"..."`) and the mobile page's hex-escaped JSON (`\x22videoId\x22:\x22...`).
pub fn candidates(page: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for pat in ["\"videoId\":\"", "\\x22videoId\\x22:\\x22"] {
        let mut rest = page;
        while let Some(i) = rest.find(pat) {
            rest = &rest[i + pat.len()..];
            let id: String = rest.chars().take(11).collect();
            if id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') && !out.contains(&id) {
                out.push(id);
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    out
}

/// "12:34" / "1:02:03" -> seconds.
pub fn parse_clock(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    parts.iter().try_fold(0u64, |acc, p| p.trim().parse::<u64>().ok().filter(|_| !p.is_empty()).map(|v| acc * 60 + v))
}

/// Candidates with the video length (seconds) the search page shows for them (`"lengthText":{..."simpleText":"12:34"}`).
/// Length is None for live streams / when the page has none for that video.
pub fn candidates_with_len(page: &str) -> Vec<(String, Option<u64>)> {
    let pat = "\"videoId\":\"";
    let mut at: Vec<(usize, String)> = Vec::new();
    let mut from = 0;
    while let Some(i) = page[from..].find(pat) {
        let st = from + i + pat.len();
        let id: String = page[st..].chars().take(11).collect();
        if id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            at.push((st, id));
        }
        from = st;
    }
    let mut out: Vec<(String, Option<u64>)> = Vec::new();
    for (n, (st, id)) in at.iter().enumerate() {
        let end = at.get(n + 1).map(|(e, _)| *e).unwrap_or(page.len()).min(st + 6000);
        let seg = &page[*st..end];
        let len = seg.find("\"lengthText\":").and_then(|i| {
            let rest = &seg[i..];
            let j = rest.find("\"simpleText\":\"")? + "\"simpleText\":\"".len();
            let v: String = rest[j..].chars().take_while(|c| *c != '"').collect();
            parse_clock(&v)
        });
        match out.iter_mut().find(|(i, _)| i == id) {
            Some(e) => {
                if e.1.is_none() {
                    e.1 = len;
                }
            }
            None => out.push((id.clone(), len)),
        }
    }
    out
}

/// The phone's media session duration against a candidate's length: within `tol_s` seconds.
/// Phone duration unknown = nothing to compare (true); phone known but the candidate length unknown = false
/// (never open a video we cannot show is the same length).
pub fn duration_matches(phone_ms: Option<u64>, cand_s: Option<u64>, tol_s: u64) -> bool {
    match (phone_ms.filter(|d| *d > 0), cand_s) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(p), Some(c)) => (p as i64 / 1000 - c as i64).unsigned_abs() <= tol_s,
    }
}

pub const DURATION_TOL_S: u64 = 3;

/// `"lengthSeconds":"754"` from a watch page (the video's real length).
pub fn length_seconds(watch_page: &str) -> Option<u64> {
    let k = "\"lengthSeconds\":\"";
    let i = watch_page.find(k)? + k.len();
    watch_page[i..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

/// A link is a POSITIVE mismatch when the page knows its length and the phone's session length is not within `tol_s`.
/// Unknown on either side is not a mismatch here (the lookup path is stricter, see [duration_matches]).
pub fn positive_duration_mismatch(phone_ms: Option<u64>, page_len_s: Option<u64>, tol_s: u64) -> bool {
    match (phone_ms.filter(|d| *d > 0), page_len_s) {
        (Some(p), Some(c)) => (p as i64 / 1000 - c as i64).unsigned_abs() > tol_s,
        _ => false,
    }
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// oEmbed JSON says the same title (and the same channel when both are known).
pub fn confident(oembed: &str, title: &str, channel: Option<&str>) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(oembed) else { return false };
    let Some(t) = v.get("title").and_then(|x| x.as_str()) else { return false };
    if norm(t) != norm(title) || norm(title).is_empty() {
        return false;
    }
    match (channel.filter(|c| !c.trim().is_empty()), v.get("author_name").and_then(|x| x.as_str())) {
        (Some(c), Some(a)) => norm(c) == norm(a),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_desktop_and_mobile_pages() {
        let d = r#"x "videoId":"T_mA4G99e5U","a" "videoId":"T_mA4G99e5U" "videoId":"Dn26dn7e8Q0""#;
        assert_eq!(candidates(d), vec!["T_mA4G99e5U", "Dn26dn7e8Q0"]);
        let m = r"\x7b\x22videoId\x22:\x22WiYDdS0Rhsw\x22\x7d";
        assert_eq!(candidates(m), vec!["WiYDdS0Rhsw"]);
        assert!(candidates("nothing here").is_empty());
    }

    #[test]
    fn confidence_exact_title_and_channel() {
        let o = r#"{"title":"Heyy Babyy CALIMAX - हम - Akshay Kumar","author_name":"Bollywood Climax"}"#;
        assert!(confident(o, "Heyy Babyy CALIMAX - हम - Akshay Kumar", Some("Bollywood Climax")));
        assert!(confident(o, "heyy  babyy calimax - हम - akshay kumar", None));
        assert!(!confident(o, "Heyy Babyy CALIMAX - हम - Akshay Kumar", Some("Shemaroo Movies")));
        assert!(!confident(o, "Heyy Babyy Emotional Ending", Some("Bollywood Climax")));
        assert!(!confident("not json", "x", None));
    }

    #[test]
    fn lengths_from_search_page() {
        let p = r#"{"videoRenderer":{"videoId":"AAAAAAAAAAA","title":"x","lengthText":{"accessibility":{"accessibilityData":{"label":"12 minutes, 34 seconds"}},"simpleText":"12:34"},"navigationEndpoint":{"videoId":"AAAAAAAAAAA"}}},
            {"videoRenderer":{"videoId":"BBBBBBBBBBB","title":"x","lengthText":{"simpleText":"1:02:03"}}},{"videoRenderer":{"videoId":"CCCCCCCCCCC","title":"live"}}"#;
        assert_eq!(
            candidates_with_len(p),
            vec![("AAAAAAAAAAA".into(), Some(754)), ("BBBBBBBBBBB".into(), Some(3723)), ("CCCCCCCCCCC".into(), None)]
        );
        assert_eq!(parse_clock("0:45"), Some(45));
        assert_eq!(parse_clock("garbage"), None);
    }

    #[test]
    fn watch_page_length_check() {
        assert_eq!(length_seconds(r#"x"videoDetails":{"videoId":"a","lengthSeconds":"754","keywords""#), Some(754));
        assert_eq!(length_seconds("nothing"), None);
        assert!(positive_duration_mismatch(Some(1_200_000), Some(754), 3));
        assert!(!positive_duration_mismatch(Some(754_500), Some(754), 3));
        assert!(!positive_duration_mismatch(Some(754_500), None, 3));
        assert!(!positive_duration_mismatch(None, Some(754), 3));
    }

    #[test]
    fn duration_identity() {
        // same title, different length: rejected; within +-3 s: accepted
        assert!(duration_matches(Some(754_000), Some(754), 3));
        assert!(duration_matches(Some(754_000), Some(751), 3));
        assert!(duration_matches(Some(754_900), Some(757), 3));
        assert!(!duration_matches(Some(754_000), Some(760), 3));
        assert!(!duration_matches(Some(754_000), Some(1800), 3));
        // phone known, candidate unknown: never open
        assert!(!duration_matches(Some(754_000), None, 3));
        // phone unknown: nothing to compare
        assert!(duration_matches(None, Some(754), 3));
        assert!(duration_matches(Some(0), None, 3));
    }

    #[test]
    fn search_url_encodes() {
        assert_eq!(search_url("a b&c", Some("ch")), "https://www.youtube.com/results?search_query=a+b%26c+ch");
    }
}
