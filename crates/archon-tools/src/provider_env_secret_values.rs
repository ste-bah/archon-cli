// Which resolved env values may be blind-replaced in agent-visible output.
//
// Redaction rewrites *every* occurrence of a value in a command's output. That
// is correct for a credential and destructive for anything else, and the old
// rule — replace every resolved value that is not the empty string — made no
// distinction. Observed live: a resolved service port of `6900` rewrote every
// "6900" an agent ever read (byte counts, line numbers, timestamps) as a
// redaction marker, and a workflow run id resolved into the
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
    "BEARER",
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
    "SIGNING",
    "TOKEN",
];

/// Short credential words that must match a key word EXACTLY. Suffix-matching
/// these would be worse than useless: `PAT` would make `COMPAT` a credential,
/// which is the same over-redaction this filter exists to prevent.
const SECRET_KEY_WORDS: &[&str] = &[
    "DSN", "HMAC", "JWT", "NONCE", "OTP", "PAT", "PIN", "SALT", "SEED",
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
    if has_credential_prefix(value) || url_carries_a_password(value) {
        return true;
    }
    key_names_a_credential(key) && !is_structural_value(value)
}

/// A connection string with userinfo — `postgres://user:pw@host/db`,
/// `redis://:pw@host` — IS the credential, whatever its key is called. This is
/// the single most common way a secret reaches an environment variable, and
/// treating every `://` value as structural leaked every one of them.
fn url_carries_a_password(value: &str) -> bool {
    let Some((_scheme, rest)) = value.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let Some((userinfo, _host)) = authority.rsplit_once('@') else {
        return false;
    };
    // `user@host` carries no secret; `user:pw@host` and `:pw@host` do.
    userinfo
        .split_once(':')
        .is_some_and(|(_, pw)| !pw.is_empty())
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
        || SECRET_KEY_WORDS.contains(&singular)
}

/// Paths, plain URLs and addresses are load-bearing in output: they name files
/// the agent is about to open and hosts it is about to reach. Rewriting one
/// breaks the agent's next action, so they are never replaced — not even under
/// a credential-named key, where they locate a secret rather than being one
/// (`SSH_PRIVATE_KEY_PATH`, `TOKEN_URL`).
///
/// A URL carrying a password is NOT structural; `url_carries_a_password`
/// catches it before this is consulted.
fn is_structural_value(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.contains("://")
        || is_ipv4_address(value)
}

/// Dotted-quad only. The old rule exempted *any* digits-and-dots string, which
/// silently spared numeric secrets — an eight-digit PIN under `ACCOUNT_PIN` was
/// treated as structural and printed verbatim.
fn is_ipv4_address(value: &str) -> bool {
    let mut octets = 0;
    for part in value.split('.') {
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        octets += 1;
    }
    octets == 4
}

#[cfg(test)]
#[path = "provider_env_secret_values_tests.rs"]
mod tests;
