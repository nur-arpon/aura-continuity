//! Turning raw strings (an address-bar value, clipboard text, a received share) into
//! something we know how to send or open.

use super::payload::Payload;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UrlKind {
    /// Ordinary http(s) page.
    Web,
    /// youtube.com/watch?v=..., m.youtube.com, music.youtube.com, youtu.be/<id>
    YouTubeVideo { video_id: String },
    /// youtube.com/shorts/<id> (no timestamp support on the phone)
    YouTubeShort { video_id: String },
    /// brave://, chrome://, edge://, about:, chrome-extension:// ... the phone cannot open these.
    BrowserInternal,
    /// file:// URL
    LocalFile,
    /// Anything else (mailto:, tel:, custom schemes)
    Other,
}

const INTERNAL_SCHEMES: &[&str] = &[
    "brave", "chrome", "edge", "about", "chrome-extension", "chrome-search", "devtools", "view-source",
    "chrome-untrusted",
];

/// Classify an absolute URL.
pub fn classify_url(raw: &str) -> UrlKind {
    let Ok(u) = Url::parse(raw.trim()) else {
        return UrlKind::Other;
    };
    let scheme = u.scheme();
    if INTERNAL_SCHEMES.contains(&scheme) {
        return UrlKind::BrowserInternal;
    }
    if scheme == "file" {
        return UrlKind::LocalFile;
    }
    if scheme != "http" && scheme != "https" {
        return UrlKind::Other;
    }
    let host = u.host_str().unwrap_or("").to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let is_yt = matches!(host, "youtube.com" | "m.youtube.com" | "music.youtube.com");
    if is_yt {
        let path = u.path();
        if path == "/watch" {
            if let Some((_, v)) = u.query_pairs().find(|(k, _)| k == "v") {
                if !v.is_empty() {
                    return UrlKind::YouTubeVideo { video_id: v.into_owned() };
                }
            }
        } else if let Some(rest) = path.strip_prefix("/shorts/") {
            let id = rest.trim_end_matches('/');
            if !id.is_empty() {
                return UrlKind::YouTubeShort { video_id: id.to_string() };
            }
        } else if let Some(rest) = path.strip_prefix("/live/") {
            let id = rest.trim_end_matches('/');
            if !id.is_empty() {
                return UrlKind::YouTubeVideo { video_id: id.to_string() };
            }
        }
        return UrlKind::Web;
    }
    if host == "youtu.be" {
        let id = u.path().trim_start_matches('/').trim_end_matches('/');
        if !id.is_empty() {
            return UrlKind::YouTubeVideo { video_id: id.to_string() };
        }
    }
    UrlKind::Web
}

/// Chromium-family address bars show "example.com/page" (scheme hidden) when unfocused,
/// and arbitrary search text while the user is typing. Turn that into an absolute URL,
/// or `None` when it is clearly not a URL.
pub fn normalize_address_bar(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return None;
    }
    // Already absolute (has a scheme we recognise)?
    if let Ok(u) = Url::parse(t) {
        let s = u.scheme();
        if s == "http" || s == "https" || s == "file" || INTERNAL_SCHEMES.contains(&s) {
            return Some(u.to_string());
        }
        // "localhost:3000" parses as scheme "localhost" - fall through to host handling.
        if s != "localhost" && t.contains("://") {
            return Some(u.to_string());
        }
    }
    let host_part = t.split(['/', '?', '#']).next().unwrap_or("");
    let host_no_port = host_part.split(':').next().unwrap_or("");
    let looks_like_host = host_no_port == "localhost"
        || (host_no_port.contains('.')
            && !host_no_port.starts_with('.')
            && !host_no_port.ends_with('.')
            && host_no_port.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.'));
    if !looks_like_host {
        return None;
    }
    let scheme = if host_no_port == "localhost" || host_no_port.starts_with("127.") { "http" } else { "https" };
    Url::parse(&format!("{scheme}://{t}")).ok().map(|u| u.to_string())
}

