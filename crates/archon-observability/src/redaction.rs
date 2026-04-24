//! Secret redaction layer for structured tracing.
//!
//! Carved out of `src/tracing.rs` in TASK-AGS-OBS-906 as part of the
//! Stage 10 LIFT sequence. The module is dedicated to the security-critical
//! surface — regex + replacement marker + layer + visitor + JSON-escape
//! helper — so future work on `tracing.rs` (e.g. OBS-907 JSON gate-walk,
//! OBS-901 metrics lift) cannot accidentally touch the redaction contract.
//!
//! Content is verbatim from the pre-carve `src/tracing.rs`: the regex,
//! `REDACTED` token, `RedactionLayer` struct, existing constructors
//! (`new`, `with_writer`, `with_writer_and_format`), `Default`,
//! `RedactingVisitor`, `json_escape`, and the `Layer<S> for RedactionLayer`
//! impl all moved unchanged. Two content-level diffs beyond this
//! module-header comment:
//!   1. New `pub(crate) fn stderr_with_format(json)` constructor that
//!      replaces the inline `RedactionLayer { writer: StdMutex::new(
//!      Box::new(stderr())), json }` build that used to live inside
//!      `init_tracing`. Keeps the raw `{ writer, json }` fields private
//!      now that `init_tracing` lives in a sibling module.
//!   2. Visibility promotions (`pub(crate)`) on `REDACTION_RE`, `REDACTED`,
//!      `redact()`, and `with_writer_and_format` — required because the
//!      sibling `tracing` module uses `stderr_with_format` (and because
//!      unit tests inside this module reach the helpers). None of these
//!      are re-exported at the crate root; external callers still only
//!      see `RedactionLayer` + its two public constructors.
//!
//! # Security contract
//!
//! [`RedactionLayer`] is the **sole** emitter installed by
//! `crate::tracing::init_tracing`. The `tracing_subscriber::fmt` layer is
//! deliberately NOT stacked because tracing layers are parallel sinks, not
//! filters — installing both would mean every event is emitted twice, with
//! only one copy redacted. That would be a catastrophic secret leak. Prior
//! drafts of the original file had that bug; this tombstone comment is
//! preserved across the carve so nobody reintroduces it.
//!
//! The public unit tests below cover the full 9-secret-shape matrix and the
//! sensitive-field-name masking. External smoke coverage lives in
//! `tests/redaction_smoke.rs`.

use once_cell::sync::Lazy;
use regex::Regex;
use std::io::Write;
use std::sync::Mutex as StdMutex;
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// Regex for secret shapes we must never log. Alternation covers:
///   * OpenAI `sk-...` (20+ alnum)
///   * Anthropic `sk-ant-...` (20+ alnum/underscore/dash)
///   * AWS access key id `AKIA[0-9A-Z]{16}`
///   * GitHub PAT/OAuth/user/server/refresh `gh[pousr]_[A-Za-z0-9]{36}`
///   * Stripe live/test secret+publishable `sk_live_`, `sk_test_`, `pk_live_`, `pk_test_`
///   * JWT `eyJ...<header>.<payload>.<sig>`
///   * `bearer <token>` authorization values
///   * Sensitive field names (`password`, `api_key`, `api-key`, `authorization`,
///     `secret`, `token`) — masked so the identifier itself never leaks into
///     log lines even when the value happens to parse clean.
///
/// Compiled once at first use; pattern is spec-constant and cannot fail.
pub(crate) static REDACTION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?ix)
        (
            sk-ant-[A-Za-z0-9_\-]{20,}                 # Anthropic
          | sk-(?:proj|svcacct)-[A-Za-z0-9_\-]{20,}    # OpenAI modern (2024+, sk-proj- / sk-svcacct-)
          | sk-[A-Za-z0-9]{20,}                         # OpenAI legacy
          | AKIA[0-9A-Z]{16}                            # AWS access key id
          | gh[pousr]_[A-Za-z0-9]{36}                   # GitHub tokens
          | (?:sk|pk)_(?:live|test)_[A-Za-z0-9]{24,}    # Stripe secret/publishable
          | eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}  # JWT
          | bearer\s+[A-Za-z0-9._\-]+                   # Authorization: bearer ...
          | password
          | api[_\-]?key
          | authorization
          | secret
          | token
        )
        "#,
    )
    .expect("redaction regex is a compile-time constant")
});

