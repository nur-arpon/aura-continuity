//! Pure parsing of `kdeconnect-cli` output and argument building (no process spawning here).

use super::payload::Payload;
use super::transport::Peer;

/// Parse `kdeconnect-cli --list-available --id-name-only` (or `-l --id-name-only`):
/// one device per line, `<id> <name with spaces>`. Summary lines like "1 device found"
/// and blank lines are ignored.
pub fn parse_id_name_list(out: &str) -> Vec<Peer> {
    out.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (id, name) = line.split_once(' ').unwrap_or((line, ""));
            let valid_id = id.len() >= 16
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            valid_id.then(|| Peer { id: id.to_string(), name: name.trim().to_string() })
        })
        .collect()
}

/// Choose which phone to use: the preferred id if present, else the first.
pub fn pick_peer(peers: &[Peer], preferred: Option<&str>) -> Option<Peer> {
    if let Some(want) = preferred.filter(|s| !s.is_empty()) {
        if let Some(p) = peers.iter().find(|p| p.id == want) {
            return Some(p.clone());
        }
        // Preferred phone is not around: do not silently send to some other phone.
        return None;
    }
    peers.first().cloned()
}

/// Arguments for sending `payload` to `device_id`.
pub fn share_args(device_id: &str, payload: &Payload) -> Vec<String> {
    let mut a = vec!["-d".to_string(), device_id.to_string()];
    match payload {
        Payload::Url(u) => {
            a.push("--share".into());
            a.push(u.clone());
        }
        Payload::Text(t) => {
            a.push("--share-text".into());
            a.push(t.clone());
        }
        Payload::File(p) => {
            a.push("--share".into());
            a.push(p.display().to_string());
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_real_cli_output() {
        // Captured from this PC, kdeconnect-cli 26.08.2
        let out = "47b360c8d8a8400ab28503670bc6483d Galaxy S23 Ultra\n";
        assert_eq!(
            parse_id_name_list(out),
            vec![Peer { id: "47b360c8d8a8400ab28503670bc6483d".into(), name: "Galaxy S23 Ultra".into() }]
        );
        assert!(parse_id_name_list("0 devices found\n").is_empty());
        assert!(parse_id_name_list("").is_empty());
        let two = "aaaaaaaaaaaaaaaa_1 Phone A\r\nbbbbbbbbbbbbbbbb-2 Tab B\r\n2 devices found";
        assert_eq!(parse_id_name_list(two).len(), 2);
    }

    #[test]
    fn picks_preferred_or_first() {
        let ps = vec![
            Peer { id: "a".repeat(16), name: "A".into() },
            Peer { id: "b".repeat(16), name: "B".into() },
        ];
        assert_eq!(pick_peer(&ps, None).unwrap().name, "A");
        assert_eq!(pick_peer(&ps, Some("")).unwrap().name, "A");
        assert_eq!(pick_peer(&ps, Some(&"b".repeat(16))).unwrap().name, "B");
        assert_eq!(pick_peer(&ps, Some("zzzzzzzzzzzzzzzz")), None);
        assert_eq!(pick_peer(&[], None), None);
    }

    #[test]
    fn builds_share_args() {
        assert_eq!(
            share_args("ID", &Payload::Url("https://example.com/".into())),
            ["-d", "ID", "--share", "https://example.com/"]
        );
        assert_eq!(share_args("ID", &Payload::Text("hi there".into())), ["-d", "ID", "--share-text", "hi there"]);
        assert_eq!(
            share_args("ID", &Payload::File(PathBuf::from(r"C:\a b\c.pdf"))),
            ["-d", "ID", "--share", r"C:\a b\c.pdf"]
        );
    }
}
