//! VAL-WC-006 secret scan: anchored, length-bounded regexes over patch
//! added-lines. Fail-closed; no allowlist (PRD §12).

use std::sync::OnceLock;

use regex::Regex;

/// (compiled regex, rule name). Built once via OnceLock.
fn secret_regexes() -> &'static [(Regex, &'static str)] {
    static REGEXES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    REGEXES.get_or_init(|| {
        let specs: &[(&str, &str)] = &[
            (r"sk-ant-[A-Za-z0-9_-]{40,}", "anthropic_api_key"),
            (r"sk-[A-Za-z0-9]{32,}", "openai_api_key"),
            (r"ghp_[A-Za-z0-9]{36}", "github_pat"),
            (r"xoxb-[0-9]{10,}-[0-9]{10,}-[A-Za-z0-9]{24,}", "slack_bot_token"),
            (r"AKIA[0-9A-Z]{16}", "aws_access_key"),
            (
                r"-----BEGIN (RSA|OPENSSH|DSA|EC|PGP) PRIVATE KEY-----",
                "private_key_pem",
            ),
        ];
        specs
            .iter()
            .map(|(pat, rule)| (Regex::new(pat).expect("static secret regex compiles"), *rule))
            .collect()
    })
}

/// Scan patch added-lines (`+ ` content, not `+++` headers) for secrets.
/// Returns the first match's rule + a bounded preview.
pub fn secret_scan(patch_bytes: &[u8]) -> Option<(&'static str, String)> {
    let text = String::from_utf8_lossy(patch_bytes);
    for line in text.lines() {
        if !line.starts_with('+') || line.starts_with("+++") {
            continue;
        }
        for (regex, rule) in secret_regexes() {
            if regex.is_match(line) {
                return Some((rule, preview(line)));
            }
        }
    }
    None
}

/// Length-bounded preview so an error message never echoes a full secret line.
fn preview(line: &str) -> String {
    const MAX: usize = 80;
    let trimmed = line.trim_start_matches('+').trim();
    if trimmed.len() <= MAX {
        trimmed.to_string()
    } else {
        format!("{}…", &trimmed[..MAX])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn added(line: &str) -> Vec<u8> {
        format!("@@ -0,0 +1 @@\n+{line}\n").into_bytes()
    }

    #[test]
    fn anthropic_key_with_long_tail_triggers() {
        let hit = secret_scan(&added("sk-ant-FAKEKEYFOR40CHARS0123456789ABCDEFGHIJKLMN"));
        assert_eq!(hit.unwrap().0, "anthropic_api_key");
    }

    #[test]
    fn private_key_pem_triggers() {
        let hit = secret_scan(&added("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert_eq!(hit.unwrap().0, "private_key_pem");
    }

    #[test]
    fn aws_key_triggers() {
        assert_eq!(
            secret_scan(&added("AKIAIOSFODNN7EXAMPLE")).unwrap().0,
            "aws_access_key"
        );
    }

    #[test]
    fn sk_prefix_without_tail_does_not_trigger() {
        assert!(secret_scan(&added("sk-input-config")).is_none());
    }

    #[test]
    fn sk_ant_short_tail_does_not_trigger() {
        assert!(secret_scan(&added("sk-ant-short")).is_none());
    }

    #[test]
    fn removed_and_header_lines_ignored() {
        let patch = b"--- a/x\n+++ b/x\n-sk-ant-FAKEKEYFOR40CHARS0123456789ABCDEFGHIJKLMN\n";
        assert!(secret_scan(patch).is_none());
    }

    #[test]
    fn preview_is_bounded() {
        let long = "a".repeat(200);
        let line = format!("+{long}");
        assert!(preview(&line).chars().count() <= 81);
    }
}