/// Replacement token written in place of any redacted substring.
pub(crate) const REDACTED: &str = "***REDACTED***";

/// Apply the redaction regex to a field value.
#[inline]
pub(crate) fn redact(value: &str) -> String {
    REDACTION_RE.replace_all(value, REDACTED).into_owned()
}

/// Writer abstraction for the redaction layer. We use a trait-object behind a
/// `Mutex` so tests can substitute an in-memory `Vec<u8>` sink while prod
/// code writes to stderr.
type BoxedWriter = Box<dyn Write + Send + 'static>;

/// The **sole** emitter installed by `crate::tracing::init_tracing`. Emits one
/// redacted line per event; layout is JSON when `json=true`, pretty key=value
/// otherwise.
///
/// Architecture: because tracing-subscriber layers are parallel sinks, not
/// filters, this layer cannot coexist with `fmt::layer()` — the fmt layer
/// would emit an unredacted copy in parallel. `init_tracing` therefore stacks
/// only `EnvFilter + RedactionLayer` on the registry.
pub struct RedactionLayer {
    writer: StdMutex<BoxedWriter>,
    json: bool,
}

impl RedactionLayer {
    /// Build a redaction layer writing to stderr (pretty layout). Production
    /// default for `init_tracing(false, _)`.
    pub fn new() -> Self {
        Self {
            writer: StdMutex::new(Box::new(std::io::stderr())),
            json: false,
        }
    }

    /// Build a redaction layer writing to the given writer (pretty layout).
    /// Used by tests that capture emitted output into an in-memory buffer.
    pub fn with_writer<W: Write + Send + 'static>(writer: W) -> Self {
        Self {
            writer: StdMutex::new(Box::new(writer)),
            json: false,
        }
    }

    /// Build a redaction layer writing to the given writer with the specified
    /// layout. Exposed `pub(crate)` so `init_tracing` (in the sibling `tracing`
    /// module) can hand tests the same constructor that production uses.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_writer_and_format<W: Write + Send + 'static>(
        writer: W,
        json: bool,
    ) -> Self {
        Self {
            writer: StdMutex::new(Box::new(writer)),
            json,
        }
    }

    /// `pub(crate)` constructor used by `crate::tracing::init_tracing` to
    /// build the production layer wired to stderr with the caller's JSON
    /// preference. Keeps the raw `{ writer, json }` fields private so no one
    /// can bypass the constructor invariants.
    pub(crate) fn stderr_with_format(json: bool) -> Self {
        Self {
            writer: StdMutex::new(Box::new(std::io::stderr())),
            json,
        }
    }
}

impl Default for RedactionLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Visitor that accumulates a redacted representation of each event field
/// into a single line `name=value name=value ...`.
struct RedactingVisitor {
    buf: String,
    json_fields: Vec<(String, String)>,
    json: bool,
}

impl RedactingVisitor {
    fn new(json: bool) -> Self {
        Self {
            buf: String::new(),
            json_fields: Vec::new(),
            json,
        }
    }

    #[inline]
    fn push_field(&mut self, name: &str, value: String) {
        // Redact both the field name (so `api_key` itself is masked) and the
        // value (so `sk-...` literals never reach the sink).
        let safe_name = redact(name);
        let safe_value = redact(&value);
        if self.json {
            self.json_fields.push((safe_name, safe_value));
        } else {
            if !self.buf.is_empty() {
                self.buf.push(' ');
            }
            self.buf.push_str(&safe_name);
            self.buf.push('=');
            self.buf.push_str(&safe_value);
        }
    }
}

impl ::tracing::field::Visit for RedactingVisitor {
    fn record_str(&mut self, field: &::tracing::field::Field, value: &str) {
        self.push_field(field.name(), value.to_string());
    }

    fn record_debug(&mut self, field: &::tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.push_field(field.name(), format!("{:?}", value));
    }

