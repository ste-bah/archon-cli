use std::io::Write;
use std::sync::{Arc, Mutex as StdMutex};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;

use super::*;

/// Shared in-memory writer, safe across threads.
#[derive(Clone)]
struct SharedBuf(Arc<StdMutex<Vec<u8>>>);

impl SharedBuf {
    fn new() -> Self {
        Self(Arc::new(StdMutex::new(Vec::new())))
    }

    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap_or_default()
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Every supported shape in `REDACTION_RE` must redact.
#[test]
fn redaction_layer_redacts_every_secret_shape() {
    let cases = [
        ("openai", concat!("sk", "-abcdefghijklmnopqrst0000")),
        (
            "github pat",
            concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
        ),
        (
            "anthropic",
            concat!("sk", "-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"),
        ),
        ("aws_akia", concat!("AK", "IAZZZZZZZZZZZZZZZZ")),
        (
            "github_pat",
            concat!("gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"),
        ),
        (
            "github_oauth",
            concat!("gh", "o_abcdefghijklmnopqrstuvwxyz0123456789"),
        ),
        (
            "stripe_secret_live",
            concat!("sk", "_live_abcdefghijklmnopqrstuvwx"),
        ),
        (
            "stripe_secret_test",
            concat!("sk", "_test_abcdefghijklmnopqrstuvwx"),
        ),
        (
            "stripe_publishable_live",
            concat!("pk", "_live_abcdefghijklmnopqrstuvwx"),
        ),
        (
            "stripe_restricted_live",
            concat!("rk", "_live_abcdefghijklmnopqrstuvwx"),
        ),
        (
            "jwt",
            concat!("ey", "JhbGciOiJIUzI1NiIs.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
        ),
        ("bearer", concat!("bearer ya", "29.a0Af_abcDEF-123")),
    ];

    for (label, secret) in cases {
        let sink = SharedBuf::new();
        let layer = RedactionLayer::with_writer(sink.clone());
        let subscriber = tracing_subscriber::registry().with(layer);
        ::tracing::subscriber::with_default(subscriber, || {
            ::tracing::info!(payload = secret, "emitting {label}");
        });
        let captured = sink.contents();
        assert!(
            captured.contains(REDACTED),
            "{label}: expected redaction marker, got: {captured:?}"
        );
        assert!(
            !captured.contains(secret),
            "{label}: raw secret leaked into sink: {captured:?}"
        );
    }
}

#[test]
fn credential_named_tracing_fields_mask_values_regardless_of_shape() {
    let sink = SharedBuf::new();
    let layer = RedactionLayer::with_writer(sink.clone());
    let subscriber = tracing_subscriber::registry().with(layer);
    ::tracing::subscriber::with_default(subscriber, || {
        ::tracing::info!(api_key = "abc123", "short credential value");
    });

    let captured = sink.contents();
    assert!(
        !captured.contains("abc123"),
        "credential value leaked: {captured:?}"
    );
    assert!(captured.contains(REDACTED), "missing marker: {captured:?}");
}

/// Sensitive FIELD NAMES (`password`, `api_key`, etc) must themselves be
/// masked so the identifier is never exposed to log sinks even when the
/// value is clean.
#[test]
fn redaction_layer_redacts_sensitive_field_names() {
    for name in [
        "password",
        "api_key",
        "api-key",
        "authorization",
        "secret",
        "token",
    ] {
        let redacted = redact_field_name(name);
        assert!(
            redacted.contains(REDACTED),
            "field name {name} should be masked; got {redacted:?}"
        );
    }
}

#[test]
fn free_text_redaction_preserves_english_and_masks_credential_shaped_bearers() {
    let prose = "Secrets, API keys, cookies, and session IDs must never be logged";
    let loop_script = r#"for token in alpha beta; do grep -qF -- "$token"; done"#;
    let secret_prose = "The word secret appears in prose, and token is a variable.";
    for text in [prose, loop_script, secret_prose] {
        assert_eq!(redact_text(text), text);
        assert_eq!(redact_secret_values(text), text);
    }

    for prose in [
        "Bearer tokens",
        "Bearer authentication",
        "Bearer token",
        "Bearer auth",
    ] {
        assert_eq!(redact_secret_values(prose), prose);
    }
    let prd = "Secrets, API keys, bearer tokens, cookies, and session IDs";
    assert_eq!(redact_secret_values(prd).as_bytes(), prd.as_bytes());

    for secret in [
        concat!("Bearer gh", "p_123456789012345678901234567890123456"),
        "Bearer token_value_1234567890123456",
        concat!(
            "sk",
            "-proj-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56"
        ),
    ] {
        let redacted = redact_secret_values(secret);
        assert!(redacted.contains(REDACTED), "missing marker: {redacted}");
        assert!(!redacted.contains(secret), "leaked credential: {redacted}");
    }
}

/// Exercise the production registry, filter, and redaction layer with a capturing writer.
#[test]
fn redaction_path_matches_production_stack() {
    let sink = SharedBuf::new();
    let filter = EnvFilter::new("info");
    let layer = RedactionLayer::with_writer_and_format(sink.clone(), false);

    let subscriber = tracing_subscriber::registry().with(filter).with(layer);
    ::tracing::subscriber::with_default(subscriber, || {
        ::tracing::info!(
            api_key = concat!("sk", "-abcdefghijklmnopqrst0000"),
            "emitting secret"
        );
    });

    let captured = sink.contents();
    assert!(
        captured.contains(REDACTED),
        "expected redaction marker in output, got: {captured:?}"
    );
    assert!(
        !captured.contains(concat!("sk", "-abcdefghijklmnopqrst0000")),
        "raw sk- secret leaked into sink: {captured:?}"
    );
}

/// JSON layout emits parseable structured output with redacted field values.
#[test]
fn redaction_layer_json_layout_is_parseable() {
    let sink = SharedBuf::new();
    let layer = RedactionLayer::with_writer_and_format(sink.clone(), true);
    let subscriber = tracing_subscriber::registry().with(layer);
    ::tracing::subscriber::with_default(subscriber, || {
        ::tracing::info!(user = "alice", "hello");
    });
    let captured = sink.contents();
    assert!(
        captured.contains("\"level\":\"INFO\""),
        "json missing level: {captured:?}"
    );
    assert!(
        captured.contains("\"target\":"),
        "json missing target: {captured:?}"
    );
    assert!(
        captured.contains("\"fields\":{"),
        "json missing fields obj: {captured:?}"
    );
}

#[test]
fn redacts_modern_openai_sk_proj_key() {
    let raw = concat!(
        "sk",
        "-proj-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56"
    );
    let out = redact(&format!("key={raw}"));
    assert!(
        out.contains("***REDACTED***"),
        "expected REDACTED marker, got: {out:?}"
    );
    assert!(!out.contains(raw), "raw sk-proj- key leaked: {out:?}");
}

#[test]
fn redacts_modern_openai_sk_svcacct_key() {
    let raw = concat!(
        "sk",
        "-svcacct-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56"
    );
    let out = redact(&format!("key={raw}"));
    assert!(
        out.contains("***REDACTED***"),
        "expected REDACTED marker, got: {out:?}"
    );
    assert!(!out.contains(raw), "raw sk-svcacct- key leaked: {out:?}");
}

#[test]
fn redaction_regex_no_catastrophic_backtracking_on_long_input() {
    // Pathological-shape input: long alphanumeric without any secret
    // pattern. If regex had exponential backtracking, this would time out.
    //
    // The returned value is asserted, not discarded. A timing bound on its own
    // is satisfied by `redact` never looking at the input at all — a length
    // short-circuit, an early return, a no-op stub — which is the failure this
    // test exists to rule out. Returning the input unchanged is proof the
    // patterns ran and matched nothing.
    let long_input = "a".repeat(10_000);
    let start = std::time::Instant::now();
    let out = redact(&long_input);
    let elapsed = start.elapsed();
    assert_eq!(
        out, long_input,
        "input carries no secret shape, so redact() must return it unchanged; a \
         different answer means the patterns did not run over it"
    );
    assert!(
        elapsed.as_millis() < 1000,
        "redact() on 10k-byte input took {}ms — suspected catastrophic backtracking",
        elapsed.as_millis()
    );
}

// ---- TASK-201 SEC-REDACTION-GCP ---------------------------------------
// New coverage: GCP service-account JSON blob, PEM private key shapes,
// `credentials` sensitive field name, and ReDoS guards on the two new
// multi-line patterns.
// -----------------------------------------------------------------------

#[test]
fn redacts_credential_field_and_pem_inside_stringified_json() {
    // A credential field is redacted while neighboring non-credential fields
    // remain useful diagnostic context.
    let raw = concat!(
        "{\"type\":\"service_account\",\"project_id\":\"my-project\",\"private_key_id\":\"abc123\",\"private_key\":\"--",
        "---BEGIN PRIVATE KEY-----\\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDexample\\n-----END PRIVATE KEY-----\\n\",\"client_email\":\"sa@my-project.iam.gserviceaccount.com\"}"
    );
    let wrapped = format!("credentials = {}", raw);
    let out = redact(&wrapped);
    assert!(
        out.contains(REDACTED),
        "expected REDACTED marker, got: {out:?}"
    );
    // The service-account object is treated as one credential-bearing blob.
    for secret in [
        "MIIEvQIBADANBgkqhkiG",
        "abc123",
        "my-project",
        "sa@my-project.iam.gserviceaccount.com",
    ] {
        assert!(
            !out.contains(secret),
            "service-account data leaked: {out:?}"
        );
    }
}

#[test]
fn redacts_pem_private_key_standalone() {
    // Bare PEM block embedded in a log line — no JSON wrapper.
    let raw = concat!(
        "--",
        "---BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG\n-----END PRIVATE KEY-----"
    );
    let out = redact(&format!("key_material = {raw}"));
    assert!(out.contains(REDACTED));
    assert!(
        !out.contains("MIIEvQIBADANBgkqhkiG"),
        "PEM body leaked: {out:?}"
    );
}

#[test]
fn redacts_pem_rsa_private_key() {
    // RSA-specific variant (`BEGIN RSA PRIVATE KEY`). The pattern also
    // covers `EC PRIVATE KEY` via the `(?:RSA |EC |)?` alternation.
    let raw = concat!(
        "--",
        "---BEGIN RSA PRIVATE KEY-----\nSECRETBYTES\n-----END RSA PRIVATE KEY-----"
    );
    let out = redact(raw);
    assert!(out.contains(REDACTED));
    assert!(!out.contains("SECRETBYTES"), "RSA PEM body leaked: {out:?}");
}

#[test]
fn redacts_exact_credential_field_name() {
    let out = redact_field_name("api_key");
    assert!(
        out.contains(REDACTED),
        "expected REDACTED marker on api_key, got: {out:?}"
    );
    assert!(
        !out.contains("api_key"),
        "credential field name should be redacted: {out:?}"
    );
}

#[test]
fn redaction_regex_gcp_no_catastrophic_backtracking_on_pathological_input() {
    // Pathological: open brace + long near-match of the GCP marker, with
    // no closing `}`. The `regex` crate uses linear-time matching so this
    // is a regression gate against a future maintainer swapping in a
    // backtracking engine or adding a `.*` alternation.
    let pathological = format!("{{\"type\":\"{}\"", "a".repeat(10_000));
    let start = std::time::Instant::now();
    let out = redact(&pathological);
    let elapsed = start.elapsed();
    // Asserted, not discarded: a bound on elapsed time alone is satisfied by a
    // `redact` that never examined the input.
    assert_eq!(
        out, pathological,
        "the GCP marker is incomplete, so redact() must return the input \
         unchanged; a different answer means the pattern did not run over it"
    );
    assert!(
        elapsed.as_millis() < 1000,
        "GCP pattern caused catastrophic backtracking: {}ms on 10k input",
        elapsed.as_millis()
    );
}

#[test]
fn redaction_regex_pem_no_catastrophic_backtracking_on_near_match() {
    // Pathological: `-----BEGIN PRIVATE KEY-----` header followed by a
    // long body but NO matching `-----END ... PRIVATE KEY-----` footer.
    let pathological = format!("-----BEGIN PRIVATE KEY-----{}", "A".repeat(10_000));
    let start = std::time::Instant::now();
    let out = redact(&pathological);
    let elapsed = start.elapsed();
    // Asserted, not discarded: a bound on elapsed time alone is satisfied by a
    // `redact` that never examined the input.
    assert_eq!(
        out, pathological,
        "the PEM footer is missing, so redact() must return the input unchanged; \
         a different answer means the pattern did not run over it"
    );
    assert!(
        elapsed.as_millis() < 1000,
        "PEM pattern caused catastrophic backtracking: {}ms on 10k input",
        elapsed.as_millis()
    );
}

#[test]
fn secret_value_redaction_keeps_words_and_replaces_credential_shapes() {
    // Shapes built at run time, so no literal credential sits in the source.
    let github = format!("ghp_{}", "K3y9".repeat(9));
    let aws = format!("AKIA{}", "Q".repeat(16));
    for path in ["src/token_store.rs", "config/secret_rules.md", "api_key.rs"] {
        assert_eq!(redact_secret_values(path), path);
        assert_eq!(
            redact_text(path),
            path,
            "free-text redaction changed {path}"
        );
    }
    let path = format!("keys/{github}/{aws}.txt");
    let redacted = redact_secret_values(&path);
    assert_eq!(redacted, format!("keys/{REDACTED}/{REDACTED}.txt"));
}

#[test]
fn free_text_redaction_matches_shapes_and_not_sensitive_words() {
    let anthropic = format!("sk-ant-{}", "a1".repeat(12));
    let text = format!("token={anthropic} password");
    assert_eq!(redact_text(&text), format!("token={REDACTED} password"));
    assert_eq!(
        redact_secret_values(&text),
        format!("token={REDACTED} password")
    );
}
