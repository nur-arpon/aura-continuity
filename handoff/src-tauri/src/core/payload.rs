//! What gets sent: a URL, a piece of text, or a file.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum Payload {
    Url(String),
    Text(String),
    File(PathBuf),
}

impl Payload {
    /// Short, log-safe description: host for URLs, length for text, file name for files.
    /// Never contains the full text or the full URL query.
    pub fn summary(&self) -> String {
        match self {
            Payload::Url(u) => match url::Url::parse(u) {
                Ok(p) => format!("link {}{}", p.host_str().unwrap_or(""), short_path(p.path())),
                Err(_) => "link".into(),
            },
            Payload::Text(t) => format!("text ({} chars)", t.chars().count()),
            Payload::File(p) => format!(
                "file {}",
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            ),
        }
    }

    /// Label for menus: a bit more readable than `summary`, still truncated.
    pub fn label(&self) -> String {
        match self {
            Payload::Url(u) => truncate(u.trim_start_matches("https://").trim_start_matches("http://"), 48),
            Payload::Text(t) => format!("\u{201C}{}\u{201D}", truncate(&t.replace(['\r', '\n'], " "), 40)),
            Payload::File(p) => truncate(
                &p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                48,
            ),
        }
    }
}

fn short_path(p: &str) -> String {
    if p.len() <= 1 {
        String::new()
    } else {
        truncate(p, 24)
    }
}

/// Truncate on a char boundary and add an ellipsis.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_hides_query_and_text() {
        let p = Payload::Url("https://example.com/a?secret=1".into());
        assert_eq!(p.summary(), "link example.com/a");
        assert_eq!(Payload::Text("hello world".into()).summary(), "text (11 chars)");
        assert_eq!(Payload::File(PathBuf::from(r"C:\x\notes.pdf")).summary(), "file notes.pdf");
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("abcdef", 4), "abc\u{2026}");
        assert_eq!(truncate("ab", 4), "ab");
        assert_eq!(truncate("\u{00e9}\u{00e9}\u{00e9}\u{00e9}\u{00e9}", 3), "\u{00e9}\u{00e9}\u{2026}");
    }

    #[test]
    fn payload_roundtrips_json() {
        let p = Payload::File(PathBuf::from(r"C:\a b\c.txt"));
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Payload>(&j).unwrap(), p);
    }
}
