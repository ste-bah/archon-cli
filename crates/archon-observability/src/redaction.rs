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
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// Regex for secret shapes we must never log. Alternation covers:
///   * OpenAI `sk-...` (20+ alnum)
///   * Anthropic `sk-ant-...` (20+ alnum/underscore/dash)
///   * AWS access key id `AKIA[0-9A-Z]{16}`
///   * GitHub PAT/OAuth/user/server/refresh `gh[pousr]_[A-Za-z0-9]{36}`
///   * Stripe live/test secret+publishable `sk_live_`, `sk_test_`, `pk_live_`, `pk_test_`
///   * JWT `eyJ...<header>.<payload>.<sig>`
///   * `bearer <token>` authorization values
///   * GCP service-account JSON blob (TASK-201) — whole `{...}` containing the
///     `"type":"service_account"` marker. Non-greedy to the next `}`, so
///     standard-shaped GCP SA keys (flat object, no nested braces) are fully
///     scrubbed. Over-redaction on nested JSON is an acceptable secrets-first
///     trade-off.
///   * PEM private key block (TASK-201) — `-----BEGIN [RSA |EC ]PRIVATE KEY-----
///     ... -----END ...-----`, standalone or embedded in a larger value.
///   * Sensitive field names (`password`, `api_key`, `api-key`, `authorization`,
///     `credentials` [TASK-201], `secret`, `token`) — masked so the identifier
///     itself never leaks into log lines even when the value happens to parse
///     clean.
///
/// Compiled once at first use; pattern is spec-constant and cannot fail. The
/// `regex` crate guarantees linear-time matching (no catastrophic
/// backtracking), so pathological inputs cannot DoS this layer — the
/// `*_no_catastrophic_backtracking_*` tests below are regression gates
/// against that property.
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
          # TASK-201 GCP service-account JSON: redact the WHOLE blob containing
          # the `"type":"service_account"` marker. Non-greedy to the next `}`.
          # Standard GCP SA shape is a flat object (private_key contains `\n`
          # but no braces), so `[^{}]` before the marker + `[\s\S]*?` after
          # catches the full object. Over-redaction on atypical nested shapes
          # is the acceptable secrets-first posture.
          | \{[^{}]*"type"\s*:\s*"service_account"[\s\S]*?\}
          # TASK-201 PEM private key block (standalone or embedded). `(?:RSA |EC |)?`
          # covers `BEGIN PRIVATE KEY`, `BEGIN RSA PRIVATE KEY`, `BEGIN EC PRIVATE KEY`.
          # Literal spaces are needed (pattern uses `\s` because (?x) ignores
          # inline whitespace).
          | -----BEGIN\s(?:RSA\s|EC\s|)?PRIVATE\sKEY-----[\s\S]*?-----END\s(?:RSA\s|EC\s|)?PRIVATE\sKEY-----
          | password
          | api[_\-]?key
          | authorization
          | credentials                               # TASK-201 GCP field name
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

/// Apply the same redaction to arbitrary text destined for a human.
///
/// The layer above redacts tracing fields on their way to a log. This is the
/// same regex for output that never becomes an event — `archon config dump`
/// above all, whose whole purpose is to be pasted into an issue (#189 Phase 7).
/// Sharing the pattern is the point: a second, weaker one would be a hole that
/// looked like a feature.
#[inline]
pub fn redact_text(value: &str) -> String {
    redact(value)
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
    /// OTLP destination, when one is configured (#189 Phase 10).
    ///
    /// Held *here* rather than installed as its own layer, and that placement
    /// is the security property: a second layer would be a parallel sink
    /// receiving the raw event, so the only copy that ever reaches the wire is
    /// the one this layer has already redacted.
    otlp: Option<std::sync::Arc<crate::otlp::OtlpExport>>,
}

impl RedactionLayer {
    /// Build a redaction layer writing to stderr (pretty layout). Production
    /// default for `init_tracing(false, _)`.
    pub fn new() -> Self {
        Self {
            writer: StdMutex::new(Box::new(std::io::stderr())),
            json: false,
            otlp: None,
        }
    }