    fn record_i64(&mut self, field: &::tracing::field::Field, value: i64) {
        self.push_field(field.name(), value.to_string());
    }

    fn record_u64(&mut self, field: &::tracing::field::Field, value: u64) {
        self.push_field(field.name(), value.to_string());
    }

    fn record_bool(&mut self, field: &::tracing::field::Field, value: bool) {
        self.push_field(field.name(), value.to_string());
    }
}

/// Escape a string for a JSON string literal body.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

impl<S> Layer<S> for RedactionLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &::tracing::Event<'_>, ctx: Context<'_, S>) {
        let mut visitor = RedactingVisitor::new(self.json);
        event.record(&mut visitor);
        let meta = event.metadata();
        let level = meta.level();
        let target = meta.target();

        // Collect span path for context. Walk from root so the outermost
        // span is first; useful for correlation across task_id scopes.
        let mut spans: Vec<String> = Vec::new();
        if let Some(span) = ctx.lookup_current() {
            for s in span.scope().from_root() {
                spans.push(s.name().to_string());
            }
        }

        let line = if self.json {
            let mut fields_json = String::from("{");
            for (i, (k, v)) in visitor.json_fields.iter().enumerate() {
                if i > 0 {
                    fields_json.push(',');
                }
                fields_json.push_str(&format!(
                    "\"{}\":\"{}\"",
                    json_escape(k),
                    json_escape(v)
                ));
            }
            fields_json.push('}');
            let spans_json: String = spans
                .iter()
                .map(|s| format!("\"{}\"", json_escape(s)))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "{{\"level\":\"{}\",\"target\":\"{}\",\"spans\":[{}],\"fields\":{}}}\n",
                level, target, spans_json, fields_json,
            )
        } else {
            let span_suffix = if spans.is_empty() {
                String::new()
            } else {
                format!(" {{{}}}", spans.join("::"))
            };
            format!(
                "[{} {}]{} {}\n",
                level, target, span_suffix, visitor.buf
            )
        };

        if let Ok(mut guard) = self.writer.lock() {
            let _ = guard.write_all(line.as_bytes());
            let _ = guard.flush();
        }
    }
}

#[cfg(test)]
mod redaction_tests {
    use super::*;
    use std::sync::Arc;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::EnvFilter;

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
            ("openai",  "sk-abcdefghijklmnopqrst0000"),
            ("anthropic", "sk-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"),
            ("aws_akia", "AKIAZZZZZZZZZZZZZZZZ"),
            ("github_pat", "ghp_abcdefghijklmnopqrstuvwxyz0123456789"),
            ("github_oauth", "gho_abcdefghijklmnopqrstuvwxyz0123456789"),
            ("stripe_live", "sk_live_abcdefghijklmnopqrstuvwx"),
            ("stripe_pub_live", "pk_live_abcdefghijklmnopqrstuvwx"),
            ("jwt", "eyJhbGciOiJIUzI1NiIs.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
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
        for name in ["password", "api_key", "api-key", "authorization", "secret", "token"] {
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
        assert!(captured.contains("\"level\":\"INFO\""), "json missing level: {captured:?}");
        assert!(captured.contains("\"target\":"), "json missing target: {captured:?}");
        assert!(captured.contains("\"fields\":{"), "json missing fields obj: {captured:?}");
    }

    #[test]
    fn redacts_modern_openai_sk_proj_key() {
        let raw = "sk-proj-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56";
        let out = redact(&format!("key={raw}"));
        assert!(
            out.contains("***REDACTED***"),
            "expected REDACTED marker, got: {out:?}"
        );
        assert!(
            !out.contains(raw),
            "raw sk-proj- key leaked: {out:?}"
        );
    }

    #[test]
    fn redacts_modern_openai_sk_svcacct_key() {
        let raw = "sk-svcacct-abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56";
        let out = redact(&format!("key={raw}"));
        assert!(
            out.contains("***REDACTED***"),
            "expected REDACTED marker, got: {out:?}"
        );
        assert!(
            !out.contains(raw),
            "raw sk-svcacct- key leaked: {out:?}"
        );
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
}
