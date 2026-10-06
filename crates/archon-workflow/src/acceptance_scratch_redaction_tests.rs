use super::*;

fn check_json(secret: &str) {
    let environment = BTreeMap::from([("SERVICE_KEY".into(), secret.into())]);
    let output = serde_json::to_vec(secret).unwrap();
    assert_eq!(redact(&environment, &output, false), b"\"[REDACTED]\"");
}
#[test]
fn json_quotes_do_not_escape_probe_cache_redaction() {
    check_json("credential-quote\"-canary");
}
#[test]
fn json_backslashes_do_not_escape_probe_cache_redaction() {
    check_json("credential-backslash\\-canary");
}
#[test]
fn json_newlines_do_not_escape_probe_cache_redaction() {
    check_json("credential-newline\n-canary");
}

#[test]
fn bare_url_passwords_are_removed_from_probe_cache_output() {
    for (address, password) in [
        ("postgres://user:small@host/db", "small"),
        (
            "https://user:escaped%40password@host/api",
            "escaped@password",
        ),
        ("http://user:proxy-password@proxy:8080", "proxy-password"),
    ] {
        let environment = BTreeMap::from([("DATABASE_URL".into(), address.into())]);
        assert_eq!(
            redact(&environment, password.as_bytes(), false),
            b"[REDACTED]"
        );
    }
}
