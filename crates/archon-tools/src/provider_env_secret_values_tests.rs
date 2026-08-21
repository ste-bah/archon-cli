use super::is_redactable;

/// The live failures. Each of these values was resolved into the provider
/// environment, and the old rule replaced every occurrence of it in every
/// command's output an agent read.
#[test]
fn non_credential_config_never_rewrites_output() {
    // A run id: appears in every path of the run by construction, so redacting
    // it handed agents directory names that do not exist.
    assert!(!is_redactable(
        "AHDM_REVIEW_RUN_ID",
        "wf-4ab9bac7-62bc-4593-835a-1686bab558fd"
    ));
    // A port: every "6900" anywhere in output, including inside larger numbers.
    assert!(!is_redactable("OPENBB_PORT", "6900"));
    assert!(!is_redactable("TV_CDP_PORT", "9222"));
    // A loopback address.
    assert!(!is_redactable("OPENBB_HOST", "127.0.0.1"));
    assert!(!is_redactable("TV_CDP_HOST", "127.0.0.1"));
}

#[test]
fn credential_named_keys_are_still_redacted() {
    assert!(is_redactable(
        "ARCHON_TEST_PROVIDER_KEY",
        "s3cr3t-value-xyz"
    ));
    assert!(is_redactable("POLYGON_API_KEY", "abcdefghijklmnop"));
    assert!(is_redactable("DB_PASSWORD", "hunter2hunter2"));
    assert!(is_redactable(
        "SERVICE_ACCOUNT_CREDENTIALS",
        "opaque-blob-1234"
    ));
    // A key that is itself a UUID is a real credential, not an identifier;
    // only the key name distinguishes it from a run id.
    assert!(is_redactable(
        "PROVIDER_API_KEY",
        "550e8400-e29b-41d4-a716-446655440000"
    ));
}

#[test]
fn a_credential_prefix_is_redacted_under_any_key_name() {
    assert!(is_redactable("PROVIDER_CONFIG", "sk-abcdefghijklmnop"));
    assert!(is_redactable("CI_VALUE", "ghp_abcdefghijklmnopqrst"));
    assert!(is_redactable("AWS_ACCESS", "AKIAIOSFODNN7EXAMPLE"));
}

#[test]
fn short_values_are_never_replaced() {
    // Too short to protect, long enough to collide with unrelated output.
    assert!(!is_redactable("API_KEY", "abc"));
    assert!(!is_redactable("AUTH_TOKEN", "1234567"));
    assert!(!is_redactable("API_KEY", ""));
}

/// A path or URL under a credential-named key names *where* a secret lives.
/// Rewriting it breaks the agent's next file read.
#[test]
fn paths_and_urls_survive_even_under_a_credential_key() {
    assert!(!is_redactable(
        "SSH_PRIVATE_KEY_PATH",
        "/home/user/.ssh/id_rsa"
    ));
    assert!(!is_redactable(
        "TOKEN_URL",
        "https://auth.example.com/token"
    ));
    assert!(!is_redactable("KEYSTORE", "./config/keystore.jks"));
    assert!(!is_redactable("SECRET_DIR", "~/secrets/provider"));
}

/// Regression: the first version of this filter exempted every value
/// containing "://" as structural, which leaked the most common form a secret
/// takes in an environment variable — a connection string with userinfo.
#[test]
fn connection_strings_with_embedded_passwords_are_redacted() {
    assert!(is_redactable(
        "DATABASE_URL",
        "postgres://user:hunter2pass@db.host/app"
    ));
    assert!(is_redactable(
        "REDIS_DSN",
        "redis://:s3cr3tpassword@127.0.0.1:6379/0"
    ));
    // Key name is irrelevant — the value itself carries the credential.
    assert!(is_redactable(
        "PROVIDER_ENDPOINT",
        "amqp://svc:pw0rd1234@broker.internal:5672"
    ));
}

/// A URL with no password, or with a bare username, is still structural.
#[test]
fn plain_urls_remain_structural() {
    assert!(!is_redactable(
        "TOKEN_URL",
        "https://auth.example.com/token"
    ));
    assert!(!is_redactable(
        "API_KEY_URL",
        "https://user@example.com/keys"
    ));
    assert!(!is_redactable("OPENBB_URL", "http://127.0.0.1:6900/api"));
}

/// Regression: "all digits and dots" exempted numeric secrets, not just
/// addresses. Only a real dotted quad is structural.
#[test]
fn numeric_secrets_are_redacted_but_addresses_are_not() {
    assert!(is_redactable("ACCOUNT_PIN", "192837465566"));
    assert!(is_redactable("CARD_SECRET", "40000000000000000002"));
    assert!(!is_redactable("SECRET_HOST", "192.168.100.14"));
}

/// Regression: short credential words were missing from the marker list, so
/// keys naming real credentials were treated as ordinary config.
#[test]
fn short_credential_words_are_recognised() {
    for key in [
        "OPENBB_PAT",
        "GH_BEARER",
        "APP_JWT",
        "HMAC_SALT",
        "WALLET_SEED",
        "REQUEST_NONCE",
        "LOGIN_OTP",
    ] {
        assert!(
            is_redactable(key, "abcdefghijklmnopqrst"),
            "{key} names a credential"
        );
    }
}

/// ...but matching those short words by suffix would recreate the original
/// over-redaction bug.
#[test]
fn short_words_never_match_as_a_suffix() {
    assert!(!is_redactable("BUILD_COMPAT", "abcdefghijklmnopqrst"));
    assert!(!is_redactable("SPIN_COUNT", "abcdefghijklmnopqrst"));
}

/// A key word that merely *contains* a marker is not a credential. Matching
/// `AUTH` inside `AUTHOR` would rewrite every commit author in git output —
/// the same corruption the filter exists to prevent.
#[test]
fn a_marker_inside_an_unrelated_word_is_not_a_credential() {
    assert!(!is_redactable("GIT_AUTHOR_NAME", "Steven Bahia-Longbottom"));
    assert!(!is_redactable("GIT_AUTHOR_EMAIL", "steven@example.com"));
    assert!(!is_redactable("KEYSTORE_DIR", "config-keystore-dir"));
}

#[test]
fn plural_credential_words_are_recognised() {
    assert!(is_redactable("PROVIDER_TOKENS", "abcdefghijklmnop"));
    assert!(is_redactable("AWS_CREDENTIALS", "abcdefghijklmnop"));
}

#[test]
fn separators_and_case_do_not_hide_a_credential_key() {
    assert!(is_redactable("api-key", "abcdefghijklmnop"));
    assert!(is_redactable("provider.secret", "abcdefghijklmnop"));
    assert!(is_redactable("Auth_Token", "abcdefghijklmnop"));
}
