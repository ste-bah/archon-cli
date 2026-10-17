//! Redact before publishing bounded wire samples, including cut credentials.
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
    if right_cut {
        for secret in secrets.iter().filter(|s| !s.is_empty()) {
            let n = (1..secret.len())
                .rev()
                .find(|&n| secret.is_char_boundary(n) && text.ends_with(&secret[..n]));
            if let Some(n) = n {
                text.truncate(text.len() - n);
                text.push_str("[REDACTED]");
            }
        }
    }
    let held = archon_observability::secret_values::SecretValues::for_evidence(
        secrets.iter().map(String::as_str),
    );
    archon_observability::shared_redactor::redact_text(&held.text(&text), "[REDACTED]")
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
        assert!(
            !scrub("held ab", &["abc".into()], false, true).contains("held ab"),
            "a held short secret's tail must be masked"
        );
    }

    #[test]
    fn free_text_samples_keep_english_but_redact_bearer_credentials() {
        let prose = "Secrets, API keys, authorization values, cookies, and session IDs must never be logged";
        let loop_script = r#"for token in alpha beta; do grep -qF -- "$token"; done"#;
        let secret_prose = "The word secret appears in prose, and token is a variable.";
        for text in [prose, loop_script, secret_prose] {
            assert_eq!(scrub(text, &[], false, false), text);
        }

        for credential in [
            concat!("Bearer gh", "p_123456789012345678901234567890123456"),
            "Bearer token_value_1234567890123456",
            concat!(
                "sk",
                "-proj-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56"
            ),
        ] {
            let redacted = scrub(credential, &[], false, false);
            assert!(
                !redacted.contains(credential),
                "leaked {credential}: {redacted}"
            );
            assert!(
                redacted.contains("[REDACTED]"),
                "missing marker: {redacted}"
            );
        }
    }

    #[test]
    fn issue_444_review_corpus_uses_shared_redaction_rules() {
        for (input, should_mask) in [
            ("Authorization: Bearer abc123", true),
            ("Proxy-Authorization: Basic abc123", true),
            ("api-key: abc123", true),
            ("Cookie: session=abc123", true),
            ("Set-Cookie: session=abc123", true),
            ("Bearer authentication", false),
            ("Bearer abc123", true),
            ("key=alpha", false),
            ("token=\"$3\"", false),
            ("curl -fsS -H token=\"x\" http://localhost/health", false),
            ("$HOME $PATH", false),
            ("token budget", false),
            ("the secret is nearby", false),
            ("password reset flow", false),
            ("Bearer tokens", false),
            ("https://alice:pass123@example.test", true),
            ("x-api-key: abc123", true),
            (r#"{"api_key":"abc123"}"#, true),
            (r#"{"apiKey":"abc123"}"#, true),
            (r#"{"access_token":"abc123"}"#, true),
            (r#"{"refresh_token":"abc123"}"#, true),
            (r#"{"client_secret":"abc123"}"#, true),
            (r#"{"password":"abc123"}"#, true),
            (r#"{"secret":"abc123"}"#, true),
            (r#"{"token":"abc123"}"#, true),
            (r#"{"auth_token":"abc123"}"#, true),
            (r#"{"private_key":"abc123"}"#, true),
            (r#"{"x-api-key":"abc123"}"#, true),
            (
                concat!(
                    "--",
                    "---BEGIN PRIVATE KEY-----\\nabc\\n-----END PRIVATE KEY-----"
                ),
                true,
            ),
            (concat!("sk", "-abcdefghijklmnopqrst0000"), true),
            (concat!("sk", "-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"), true),
            (
                concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
                true,
            ),
            (
                concat!("gh", "o_abcdefghijklmnopqrstuvwxyz0123456789"),
                true,
            ),
            (
                concat!("gh", "u_abcdefghijklmnopqrstuvwxyz0123456789"),
                true,
            ),
            (
                concat!("gh", "s_abcdefghijklmnopqrstuvwxyz0123456789"),
                true,
            ),
            (
                concat!("gh", "r_abcdefghijklmnopqrstuvwxyz0123456789"),
                true,
            ),
            (
                concat!("gi", "thub_pat_abcdefghijklmnopqrstuvwxyz0123456789"),
                true,
            ),
            (concat!("gh", "p_1234567890abcdef"), false),
            (concat!("AK", "IA0123456789ABCDEF"), true),
            (concat!("AS", "IA0123456789ABCDEF"), true),
            (concat!("xo", "xb-1234567890-abcdef"), true),
            (concat!("sk", "_live_abcdefghijklmnopqrstuvwx"), true),
            (concat!("sk", "_test_abcdefghijklmnopqrstuvwx"), true),
            (concat!("pk", "_live_abcdefghijklmnopqrstuvwx"), true),
            (concat!("pk", "_test_abcdefghijklmnopqrstuvwx"), true),
            (concat!("rk", "_live_abcdefghijklmnopqrstuvwx"), true),
            (
                concat!("ey", "JhbGciOiJIUzI1NiI.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
                true,
            ),
        ] {
            let output = scrub(input, &[], false, false);
            assert_eq!(output == input, !should_mask, "{input:?} => {output:?}");
        }
    }
}
