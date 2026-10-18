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
        concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("gh", "o_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("gh", "u_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("gh", "s_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("gh", "r_abcdefghijklmnopqrstuvwxyz0123456789"),
        concat!("sk", "_live_abcdefghijklmnopqrstuvwx"),
        concat!("sk", "_test_abcdefghijklmnopqrstuvwx"),
        concat!("rk", "_live_abcdefghijklmnopqrstuvwx"),
        concat!("pk", "_live_abcdefghijklmnopqrstuvwx"),
        concat!("pk", "_test_abcdefghijklmnopqrstuvwx"),
        concat!("xo", "xb-1234567890-abcdef"),
        concat!("AS", "IA0123456789ABCDEF"),
        concat!("gi", "thub_pat_abcdefghijklmnopqrstuvwxyz0123456789"),
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
fn credential_formats_preserve_just_too_short_values_and_mask_real_lengths() {
    let cases = [
        (
            format!("sk-{}", "a".repeat(19)),
            format!("sk-{}", "a".repeat(20)),
        ),
        (
            format!("sk-ant-{}", "a".repeat(19)),
            format!("sk-ant-{}", "a".repeat(20)),
        ),
        (
            format!("sk-proj-{}", "a".repeat(19)),
            format!("sk-proj-{}", "a".repeat(20)),
        ),
        (
            format!("sk-svcacct-{}", "a".repeat(19)),
            format!("sk-svcacct-{}", "a".repeat(20)),
        ),
        (
            format!("AKIA{}", "A".repeat(15)),
            format!("AKIA{}", "A".repeat(16)),
        ),
        (
            format!("ASIA{}", "A".repeat(15)),
            format!("ASIA{}", "A".repeat(16)),
        ),
        (
            format!("github_pat_{}", "a".repeat(35)),
            format!("github_pat_{}", "a".repeat(36)),
        ),
        (
            format!("xoxb-{}", "a".repeat(9)),
            format!("xoxb-{}", "a".repeat(10)),
        ),
        (
            "eyJ123456789.123456789.123456789".to_string(),
            concat!("ey", "J1234567890.1234567890.1234567890").to_string(),
        ),
    ];
    for (short, real) in cases {
        assert_eq!(redact_text(&short, "[REDACTED]"), short, "{short}");
        assert_eq!(redact_text(&real, "[REDACTED]"), "[REDACTED]", "{real}");
    }
    for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        let short = format!("{prefix}{}", "a".repeat(35));
        let real = format!("{prefix}{}", "a".repeat(36));
        assert_eq!(redact_text(&short, "[REDACTED]"), short, "{prefix}");
        assert_eq!(redact_text(&real, "[REDACTED]"), "[REDACTED]", "{prefix}");
    }
    for prefix in [
        "sk_live_", "sk_test_", "pk_live_", "pk_test_", "rk_live_", "rk_test_",
    ] {
        let short = format!("{prefix}{}", "a".repeat(23));
        let real = format!("{prefix}{}", "a".repeat(24));
        assert_eq!(redact_text(&short, "[REDACTED]"), short, "{prefix}");
        assert_eq!(redact_text(&real, "[REDACTED]"), "[REDACTED]", "{prefix}");
    }
    for text in ["sk-short", "ghp_example"] {
        assert_eq!(redact_text(text, "[REDACTED]"), text);
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
        "provider_api_key=abc123",
        "x_auth_token=abc123",
        "apikey=abc123",
        "api-key=abc123",
    ] {
        assert_ne!(redact_text(input, "[REDACTED]"), input, "{input}");
    }
    for input in ["sk-abc", "sk-short", "ghp_example"] {
        assert_eq!(redact_text(input, "[REDACTED]"), input, "{input}");
    }
    for prose in [
        "Bearer tokens",
        "Bearer authentication",
        "Bearer token",
        "Bearer auth",
    ] {
        assert_eq!(redact_text(prose, "[REDACTED]"), prose);
    }
    for bearer in ["Bearer abc123", "Bearer x.y.z", "Bearer abcdefghijklmnop"] {
        assert!(
            redact_text(bearer, "[REDACTED]").contains("[REDACTED]"),
            "{bearer}"
        );
    }
    let prd = "Secrets, API keys, bearer tokens, cookies, and session IDs";
    assert_eq!(redact_text(prd, "[REDACTED]").as_bytes(), prd.as_bytes());
    for token in [
        concat!(
            "Bearer ey",
            "JhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY.abcdefghijklmnop"
        ),
        concat!("Bearer sk", "-ant-api03-abcdefghijklmnopqrstuvwxyz123456"),
    ] {
        assert!(
            redact_text(token, "[REDACTED]").contains("[REDACTED]"),
            "{token}"
        );
    }
    for key in ["provider_api_key", "x_auth_token"] {
        let value = redact_json(json!({key:"abc123"}), "[REDACTED]");
        assert_eq!(value[key], "[REDACTED]", "{key}");
    }
    for input in [
        r#"{"provider.apiKey":"abc123"}"#,
        r#"{"auth.token":"x"}"#,
        "'db.password': 'p'",
        "provider.apiKey=abc123",
    ] {
        let output = redact_text(input, "[REDACTED]");
        assert!(
            output.contains("[REDACTED]"),
            "not masked: {input} -> {output}"
        );
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
fn api_key_header_values_are_redacted_without_masking_header_names_in_prose() {
    assert_eq!(
        redact_text("invalid x-api-key", "[REDACTED]"),
        "invalid x-api-key"
    );
    assert_eq!(
        redact_text("x-api-key: abc123", "[REDACTED]"),
        "x-api-key: [REDACTED]"
    );
    assert_eq!(
        redact_text("api_key=abc123", "[REDACTED]"),
        "api_key=[REDACTED]"
    );
    assert_eq!(
        redact_text(
            concat!("curl -H x-api-key sk", "-abcdefghijklmnopqrst"),
            "[REDACTED]"
        ),
        "curl -H x-api-key [REDACTED]"
    );
}

#[test]
fn issue_444_credential_carriers_mask_values_without_corrupting_headers_or_prose() {
    let carriers = [
        (
            "authorization basic",
            "Authorization: Basic abc123",
            "abc123",
        ),
        (
            "proxy authorization",
            "Proxy-Authorization: Basic abc123",
            "abc123",
        ),
        ("authorization raw", "Authorization: abc123", "abc123"),
        ("key equals", "api_key=abc123", "abc123"),
        ("key colon", "password: abc123", "abc123"),
        ("json", r#"{"token":"abc123"}"#, "abc123"),
        ("yaml", "password: yamlsecret", "yamlsecret"),
        ("env export", "export API_KEY=envsecret", "envsecret"),
        (
            "url userinfo",
            "https://user:pass123@example.test",
            "pass123",
        ),
        (
            "query token",
            "https://example.test/?token=querysecret",
            "querysecret",
        ),
        (
            "query api key",
            "https://example.test/?x=1&api_key=querysecret",
            "querysecret",
        ),
        ("single quote assignment", "token='hunter2'", "hunter2"),
        ("single quote yaml", "password: 'hunter2'", "hunter2"),
        ("single quote spaced", "api_key = 'xsecret'", "xsecret"),
        ("backtick assignment", "token=`ticksecret`", "ticksecret"),
    ];
    for (label, input, secret) in carriers {
        let output = redact_text(input, "[REDACTED]");
        assert!(!output.contains(secret), "{label} leaked: {output:?}");
        assert!(
            output.contains("[REDACTED]"),
            "{label} lacks marker: {output:?}"
        );
    }
    for (input, header) in [
        ("Authorization: Basic abc123", "Authorization:"),
        ("Proxy-Authorization: Basic abc123", "Proxy-Authorization:"),
    ] {
        assert!(
            redact_text(input, "[REDACTED]").contains(header),
            "header name was lost for {input:?}"
        );
    }
    for prose in ["the token's lifetime", "for token in a b"] {
        assert_eq!(redact_text(prose, "[REDACTED]"), prose);
    }
}

#[test]
fn issue_444_keys_are_whole_tokens_and_colon_prose_requires_secret_shape() {
    for prose in [
        "Token: The request succeeded",
        "src/token.rs:12: error",
        "see secret_store.py:40",
        "docs/api_key.md",
        "Secrets, API keys, bearer tokens, cookies, and session IDs",
    ] {
        assert_eq!(redact_text(prose, "[REDACTED]"), prose, "{prose}");
    }
    for (input, secret) in [
        ("password: hunter2", "hunter2"),
        ("token: abc", "abc"),
        ("Password: Tr0ub4dor&3", "Tr0ub4dor&3"),
        ("api_key=xyz", "xyz"),
        ("TOKEN=abcdef", "abcdef"),
    ] {
        let output = redact_text(input, "[REDACTED]");
        assert!(!output.contains(secret), "leaked {secret}: {output}");
        assert!(output.contains("[REDACTED]"), "not masked: {output}");
    }
}

#[test]
fn issue_444_common_case_styles_redact_without_masking_prose() {
    for input in [
        r#"{"apiKey":"abc123"}"#,
        r#"{"accessToken":"x"}"#,
        r#"{"ClientSecret":"abc"}"#,
        "apiKey: abc123",
        "AccessToken: abc",
        "X-Api-Key: abc123",
        "Proxy-Authorization: Basic abc",
    ] {
        let output = redact_text(input, "[REDACTED]");
        assert!(
            output.contains("[REDACTED]"),
            "not masked: {input} => {output}"
        );
        for secret in ["abc123", "abc", "Basic abc"] {
            if input.contains(secret) {
                assert!(!output.contains(secret), "leaked: {input} => {output}");
            }
        }
    }

    for prose in [
        "Token: The request succeeded",
        "Authentication: required",
        "**Secret:** keep it safe",
    ] {
        assert_eq!(redact_text(prose, "[REDACTED]"), prose, "{prose}");
    }
}

#[test]
fn quoted_capitalised_keys_apply_credential_shape_check() {
    for prose in [
        r#"Token: "The request succeeded""#,
        r#"Token: "please retry the request""#,
    ] {
        assert_eq!(redact_text(prose, "[REDACTED]"), prose, "{prose}");
    }
    for credential in [r#"Token: "abc123""#, r#"Token: "x.y.z""#] {
        assert!(
            redact_text(credential, "[REDACTED]").contains("[REDACTED]"),
            "{credential}"
        );
    }
    for input in [
        r#"token: "ordinary prose""#,
        r#"TOKEN: "ordinary prose""#,
        r#"api_key: "ordinary prose""#,
    ] {
        assert!(
            redact_text(input, "[REDACTED]").contains("[REDACTED]"),
            "{input}"
        );
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
