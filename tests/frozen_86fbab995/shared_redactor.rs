//! Shared credential redaction for logs, workflow records, and transport samples.
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde_json::Value;

static SHAPES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r#"(?i)(?:\{[^{}]*"type"\s*:\s*"service_account"[\s\S]*?\}|"#,
        r"-----BEGIN\s(?:RSA\s|EC\s|)?PRIVATE\sKEY-----[\s\S]*?-----END\s(?:RSA\s|EC\s|)?PRIVATE\sKEY-----|",
        r"(?:sk|pk|rk)_(?:live|test)_[A-Za-z0-9]{24,}|",
        r"\bsk-ant-[A-Za-z0-9_-]{20,}|\bsk-(?:proj|svcacct)-[A-Za-z0-9_-]{20,}|\bsk-[A-Za-z0-9]{20,}|",
        r"gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{36,}|",
        r"(?:AKIA|ASIA)[0-9A-Z]{16}|xox[abpr]-[A-Za-z0-9-]{10,}|",
        r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}|",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----)"
    ))
    .expect("credential shape regex is constant")
});
// Exact held-value matching and fixed shapes cannot recover a credential
// deliberately split between fields, or identify an arbitrary base64 encoding.
static USERINFO: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)([a-z][a-z0-9+.-]*://[^:/@\s]+:)[^@/\s]+(@)").unwrap());
static AUTH_HEADER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)((?:authorization|proxy-authorization)\s*:\s*)[^\r\n]*").unwrap()
});
static VALUE_HEADER: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)((?:x-api-key|api-key|cookie|set-cookie)\s*:\s*)[^\r\n]*").unwrap()
});
static QUOTED_CREDENTIAL_FIELD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r#"(?i)(^|[\s"'({,;&?])(["']?[A-Za-z0-9_.-]*(?:api[_-]?key|apikey|token|secret|password|passwd|authorization|credential|private[_-]?key|cookie|thinking|reasoning|raw_text|encrypted_reasoning)[A-Za-z0-9_.-]*["']?\s*[:=]\s*)(?:"(?:\\.|[^"\\])*"|'[^'\r\n]*'|`[^`\r\n]*`)"#
    ))
    .unwrap()
});
static TRUNCATED_CREDENTIAL_FIELD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r#"(?i)(^|[\s"'({,;&?])(["']?[A-Za-z0-9_.-]*(?:api[_-]?key|apikey|token|secret|password|passwd|authorization|credential|private[_-]?key|cookie|thinking|reasoning|raw_text|encrypted_reasoning)[A-Za-z0-9_.-]*["']?\s*:\s*["'])[^"\r\n]*$"#
    ))
    .unwrap()
});
static CREDENTIAL_FIELD_WITH_VALUE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)(^|[\s"'({,;&?])(["']?[A-Za-z0-9_.-]*(?:api[_-]?key|apikey|token|secret|password|passwd|authorization|credential|private[_-]?key|cookie|thinking|reasoning|raw_text|encrypted_reasoning)[A-Za-z0-9_.-]*["']?\s*[:=]\s*)([^"'\s,;}]+)"#,
    )
    .unwrap()
});
static BEARER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(\bbearer\s+)([A-Za-z0-9._~+/=-]+)").unwrap());
/// Redact held values, credential carriers, and the documented fixed formats.
pub fn redact_text(value: &str, marker: &str) -> String {
    redact_text_inner(value, marker)
}

/// Redact credential values without masking field-name words in paths or prose.
pub fn redact_secret_values(value: &str, marker: &str) -> String {
    redact_text_inner(value, marker)
}

fn redact_text_inner(value: &str, marker: &str) -> String {
    let mut out = crate::secret_values::redact_registered(value);
    out = SHAPES.replace_all(&out, marker).into_owned();
    out = USERINFO
        .replace_all(&out, |c: &Captures<'_>| {
            format!("{}{}{}", &c[1], marker, &c[2])
        })
        .into_owned();
    out = AUTH_HEADER
        .replace_all(&out, |c: &Captures<'_>| format!("{}{}", &c[1], marker))
        .into_owned();
    out = VALUE_HEADER
        .replace_all(&out, |c: &Captures<'_>| format!("{}{}", &c[1], marker))
        .into_owned();
    out = TRUNCATED_CREDENTIAL_FIELD
        .replace_all(&out, |c: &Captures<'_>| {
            if is_credential_field_prefix(&c[2], "") {
                format!("{}{}{}", &c[1], &c[2], marker)
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
            let matched = &c[0];
            let rest = matched
                .strip_prefix(&c[1])
                .and_then(|tail| tail.strip_prefix(&c[2]))
                .unwrap_or_default();
            let quote = rest.chars().next().unwrap_or('"');
            let quoted_value = rest.to_string();
            if !is_credential_field_prefix(&c[2], &quoted_value)
                || (c[2].contains('=') && (c[0].contains('$') || shell_header))
            {
                c[0].to_string()
            } else {
                format!("{}{}{}{}{}", &c[1], &c[2], quote, marker, quote)
            }
        })
        .into_owned();
    let input = out;
    out = CREDENTIAL_FIELD_WITH_VALUE
        .replace_all(&input, |c: &Captures<'_>| {
            let start = c.get(0).unwrap().start();
            let line = input[..start].rsplit('\n').next().unwrap_or_default();
            let shell_header = line.contains(" -H ") || line.trim_start().starts_with("curl ");
            if !is_credential_field_prefix(&c[2], &c[3])
                || (c[2].contains('=') && (shell_header || c[3].contains('$')))
            {
                c[0].to_string()
            } else {
                format!("{}{}{}", &c[1], &c[2], marker)
            }
        })
        .into_owned();
    // Outside authorization headers, mask only bearer values with credential
    // shape. Ordinary prose such as "Bearer authentication" stays intact.
    out = BEARER
        .replace_all(&out, |c: &Captures<'_>| {
            let token = &c[2];
            let has_digit_or_separator = token.chars().any(|ch| {
                ch.is_ascii_digit() || matches!(ch, '.' | '_' | '~' | '+' | '/' | '=' | '-')
            });
            if has_digit_or_separator || token.len() >= 16 {
                format!("{}{marker}", &c[1])
            } else {
                c[0].to_string()
            }
        })
        .into_owned();
    out
}

