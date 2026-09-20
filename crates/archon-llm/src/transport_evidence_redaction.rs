//! Redact before publishing bounded wire samples, including cut credentials.
use once_cell::sync::Lazy;
use regex::Regex;
static FIELDS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
    r#"(?i)("[^"]*(?:token|secret|password|api[_-]?key|authorization|cookie|credential)[^"]*"\s*:\s*")(?:\\.|[^"\\])*(?:"|$)"#
).unwrap()
});

pub(super) fn scrub(text: &str, secrets: &[String], left_cut: bool, right_cut: bool) -> String {
    let mut text = text.to_string();
    // A tail can begin inside an unknown credential field: discard that partial
    // wire line. Full subsequent SSE lines remain useful for stop metadata.
    if left_cut {
        text = text
            .find('\n')
            .map(|i| format!("[REDACTED PARTIAL LINE]{}", &text[i..]))
            .unwrap_or_else(|| "[REDACTED PARTIAL LINE]".into());
    }
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        if right_cut {
            let n = (1..secret.len())
                .rev()
                .find(|&n| secret.is_char_boundary(n) && text.ends_with(&secret[..n]));
            if let Some(n) = n {
                text.truncate(text.len() - n);
                text.push_str("[REDACTED]");
            }
        }
        text = text.replace(secret, "[REDACTED]");
    }
    let text = FIELDS.replace_all(&text, "$1[REDACTED]\"");
    static SHAPES: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
        r"(?i)bearer\s+[A-Za-z0-9._-]+|sk-[A-Za-z0-9_-]+|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{36}|eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+"
    ).unwrap()
    });
    SHAPES.replace_all(&text, "[REDACTED]").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_mask_values_without_corrupting_finish_reason() {
        let raw = r#"{"access_token":"private-value","finish_reason":"max_tokens"}"#;
        let redacted = scrub(raw, &[], false, false);
        assert!(!redacted.contains("private-value"));
        assert!(redacted.contains("max_tokens"));
        assert!(!scrub(r#"{"password":"partial"#, &[], false, true).contains("partial"));
        assert!(!scrub("private-value\"}\ndata: {}", &[], true, false).contains("private-value"));
        assert!(
            !scrub(
                "echo long-credential",
                &["long-credential-tail".into()],
                false,
                true
            )
            .contains("long-credential")
        );
    }
}
