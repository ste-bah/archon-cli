//! Shared credential redaction for logs, workflow records, and transport samples.
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde_json::Value;

static SHAPES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r#"(?i)(?:\{[^{}]*"type"\s*:\s*"service_account"[\s\S]*?\}|"#,
        r"-----BEGIN\s(?:RSA\s|EC\s|)?PRIVATE\sKEY-----[\s\S]*?-----END\s(?:RSA\s|EC\s|)?PRIVATE\sKEY-----|",
        r"(?:sk|pk|rk)_(?:live|test)_[A-Za-z0-9]{24,}|",
        r"\bsk-ant-[A-Za-z0-9_-]{20,}|\bsk-[A-Za-z0-9_-]+|",
        r"gh[pousr]_[A-Za-z0-9]+|github_pat_[A-Za-z0-9_]+|",
        r"(?:AKIA|ASIA)[A-Z0-9]{16}|xox[abpr]-[A-Za-z0-9-]+|",
        r"eyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+|",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----)"
    ))
    .expect("credential shape regex is constant")
});
// Exact held-value matching and fixed shapes cannot recover a credential
// deliberately split between fields, or identify an arbitrary base64 encoding.
static USERINFO: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)([a-z][a-z0-9+.-]*://[^:/@\s]+:)[^@/\s]+(@)").unwrap());
static AUTH_HEADER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)((?:authorization|proxy-authorization)\s*:\s*)([A-Za-z][A-Za-z0-9_-]*)(?:\s+[^\r\n]*)?").unwrap()
});
static VALUE_HEADER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)((?:x-api-key|api-key|cookie|set-cookie)\s*:\s*)[^\r\n]*").unwrap()
});
static QUOTED_CREDENTIAL_FIELD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r#"(?i)(["']?[^\s"']*(?:api[_-]?key|apikey|token|secret|password|passwd|authorization|credential|private[_-]?key|cookie|thinking|reasoning|raw_text|encrypted_reasoning)[^\s"']*["']?\s*[:=]\s*)"(?:\\.|[^"\\])*""#
    ))
    .unwrap()
});
static TRUNCATED_CREDENTIAL_FIELD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r#"(?i)(["']?[^\s"']*(?:api[_-]?key|apikey|token|secret|password|passwd|authorization|credential|private[_-]?key|cookie|thinking|reasoning|raw_text|encrypted_reasoning)[^\s"']*["']?\s*:\s*["'])[^"\r\n]*$"#
    ))
    .unwrap()
});
static CREDENTIAL_FIELD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)(["']?[^\s"']*(?:api[_-]?key|apikey|token|secret|password|passwd|authorization|credential|private[_-]?key|cookie|thinking|reasoning|raw_text|encrypted_reasoning)[^\s"']*["']?\s*[:=]\s*)([^"'\s,;}]+)"#,
    )
    .unwrap()
});
static BEARER: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)(\bbearer\s+)(\S+)").unwrap());

/// Redact held values, credential carriers, and the documented fixed formats.
pub fn redact_text(value: &str, marker: &str) -> String {
    let mut out = crate::secret_values::redact_registered(value);
    out = SHAPES.replace_all(&out, marker).into_owned();
    out = USERINFO
        .replace_all(&out, |c: &Captures<'_>| {
            format!("{}{}{}", &c[1], marker, &c[2])
        })
        .into_owned();
    out = AUTH_HEADER
        .replace_all(&out, |c: &Captures<'_>| {
            format!("{}{} {}", &c[1], &c[2], marker)
        })
        .into_owned();
    out = VALUE_HEADER
        .replace_all(&out, |c: &Captures<'_>| format!("{}{}", &c[1], marker))
        .into_owned();
    out = TRUNCATED_CREDENTIAL_FIELD
        .replace_all(&out, |c: &Captures<'_>| {
            if is_credential_field_prefix(&c[1]) {
                format!("{}{}", &c[1], marker)
            } else {
                c[0].to_string()
            }
        })
        .into_owned();
    let input = out;
    out = QUOTED_CREDENTIAL_FIELD
        .replace_all(&input, |c: &Captures<'_>| {
            let line = input[..c.get(0).unwrap().start()]
                .rsplit('\n')
                .next()
                .unwrap_or_default();
            let shell_header = line.contains(" -H ") || line.trim_start().starts_with("curl ");
            if !is_credential_field_prefix(&c[1])
                || (c[1].contains('=') && (c[0].contains('$') || shell_header))
            {
                c[0].to_string()
            } else {
                format!("{}\"{}\"", &c[1], marker)
            }
        })
        .into_owned();
    let input = out;
    out = CREDENTIAL_FIELD
        .replace_all(&input, |c: &Captures<'_>| {
            let start = c.get(0).unwrap().start();
            let line = input[..start].rsplit('\n').next().unwrap_or_default();
            let shell_header = line.contains(" -H ") || line.trim_start().starts_with("curl ");
            if !is_credential_field_prefix(&c[1])
                || (c[1].contains('=') && (shell_header || c[2].contains('$')))
            {
                c[0].to_string()
            } else {
                format!("{}{}", &c[1], marker)
            }
        })
        .into_owned();
    // Mask any non-empty word after Bearer. This intentionally masks prose
    // such as "Bearer authentication" because free text cannot distinguish
    // that phrase from a scheme credential.
    out = BEARER.replace_all(&out, format!("$1{marker}")).into_owned();
    out
}

