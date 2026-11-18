//! `kdeconnect-cli` output parsing and argument building. No process spawning here.
//! The list parser is the one from `..\handoff\src-tauri\src\core\kde_cli.rs` (read, copied,
//! not edited there), so both apps read the CLI the same way until Handoff is absorbed.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
}

/// Parse `kdeconnect-cli -l --id-name-only` / `-a --id-name-only`: one device per line,
/// `<id> <name with spaces>`. Summary lines ("1 device found") and blanks are ignored.
pub fn parse_id_name_list(out: &str) -> Vec<Device> {
    out.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (id, name) = line.split_once(' ').unwrap_or((line, ""));
            let valid = id.len() >= 16 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            valid.then(|| Device { id: id.to_string(), name: name.trim().to_string() })
        })
        .collect()
}

/// The preferred phone if present; with no preference, the first one. A preferred phone that
/// is missing gives `None` (never send to some other device by surprise).
pub fn pick<'a>(devs: &'a [Device], preferred: Option<&str>) -> Option<&'a Device> {
    match preferred.filter(|s| !s.is_empty()) {
        Some(want) => devs.iter().find(|d| d.id == want),
        None => devs.first(),
    }
}

pub fn list_paired_args() -> Vec<String> {
    vec!["-l".into(), "--id-name-only".into()]
}
pub fn list_reachable_args() -> Vec<String> {
    vec!["-a".into(), "--id-name-only".into()]
}
pub fn share_file_args(id: &str, path: &str) -> Vec<String> {
    vec!["-d".into(), id.into(), "--share".into(), path.into()]
}
pub fn ring_args(id: &str) -> Vec<String> {
    vec!["-d".into(), id.into(), "--ring".into()]
}
/// Ask the daemon to re-run discovery on every link (UDP broadcast on 1716).
pub fn refresh_args() -> Vec<String> {
    vec!["--refresh".into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_cli_output() {
        // captured on this PC by Handoff, kdeconnect-cli 26.08.2
        let d = parse_id_name_list("47b360c8d8a8400ab28503670bc6483d Galaxy S23 Ultra\n");
        assert_eq!(d, vec![Device { id: "47b360c8d8a8400ab28503670bc6483d".into(), name: "Galaxy S23 Ultra".into() }]);
        assert!(parse_id_name_list("0 devices found\n").is_empty());
        assert_eq!(parse_id_name_list("aaaaaaaaaaaaaaaa_1 A\r\nbbbbbbbbbbbbbbbb-2 B\r\n2 devices found").len(), 2);
    }

    #[test]
    fn picks_preferred_or_first_never_other() {
        let ds = vec![Device { id: "a".repeat(16), name: "A".into() }, Device { id: "b".repeat(16), name: "B".into() }];
        assert_eq!(pick(&ds, None).unwrap().name, "A");
        assert_eq!(pick(&ds, Some("")).unwrap().name, "A");
        assert_eq!(pick(&ds, Some(&"b".repeat(16))).unwrap().name, "B");
        assert!(pick(&ds, Some("zzzzzzzzzzzzzzzz")).is_none());
    }

    #[test]
    fn args() {
        assert_eq!(share_file_args("ID", r"C:\a b\s.png"), ["-d", "ID", "--share", r"C:\a b\s.png"]);
        assert_eq!(ring_args("ID"), ["-d", "ID", "--ring"]);
        assert_eq!(list_reachable_args(), ["-a", "--id-name-only"]);
        assert_eq!(list_paired_args(), ["-l", "--id-name-only"]);
        assert_eq!(refresh_args(), ["--refresh"]);
    }
}
