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

use std::io::Write;
use std::sync::Mutex as StdMutex;
use tracing::Subscriber;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

/// Replacement token written in place of any redacted substring.
pub(crate) const REDACTED: &str = "***REDACTED***";

/// Apply the redaction regex to a field value.
#[inline]
pub(crate) fn redact(value: &str) -> String {
    crate::shared_redactor::redact_text(value, REDACTED)
}

/// Mask sensitive identifiers when they are used as structured field names.
#[inline]
fn redact_field_name(value: &str) -> String {
    if crate::shared_redactor::is_credential_key(value) {
        REDACTED.to_string()
    } else {
        crate::secret_values::redact_registered(value)
    }
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
    crate::shared_redactor::redact_text(value, REDACTED)
}

/// Redact secret VALUES only: registered secret values and the credential
/// shapes of [`redact_text`], but not its field-name words.
///
/// For text that must stay exact to be true, above all a file path: under
/// [`redact_text`], `src/token_store.rs` loses the word `token` and stops
/// naming the file that was read. A credential-shaped value inside the text
/// is still replaced.
#[inline]
pub fn redact_secret_values(value: &str) -> String {
    crate::shared_redactor::redact_text(value, REDACTED)
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
        let credential_value = crate::shared_redactor::is_credential_key(name);
        let safe_name = redact_field_name(name);
        let safe_value = if credential_value {
            REDACTED.to_string()
        } else {
            redact(&value)
        };
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
        let name = redact(span.name());
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
        let target = redact(meta.target());

        // Collect span path for context. Walk from root so the outermost
        // span is first; useful for correlation across task_id scopes.
        let mut spans: Vec<String> = Vec::new();
        if let Some(span) = ctx.lookup_current() {
            for s in span.scope().from_root() {
                spans.push(redact(s.name()));
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