/// Apply the shared rules recursively to parsed JSON, redacting exact credential keys.
pub fn redact_json(value: Value, marker: &str) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let value = if is_credential_key(&key) {
                        Value::String(marker.to_string())
                    } else {
                        redact_json(value, marker)
                    };
                    (key, value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| redact_json(item, marker))
                .collect(),
        ),
        Value::String(text) => Value::String(redact_text(&text, marker)),
        other => other,
    }
}

/// Whether a structured key contains a credential carrier name.
pub fn is_credential_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    if is_usage_counter_key(&key) {
        return false;
    }
    [
        "api_key",
        "api-key",
        "apikey",
        "token",
        "secret",
        "password",
        "passwd",
        "authorization",
        "credential",
        "private_key",
        "private-key",
        "cookie",
        "thinking",
        "reasoning",
        "oauth_token",
        "access_token",
        "refresh_token",
        "raw_text",
        "encrypted_reasoning",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn is_usage_counter_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "max_tokens",
        "token_count",
        "tokens_used",
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
    ]
    .contains(&key.as_str())
        || (key.ends_with("_tokens")
            && [
                "input_",
                "output_",
                "cache_",
                "prompt_",
                "completion_",
                "total_",
                "reasoning_",
                "audio_",
                "image_",
            ]
            .iter()
            .any(|prefix| key.starts_with(prefix)))
}

fn is_credential_field_prefix(prefix: &str) -> bool {
    let name = prefix
        .trim()
        .trim_matches(['"', '\''])
        .trim_end_matches(':');
    let name = name.trim().trim_matches(['"', '\'']);
    is_credential_key(name)
}

#[cfg(test)]
mod tests {
    use super::{is_credential_key, redact_json, redact_text};
    use serde_json::json;

