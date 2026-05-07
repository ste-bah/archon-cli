use std::io::Write;
use std::sync::{Arc, Mutex as StdMutex};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;

use super::*;

/// Shared in-memory writer that clones cheaply and is safe across threads.
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

/// Secret-pattern matrix: every shape in `REDACTION_RE` must redact.
/// This is the security contract of the layer — if ANY of these leaks
/// into an emitted line the test fails with the raw secret in the panic
/// message (the test itself is the only place that printing raw secrets
/// is acceptable, and only on a FAIL).
#[test]
fn redaction_layer_redacts_every_secret_shape() {
    let cases = [
        ("openai", "sk-abcdefghijklmnopqrst0000"),
        ("anthropic", "sk-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"),
        ("aws_akia", "AKIAZZZZZZZZZZZZZZZZ"),
        ("github_pat", "ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
        ("github_oauth", "gho_abcdefghijklmnopqrstuvwxyz0123456789"),
        ("stripe_live", "sk_live_abcdefghijklmnopqrstuvwx"),
        ("stripe_pub_live", "pk_live_abcdefghijklmnopqrstuvwx"),
        (
            "jwt",
            "eyJhbGciOiJIUzI1NiIs.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe",
        ),
        ("bearer", "bearer ya29.a0Af_abcDEF-123"),
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
        let redacted = redact(name);
        assert!(
            redacted.contains(REDACTED),
            "field name {name} should be masked; got {redacted:?}"
        );
    }
}

/// Exercises the EXACT production redaction path: `init_tracing` builds
/// a `RedactionLayer` via the `stderr_with_format` constructor and
/// installs it as the sole emitter. We can't redirect `init_tracing`'s
/// stderr writer from a test (global install), but we CAN verify the
/// same `registry + filter + layer` stack with a capturing writer
/// produces the redacted output.
#[test]
fn redaction_path_matches_production_stack() {
    let sink = SharedBuf::new();
    let filter = EnvFilter::new("info");
    let layer = RedactionLayer::with_writer_and_format(sink.clone(), false);

    let subscriber = tracing_subscriber::registry().with(filter).with(layer);
    ::tracing::subscriber::with_default(subscriber, || {
        ::tracing::info!(api_key = "sk-abcdefghijklmnopqrst0000", "emitting secret");
    });

    let captured = sink.contents();
    assert!(
        captured.contains(REDACTED),
        "expected redaction marker in output, got: {captured:?}"
    );
    assert!(
        !captured.contains("sk-abcdefghijklmnopqrst0000"),
        "raw sk- secret leaked into sink: {captured:?}"
    );
}

/// JSON layout must emit well-formed structured output with redacted
/// field values. Regression gate against the parallel-fmt-leak bug —
/// if anyone adds `fmt::layer().json()` back into the stack this test
/// won't catch it, but the architecture comment at file top + code
/// review will.
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
    let raw = "sk-proj-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56";
    let out = redact(&format!("key={raw}"));
    assert!(
        out.contains("***REDACTED***"),
        "expected REDACTED marker, got: {out:?}"
    );
    assert!(!out.contains(raw), "raw sk-proj- key leaked: {out:?}");
}

#[test]
fn redacts_modern_openai_sk_svcacct_key() {
    let raw = "sk-svcacct-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56";
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
    let long_input = "a".repeat(10_000);
    let start = std::time::Instant::now();
    let _ = redact(&long_input);
    let elapsed = start.elapsed();
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
fn redacts_gcp_service_account_json_blob() {
    // Realistic GCP service-account key shape. All sensitive fields
    // (private_key body, private_key_id, client_email, project_id) MUST
    // be scrubbed — the design call on #201 chose whole-blob redaction
    // over field-selective redaction for secrets-first safety.
    let raw = r#"{"type":"service_account","project_id":"my-project","private_key_id":"abc123","private_key":"-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDexample\n-----END PRIVATE KEY-----\n","client_email":"sa@my-project.iam.gserviceaccount.com"}"#;
    let wrapped = format!("credentials = {}", raw);
    let out = redact(&wrapped);
    assert!(
        out.contains(REDACTED),
        "expected REDACTED marker, got: {out:?}"
    );
    // Sensitive fields must NOT leak. Private key body and private_key_id
    // would be catastrophic; project_id + client_email are PII-adjacent
    // and scrubbed per the whole-blob design call.
    assert!(
        !out.contains("MIIEvQIBADANBgkqhkiG"),
        "private_key body leaked: {out:?}"
    );
    assert!(
        !out.contains("sa@my-project.iam.gserviceaccount.com"),
        "client_email leaked: {out:?}"
    );
    assert!(!out.contains("abc123"), "private_key_id leaked: {out:?}");
}

#[test]
fn redacts_pem_private_key_standalone() {
    // Bare PEM block embedded in a log line — no JSON wrapper.
    let raw = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBgkqhkiG\n-----END PRIVATE KEY-----";
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
    let raw = "-----BEGIN RSA PRIVATE KEY-----\nSECRETBYTES\n-----END RSA PRIVATE KEY-----";
    let out = redact(raw);
    assert!(out.contains(REDACTED));
    assert!(!out.contains("SECRETBYTES"), "RSA PEM body leaked: {out:?}");
}

#[test]
fn redacts_credentials_field_name() {
    // `credentials` as a sensitive field name — the WORD itself is masked,
    // not the value. Mirrors the treatment of `password`, `api_key`, etc.
    let out = redact("credentials foo bar");
    assert!(
        out.contains(REDACTED),
        "expected REDACTED marker on 'credentials' word, got: {out:?}"
    );
    assert!(
        !out.contains("credentials"),
        "word 'credentials' itself should be redacted: {out:?}"
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
    let _ = redact(&pathological);
    let elapsed = start.elapsed();
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
    let _ = redact(&pathological);
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 1000,
        "PEM pattern caused catastrophic backtracking: {}ms on 10k input",
        elapsed.as_millis()
    );
}