/// Apply the shared rules recursively to parsed JSON, redacting exact credential keys.
pub fn redact_json(value: Value, marker: &str) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let value = if is_credential_field(&key, &value) {
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

/// Remove credential-named fields recursively using the same value-aware rule as redaction.
pub fn remove_credential_fields(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(key, value)| !is_credential_field(key, value))
                .map(|(key, value)| (key, remove_credential_fields(value)))
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(remove_credential_fields).collect())
        }
        other => other,
    }
}

/// Whether a structured key and value should be treated as a credential carrier.
/// The four exact usage-count names are safe only when their value is numeric or null.
pub fn is_credential_field(key: &str, value: &Value) -> bool {
    if is_exact_usage_counter_key(key) && (value.is_number() || value.is_null()) {
        return false;
    }
    is_credential_key(key)
}

fn is_exact_usage_counter_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "tokens_in" | "tokens_out" | "tokensin" | "tokensout"
    )
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

fn is_credential_field_prefix(prefix: &str, value: &str) -> bool {
    let quoted_name = prefix.trim_start().starts_with(['"', '\'']);
    let name = prefix
        .trim()
        .trim_matches(['"', '\'', '{', '['])
        .trim_end_matches([':', '=']);
    let name = name
        .trim()
        .trim_matches(['"', '\'', '{', '['])
        .trim_start_matches(['?', '&']);
    if name.is_empty()
        || name.contains(['/', '\\'])
        || (name.contains('.')
            && !quoted_name
            && !prefix.trim_end_matches(char::is_whitespace).ends_with('='))
        || name.chars().any(char::is_whitespace)
    {
        return false;
    }
    if !is_credential_key(name) {
        return false;
    }

    let separator = prefix.trim_end_matches(char::is_whitespace);
    if separator.ends_with('=') {
        return is_credential_field_text(name, value);
    }
    if separator.ends_with(':') {
        if quoted_name {
            return is_credential_field_text(name, value);
        }
        if colon_key_is_identifier(name) {
            return is_credential_field_text(name, value);
        }
        if is_capitalised_word(name) {
            return credential_shaped_unquoted(value);
        }
        return false;
    }
    if value.starts_with(['"', '\'', '`']) {
        return is_credential_field_text(name, value);
    }
    is_credential_field_text(name, value)
}

fn credential_shaped_unquoted(value: &str) -> bool {
    if value.starts_with(['"', '\'', '`']) {
        let Some(quote) = value.chars().next() else {
            return false;
        };
        let value = value
            .strip_prefix(quote)
            .and_then(|value| value.strip_suffix(quote))
            .unwrap_or(value);
        if value.chars().any(char::is_whitespace) {
            return false;
        }
        credential_shaped(value)
    } else {
        credential_shaped(value)
    }
}

fn colon_key_is_identifier(key: &str) -> bool {
    let chars: Vec<char> = key.chars().collect();
    let established_style = chars
        .iter()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '_')
        || chars
            .iter()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || *ch == '_')
        || chars
            .iter()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || *ch == '-');
    let uppercase_humps = chars.iter().filter(|ch| ch.is_ascii_uppercase()).count();
    let mixed_case_identifier = chars
        .iter()
        .all(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        && ((chars.first().is_some_and(|ch| ch.is_ascii_lowercase()) && uppercase_humps > 0)
            || uppercase_humps >= 2);

    established_style || mixed_case_identifier
}

fn is_capitalised_word(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_uppercase())
        && chars.all(|ch| ch.is_ascii_lowercase())
}

fn credential_shaped(value: &str) -> bool {
    let value = value.trim();
    value
        .chars()
        .any(|ch| ch.is_ascii_digit() || (!ch.is_ascii_alphanumeric() && !ch.is_whitespace()))
        || (value.len() >= 16
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
}

/// Text fields have the same four exact usage-counter exceptions as JSON;
/// only an unquoted integer literal is safe in text.
pub fn is_credential_field_text(key: &str, value: &str) -> bool {
    if is_exact_usage_counter_key(key) {
        return !is_plain_integer(value);
    }
    is_credential_key(key)
}

fn is_plain_integer(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
#[path = "shared_redactor_usage_tests.rs"]
mod usage_tests;

#[cfg(test)]
#[path = "shared_redactor_tests.rs"]
mod tests;