    #[test]
    fn fixed_shapes_from_legacy_sinks_still_redact() {
        let cases = [
            concat!("sk", "-abcdefghijklmnopqrst0000"),
            concat!("sk", "-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"),
            concat!(
                "sk",
                "-proj-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56"
            ),
            concat!(
                "sk",
                "-svcacct-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56"
            ),
            concat!("AK", "IA0123456789ABCDEF"),
            concat!("AS", "IA0123456789ABCDEF"),
            concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gh", "o_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gh", "u_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gh", "s_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gh", "r_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gi", "thub_pat_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("sk", "_live_abcdefghijklmnopqrstuvwx"),
            concat!("sk", "_test_abcdefghijklmnopqrstuvwx"),
            concat!("pk", "_live_abcdefghijklmnopqrstuvwx"),
            concat!("pk", "_test_abcdefghijklmnopqrstuvwx"),
            concat!("rk", "_live_abcdefghijklmnopqrstuvwx"),
            concat!("rk", "_test_abcdefghijklmnopqrstuvwx"),
            concat!("xo", "xb-1234567890-abcdef"),
            concat!("ey", "JhbGciOiJIUzI1NiI.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
            concat!("Bearer ya", "29.a0Af_abcDEF-123"),
            concat!(
                "--",
                "---BEGIN PRIVATE KEY-----\\nabc\\n-----END PRIVATE KEY-----"
            ),
            concat!(
                "--",
                "---BEGIN RSA PRIVATE KEY-----\\nabc\\n-----END RSA PRIVATE KEY-----"
            ),
            concat!(
                "--",
                "---BEGIN EC PRIVATE KEY-----\\nabc\\n-----END EC PRIVATE KEY-----"
            ),
            r#"{"type":"service_account","project_id":"my-project","private_key":"key"}"#,
        ];
        for input in cases {
            let output = redact_text(input, "[REDACTED]");
            assert!(
                !output.contains(input),
                "shape survived: {input:?} => {output:?}"
            );
            assert!(
                output.contains("[REDACTED]"),
                "missing marker for {input:?}"
            );
        }
    }

    #[test]
    fn exact_credential_names_mask_arbitrary_json_values() {
        for key in [
            "api_key",
            "apiKey",
            "api-key",
            "access_token",
            "refresh_token",
            "client_secret",
            "password",
            "secret",
            "token",
            "auth_token",
            "private_key",
            "authorization",
            "cookie",
            "credentials",
            "oauth_token",
        ] {
            assert!(is_credential_key(key), "not recognized: {key}");
        }
        let safe = redact_json(
            json!({"api_key":"abc123", "Authorization":"Basic arbitrary", "Cookie":"session=abc123", "credentials":"inline", "oauth_token":"oauth-value", "message":"token budget"}),
            "[REDACTED]",
        );
        assert_eq!(safe["api_key"], "[REDACTED]");
        assert_eq!(safe["Authorization"], "[REDACTED]");
        assert_eq!(safe["Cookie"], "[REDACTED]");
        assert_eq!(safe["credentials"], "[REDACTED]");
        assert_eq!(safe["oauth_token"], "[REDACTED]");
        assert_eq!(safe["message"], "token budget");
    }

    #[test]
    fn usage_counters_survive_credential_field_redaction_in_every_sink() {
        let usage_keys = [
            "input_tokens",
            "output_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
            "max_tokens",
            "token_count",
            "tokens_used",
            "prompt_tokens",
            "completion_tokens",
            "total_tokens",
        ];
        let credential_keys = [
            "api_key",
            "provider_api_key",
            "access_token",
            "refresh_token",
            "client_secret",
            "password",
            "authorization",
            "cookie",
        ];

        for key in usage_keys {
            let value = redact_json(json!({key: 19}), "[REDACTED]");
            assert_eq!(value[key], 19, "usage counter was redacted: {key}");

            let wire = format!(r#"{{"{key}":19}}"#);
            assert_eq!(redact_text(&wire, "[REDACTED]"), wire, "{key}");
        }

        for key in credential_keys {
            let value = redact_json(json!({key: "ordinary-value"}), "[REDACTED]");
            assert_eq!(value[key], "[REDACTED]", "credential key survived: {key}");

            let wire = format!(r#"{{"{key}":"ordinary-value"}}"#);
            assert!(
                redact_text(&wire, "[REDACTED]").contains("[REDACTED]"),
                "credential field survived: {key}"
            );
        }
    }

    #[test]
    fn issue_444_baseline_regressions_are_covered_before_repair() {
        for input in [
            "Bearer abcdef",
            "Bearer authentication",
            "provider_api_key=abc123",
            "x_auth_token=abc123",
            "apikey=abc123",
            "api-key=abc123",
            "sk-abc",
        ] {
            assert_ne!(redact_text(input, "[REDACTED]"), input, "{input}");
        }
        assert!(redact_text("Bearer authentication", "[REDACTED]").contains("[REDACTED]"));
        for key in ["provider_api_key", "x_auth_token"] {
            let value = redact_json(json!({key:"abc123"}), "[REDACTED]");
            assert_eq!(value[key], "[REDACTED]", "{key}");
        }
        for input in [
            "key=alpha",
            "token budget",
            "password reset flow",
            "the secret is nearby",
            "$HOME $PATH ${VAR}",
        ] {
            assert_eq!(redact_text(input, "[REDACTED]"), input, "{input}");
        }
    }

    #[test]
    fn baseline_shape_union_and_structured_key_substrings_are_masked() {
        // Inputs copied from redaction.rs, redaction_patterns_smoke.rs, and
        // transport_evidence_redaction.rs at afd24174c.
        let baseline = [
            concat!("sk", "-abcdefghijklmnopqrst0000"),
            concat!("sk", "-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"),
            concat!("AK", "IAZZZZZZZZZZZZZZZZ"),
            concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gh", "o_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("sk", "_live_abcdefghijklmnopqrstuvwx"),
            concat!("pk", "_live_abcdefghijklmnopqrstuvwx"),
            concat!("ey", "JhbGciOiJIUzI1NiIs.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
            "Bearer abc123",
            "Bearer abcdef",
            r#"{"type":"service_account","private_key":"key"}"#,
            concat!(
                "--",
                "---BEGIN PRIVATE KEY-----\\nabc\\n-----END PRIVATE KEY-----"
            ),
            concat!(
                "--",
                "---BEGIN RSA PRIVATE KEY-----\\nabc\\n-----END RSA PRIVATE KEY-----"
            ),
            concat!(
                "--",
                "---BEGIN EC PRIVATE KEY-----\\nabc\\n-----END EC PRIVATE KEY-----"
            ),
            "sk-abc",
            r#"{"access_token":"private-value"}"#,
            r#"{"api_key":"abc123"}"#,
            "apikey=abc123",
            "api-key=abc123",
            "x-api-key: abc123",
        ];
        for input in baseline {
            let output = redact_text(input, "[REDACTED]");
            assert!(
                !output.contains(input),
                "baseline input survived: {input:?}"
            );
        }
        for key in ["provider_api_key", "x_auth_token", "client_secret"] {
            let value = redact_json(json!({key:"abc123"}), "[REDACTED]");
            assert_eq!(value[key], "[REDACTED]", "baseline key {key}");
        }
    }
}
