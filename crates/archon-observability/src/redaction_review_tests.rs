#[test]
fn issue_444_review_corpus_distinguishes_credentials_from_prose() {
    let must_mask = [
        (
            "short header bearer",
            "Authorization: Bearer abc123",
            "abc123",
        ),
        ("proxy auth", "Proxy-Authorization: Basic abc123", "abc123"),
        (
            "authorization raw scheme",
            "Authorization: token abc123",
            "abc123",
        ),
        ("api key header", "api-key: abc123", "abc123"),
        ("cookie header", "Cookie: session=abc123", "abc123"),
        ("set cookie header", "Set-Cookie: session=abc123", "abc123"),
        (
            "url userinfo",
            "https://alice:pass123@example.test",
            "pass123",
        ),
        (
            "query token",
            "https://example.test/path?token=querysecret",
            "querysecret",
        ),
        (
            "query api key",
            "https://example.test/path?x=1&api_key=querysecret",
            "querysecret",
        ),
        ("env export", "export API_KEY=envsecret", "envsecret"),
        ("yaml", "password: yamlsecret", "yamlsecret"),
        ("single quote assignment", "token='hunter2'", "hunter2"),
        ("single quote yaml", "password: 'hunter2'", "hunter2"),
        ("single quote api key", "api_key = 'xsecret'", "xsecret"),
        (
            "backtick assignment",
            "token=`backticksecret`",
            "backticksecret",
        ),
        ("x api key header", "x-api-key: abc123", "abc123"),
        ("stringified json", r#"{"api_key":"abc123"}"#, "abc123"),
        ("apiKey json", r#"{"apiKey":"abc123"}"#, "abc123"),
        (
            "access token json",
            r#"{"access_token":"abc123"}"#,
            "abc123",
        ),
        (
            "refresh token json",
            r#"{"refresh_token":"abc123"}"#,
            "abc123",
        ),
        (
            "client secret json",
            r#"{"client_secret":"abc123"}"#,
            "abc123",
        ),
        ("password json", r#"{"password":"abc123"}"#, "abc123"),
        ("secret json", r#"{"secret":"abc123"}"#, "abc123"),
        ("token json", r#"{"token":"abc123"}"#, "abc123"),
        ("auth token json", r#"{"auth_token":"abc123"}"#, "abc123"),
        ("private key json", r#"{"private_key":"abc123"}"#, "abc123"),
        ("x api key json", r#"{"x-api-key":"abc123"}"#, "abc123"),
        (
            "pem",
            concat!(
                "--",
                "---BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----"
            ),
            "abc",
        ),
        (
            "openai",
            concat!("sk", "-abcdefghijklmnopqrst0000"),
            concat!("sk", "-abcdefghijklmnopqrst0000"),
        ),
        (
            "anthropic",
            concat!("sk", "-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"),
            concat!("sk", "-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"),
        ),
        (
            "aws",
            concat!("AK", "IAZZZZZZZZZZZZZZZZ"),
            "AKIAZZZZZZZZZZZZZZ",
        ),
        (
            "jwt",
            concat!("ey", "JhbGciOiJIUzI1NiI.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
            concat!("ey", "JhbGciOiJIUzI1NiI.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
        ),
        (
            "github oauth",
            concat!("gh", "o_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gh", "o_abcdefghijklmnopqrstuvwxyz0123456789"),
        ),
        (
            "github user",
            concat!("gh", "u_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gh", "u_abcdefghijklmnopqrstuvwxyz0123456789"),
        ),
        (
            "github server",
            concat!("gh", "s_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gh", "s_abcdefghijklmnopqrstuvwxyz0123456789"),
        ),
        (
            "github refresh",
            concat!("gh", "r_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gh", "r_abcdefghijklmnopqrstuvwxyz0123456789"),
        ),
        (
            "github fine grained",
            concat!("gi", "thub_pat_abcdefghijklmnopqrstuvwxyz0123456789"),
            concat!("gi", "thub_pat_abcdefghijklmnopqrstuvwxyz0123456789"),
        ),
        (
            "aws session key",
            concat!("AS", "IA0123456789ABCDEF"),
            concat!("AS", "IA0123456789ABCDEF"),
        ),
        (
            "xox",
            concat!("xo", "xb-1234567890-abcdef"),
            concat!("xo", "xb-1234567890-abcdef"),
        ),
    ];
    for (label, input, secret) in must_mask {
        let output = crate::shared_redactor::redact_text(input, "[REDACTED]");
        assert!(
            !output.contains(secret),
            "{label} leaked {secret:?}: {output:?}"
        );
        assert!(
            output.contains("[REDACTED]"),
            "{label} lacks marker: {output:?}"
        );
    }

    for ordinary in [
        "Bearer tokens",
        "Bearer authentication",
        "Bearer token",
        "Bearer auth",
        "key=alpha",
        "$HOME $PATH",
        "token budget",
        "the secret is nearby",
        "password reset flow",
        "token=\"$3\"",
        "curl -fsS -H token=\"x\" http://localhost/health",
        "the token's lifetime",
        "for token in a b",
    ] {
        assert_eq!(
            crate::shared_redactor::redact_text(ordinary, "[REDACTED]"),
            ordinary
        );
    }
    for token in [
        concat!(
            "Bearer ey",
            "JhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY.abcdefghijklmnop"
        ),
        concat!("Bearer sk", "-ant-api03-abcdefghijklmnopqrstuvwxyz123456"),
    ] {
        assert!(
            crate::shared_redactor::redact_text(token, "[REDACTED]").contains("[REDACTED]"),
            "{token}"
        );
    }
}
