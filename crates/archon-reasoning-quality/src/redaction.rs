use regex::Regex;

use crate::canonical::hash_hex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionConfig {
    pub allow_raw_text: bool,
    pub home_dir: Option<String>,
    pub workspace_root: Option<String>,
    pub max_excerpt_chars: usize,
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            allow_raw_text: false,
            home_dir: std::env::var("HOME").ok(),
            workspace_root: None,
            max_excerpt_chars: 600,
        }
    }
}

pub fn redact_text(text: &str, config: &RedactionConfig) -> String {
    if config.allow_raw_text {
        return truncate(text, config.max_excerpt_chars);
    }
    let mut out = text.to_string();
    if let Some(home) = &config.home_dir {
        if !home.is_empty() {
            out = out.replace(home, "$HOME");
        }
    }
    if let Some(root) = &config.workspace_root {
        if !root.is_empty() {
            out = out.replace(root, "$WORKSPACE");
        }
    }
    out = replace_regex(&out, r#"https?://[^\s)>"]+"#, "[url-redacted]");
    out = replace_regex(
        &out,
        r#"(?i)bearer\s+[a-z0-9._\-]{12,}"#,
        "bearer [token-redacted]",
    );
    out = replace_regex(
        &out,
        r#"(?i)(api[_-]?key|token|secret)\s*[:=]\s*[a-z0-9._\-]{8,}"#,
        "$1=[secret-redacted]",
    );
    out = replace_regex(
        &out,
        r#"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}"#,
        "[email-redacted]",
    );
    out = replace_regex(
        &out,
        r#"\b[A-Za-z0-9+/=_\-]{32,}\b"#,
        "[high-entropy-redacted]",
    );
    truncate(&out, config.max_excerpt_chars)
}

pub fn redact_entity_key(entity_key: &str, config: &RedactionConfig) -> String {
    let redacted = redact_text(entity_key, config);
    if config.allow_raw_text || !looks_sensitive_entity(&redacted) {
        redacted
    } else {
        format!(
            "entity_hash:{}",
            hash_hex(&redacted).chars().take(16).collect::<String>()
        )
    }
}

fn replace_regex(input: &str, pattern: &str, replacement: &str) -> String {
    Regex::new(pattern)
        .map(|regex| regex.replace_all(input, replacement).to_string())
        .unwrap_or_else(|_| input.to_string())
}

fn looks_sensitive_entity(value: &str) -> bool {
    value.starts_with('/')
        || value.contains("://")
        || value.contains("@")
        || value.to_lowercase().contains("token")
        || value.to_lowercase().contains("secret")
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets_email_urls_and_home_paths() {
        let config = RedactionConfig {
            home_dir: Some("/home/steve".to_string()),
            ..RedactionConfig::default()
        };
        let text = "/home/steve/project token=abc1234567890 me@example.com https://example.com/x";
        let redacted = redact_text(text, &config);
        assert!(redacted.contains("$HOME"));
        assert!(redacted.contains("[secret-redacted]"));
        assert!(redacted.contains("[email-redacted]"));
        assert!(redacted.contains("[url-redacted]"));
    }
}
