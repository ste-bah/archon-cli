// Which resolved env values may be blind-replaced in agent-visible output.
//
// Redaction rewrites *every* occurrence of a value in a command's output. That
// is correct for a credential and destructive for anything else, and the old
// rule — replace every resolved value that is not the empty string — made no
// distinction. Observed live: a resolved `OPENBB_PORT=6900` rewrote every
// "6900" an agent ever read (byte counts, line numbers, timestamps) as
// `<redacted:OPENBB_PORT>`, and a workflow run id resolved into the
// environment rewrote the run's own paths, so agents were handed directory
// names that do not exist.
//
// So a value has to earn the replacement. It earns it by carrying a known
// credential prefix, or by living under a key that names a credential and not
// being an obviously structural string. Everything else — hosts, ports, run
// ids, paths — passes through untouched, because a value that cannot be
// protected by replacing it can still be destroyed by replacing it.

/// Words that mark a key as naming a credential. Matched as a *suffix* of a
/// key word, so `API_KEY` and `APIKEY` both match `KEY` while `GIT_AUTHOR_NAME`
/// does not match `AUTH` — plain substring matching would redact author names
/// and reintroduce exactly the corruption this filter exists to stop.
const SECRET_KEY_MARKERS: &[&str] = &[
    "AUTH",
    "CERT",
    "CREDENTIAL",
    "KEY",
    "PASSPHRASE",
    "PASSWD",
    "PASSWORD",
    "PRIVATE",
    "SECRET",
    "SESSION",
    "SIGNATURE",
    "TOKEN",
];

/// Literal prefixes used by issuers of opaque credentials. A value carrying
/// one of these is a credential whatever its key is called, so it is redacted
/// even under a key like `PROVIDER_CONFIG`.
const CREDENTIAL_VALUE_PREFIXES: &[&str] = &[
    "AKIA",
    "ASIA",
    "eyJ",
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "glpat-",
    "hf_",
    "npm_",
    "sk-",
    "sk_live_",
    "sk_test_",
    "xoxb-",
    "xoxp-",
];

/// Shorter than this and a replacement destroys far more than it protects: the
/// shorter the value, the more unrelated text collides with it.
const MIN_REDACTABLE_LEN: usize = 8;

/// True when replacing `value` throughout agent-visible output is warranted.
pub(crate) fn is_redactable(key: &str, value: &str) -> bool {
    if value.len() < MIN_REDACTABLE_LEN {
        return false;
    }
    if has_credential_prefix(value) {
        return true;
    }
    key_names_a_credential(key) && !is_structural_value(value)
}

fn has_credential_prefix(value: &str) -> bool {
    CREDENTIAL_VALUE_PREFIXES
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

fn key_names_a_credential(key: &str) -> bool {
    key.to_ascii_uppercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(word_names_a_credential)
}

fn word_names_a_credential(word: &str) -> bool {
    // `CREDENTIALS` and `TOKENS` name the same thing as their singulars.
    let singular = word.strip_suffix('S').unwrap_or(word);
    SECRET_KEY_MARKERS
        .iter()
        .any(|marker| singular.ends_with(marker))
}

/// Paths, URLs and bare numbers are load-bearing in output: they name files the
/// agent is about to open and ports it is about to reach. Rewriting one breaks
/// the agent's next action, so they are never replaced — not even under a
/// credential-named key, where they are a location for a secret rather than the
/// secret itself (`SSH_PRIVATE_KEY_PATH`, `TOKEN_URL`).
fn is_structural_value(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.contains("://")
        || value
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
}

#[cfg(test)]
#[path = "provider_env_secret_values_tests.rs"]
mod tests;