/// Clipboard / selected text: a single http(s) URL becomes a Url payload, everything else is Text.
pub fn classify_text(text: &str) -> Option<Payload> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if !t.chars().any(char::is_whitespace) {
        if let Ok(u) = Url::parse(t) {
            if u.scheme() == "http" || u.scheme() == "https" {
                return Some(Payload::Url(u.to_string()));
            }
        }
        if t.starts_with("www.") {
            if let Some(u) = normalize_address_bar(t) {
                return Some(Payload::Url(u));
            }
        }
    }
    Some(Payload::Text(t.to_string()))
}

/// Can the phone open this URL?
pub fn is_sendable_url(raw: &str) -> bool {
    matches!(
        classify_url(raw),
        UrlKind::Web | UrlKind::YouTubeVideo { .. } | UrlKind::YouTubeShort { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_youtube_variants() {
        let id = |s: &str| UrlKind::YouTubeVideo { video_id: s.into() };
        assert_eq!(classify_url("https://www.youtube.com/watch?v=abc123&list=PL1"), id("abc123"));
        assert_eq!(classify_url("https://m.youtube.com/watch?v=abc123"), id("abc123"));
        assert_eq!(classify_url("https://music.youtube.com/watch?v=abc123"), id("abc123"));
        assert_eq!(classify_url("https://youtu.be/abc123?t=30"), id("abc123"));
        assert_eq!(classify_url("https://www.youtube.com/live/xyz"), id("xyz"));
        assert_eq!(
            classify_url("https://youtube.com/shorts/sh0rt"),
            UrlKind::YouTubeShort { video_id: "sh0rt".into() }
        );
        assert_eq!(classify_url("https://www.youtube.com/"), UrlKind::Web);
        assert_eq!(classify_url("https://www.youtube.com/watch"), UrlKind::Web);
        assert_eq!(classify_url("https://notyoutube.com/watch?v=x"), UrlKind::Web);
    }

    #[test]
    fn classifies_internal_and_other() {
        assert_eq!(classify_url("brave://settings"), UrlKind::BrowserInternal);
        assert_eq!(classify_url("chrome://newtab/"), UrlKind::BrowserInternal);
        assert_eq!(classify_url("edge://favorites"), UrlKind::BrowserInternal);
        assert_eq!(classify_url("about:blank"), UrlKind::BrowserInternal);
        assert_eq!(classify_url("file:///C:/x.pdf"), UrlKind::LocalFile);
        assert_eq!(classify_url("mailto:a@b.c"), UrlKind::Other);
        assert_eq!(classify_url("not a url"), UrlKind::Other);
        assert!(is_sendable_url("https://example.com"));
        assert!(!is_sendable_url("brave://newtab"));
    }

    #[test]
    fn normalizes_chromium_address_bar() {
        assert_eq!(normalize_address_bar("example.com").as_deref(), Some("https://example.com/"));
        assert_eq!(
            normalize_address_bar("youtube.com/watch?v=abc").as_deref(),
            Some("https://youtube.com/watch?v=abc")
        );
        assert_eq!(
            normalize_address_bar("https://example.com/a?b=1").as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(normalize_address_bar("http://site.test/").as_deref(), Some("http://site.test/"));
        assert_eq!(normalize_address_bar("localhost:3000/x").as_deref(), Some("http://localhost:3000/x"));
        assert_eq!(normalize_address_bar("brave://settings").as_deref(), Some("brave://settings"));
        // The user is typing a search, not a URL.
        assert_eq!(normalize_address_bar("how to cook rice"), None);
        assert_eq!(normalize_address_bar("rice"), None);
        assert_eq!(normalize_address_bar(""), None);
        assert_eq!(normalize_address_bar("   "), None);
    }

    #[test]
    fn classifies_text() {
        assert_eq!(
            classify_text("  https://example.com/x \n"),
            Some(Payload::Url("https://example.com/x".into()))
        );
        assert_eq!(classify_text("www.example.com"), Some(Payload::Url("https://www.example.com/".into())));
        assert_eq!(classify_text("see https://example.com"), Some(Payload::Text("see https://example.com".into())));
        assert_eq!(classify_text("hello"), Some(Payload::Text("hello".into())));
        assert_eq!(classify_text("  \n "), None);
    }
}