    /// Build a redaction layer writing to the given writer (pretty layout).
    /// Used by tests that capture emitted output into an in-memory buffer.
    pub fn with_writer<W: Write + Send + 'static>(writer: W) -> Self {
        Self {
            writer: StdMutex::new(Box::new(writer)),
            json: false,
            otlp: None,
        }
    }

    /// Build a redaction layer writing to the given writer with the specified
    /// layout. Exposed `pub(crate)` so `init_tracing` (in the sibling `tracing`
    /// module) can hand tests the same constructor that production uses.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_writer_and_format<W: Write + Send + 'static>(writer: W, json: bool) -> Self {
        Self {
            writer: StdMutex::new(Box::new(writer)),
            json,
            otlp: None,
        }
    }

    /// `pub(crate)` constructor used by `crate::tracing::init_tracing` to
    /// build the production layer wired to stderr with the caller's JSON
    /// preference. Keeps the raw `{ writer, json }` fields private so no one
    /// can bypass the constructor invariants.
    /// Attach an OTLP destination to this layer (#189 Phase 10).
    ///
    /// Consuming `self` rather than taking `&mut`: there must be no moment at
    /// which a layer is installed and its export destination is still being
    /// decided.
    #[must_use]
    pub fn with_otlp(mut self, otlp: std::sync::Arc<crate::otlp::OtlpExport>) -> Self {
        self.otlp = Some(otlp);
        self
    }

    pub(crate) fn stderr_with_format(json: bool) -> Self {
        Self {
            writer: StdMutex::new(Box::new(std::io::stderr())),
            json,
            otlp: None,
        }
    }
}

impl Default for RedactionLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// A span's redacted attributes, waiting for it to close (#189 Phase 10).
///
/// Stored in the registry's per-span extensions rather than in a map on the
/// layer, so it lives and dies with the span it belongs to — a span that is
/// never closed cannot leak an entry, and nothing has to be reaped.
struct PendingExport {
    attributes: Vec<(String, String)>,
    start: std::time::SystemTime,
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
    /// Record a span's attributes, redacted, for export on close.
    ///
    /// Redaction happens here rather than at export time so there is no window
    /// in which an unscrubbed value is sitting in the registry's extensions —
    /// and so the same [`RedactingVisitor`] that guards the log line guards the
    /// exported attributes. One implementation, one contract.
    fn on_new_span(
        &self,
        attrs: &::tracing::span::Attributes<'_>,
        id: &::tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        if self.otlp.is_none() {
            // Configured off: no allocation, no extension, no cost.
            return;
        }
        let Some(span) = ctx.span(id) else {
            return;
        };
        // `json: true` so the visitor keeps name/value pairs apart, which is
        // the shape OTLP attributes need. The redaction applied is identical.
        let mut visitor = RedactingVisitor::new(true);
        attrs.record(&mut visitor);
        span.extensions_mut().insert(PendingExport {
            attributes: visitor.json_fields,
            start: std::time::SystemTime::now(),
        });
    }

    /// Fields recorded after a span opens are redacted the same way.
    fn on_record(
        &self,
        id: &::tracing::span::Id,
        values: &::tracing::span::Record<'_>,
        ctx: Context<'_, S>,
    ) {
        if self.otlp.is_none() {
            return;
        }
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut visitor = RedactingVisitor::new(true);
        values.record(&mut visitor);
        let mut extensions = span.extensions_mut();
        if let Some(pending) = extensions.get_mut::<PendingExport>() {
            pending.attributes.extend(visitor.json_fields);
        }
    }

    fn on_close(&self, id: ::tracing::span::Id, ctx: Context<'_, S>) {
        let Some(otlp) = self.otlp.as_ref() else {
            return;
        };
        let Some(span) = ctx.span(&id) else {
            return;
        };
        let name = span.name().to_string();
        let Some(pending) = span.extensions_mut().remove::<PendingExport>() else {
            return;
        };
        otlp.export(crate::otlp::RedactedSpan {
            name,
            attributes: pending.attributes,
            start: pending.start,
            end: std::time::SystemTime::now(),
        });
    }

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
                fields_json.push_str(&format!("\"{}\":\"{}\"", json_escape(k), json_escape(v)));
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
            format!("[{} {}]{} {}\n", level, target, span_suffix, visitor.buf)
        };

        if let Ok(mut guard) = self.writer.lock() {
            let _ = guard.write_all(line.as_bytes());
            let _ = guard.flush();
        }
    }
}

#[cfg(test)]
mod redaction_tests;
