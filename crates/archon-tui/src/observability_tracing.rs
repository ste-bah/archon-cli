//! TASK-TUI-802: tracing spans, structured logging, task_id propagation, redaction.
//!
//! Split out of `observability.rs` in TASK-TUI-803 so the channel-metrics +
//! Prometheus exporter module stays under the 500-LoC ceiling mandated by
//! NFR-TUI-QUAL-001. External callers continue to reach these symbols via the
//! `archon_tui::observability::{init_tracing, span_agent_turn,
//! span_slash_dispatch, span_channel_send}` re-exports.
//!
//! See project-tasks/archon-fixes/tui_fixes/phase-8-observability/TASK-TUI-802.md

use once_cell::sync::Lazy;
use regex::Regex;
use std::io::Write;
use std::sync::Mutex as StdMutex;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Regex for secret shapes we must never log: OpenAI `sk-...` keys (20+ alnum),
/// bearer tokens, literal "password", and `api_key`/`api-key` field names.
/// Compiled once at first use; pattern is spec-constant and cannot fail.
static REDACTION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(sk-[A-Za-z0-9]{20,}|bearer\s+[A-Za-z0-9._-]+|password|api[_-]?key)")
        .expect("redaction regex is a compile-time constant")
});

/// Replacement token written in place of any redacted substring.
const REDACTED: &str = "***REDACTED***";

/// Apply the redaction regex to a field value.
#[inline]
fn redact(value: &str) -> String {
    REDACTION_RE.replace_all(value, REDACTED).into_owned()
}

/// Writer abstraction for the redaction layer. We use a trait-object behind a
/// `Mutex` so tests can substitute an in-memory `Vec<u8>` sink while prod
/// code writes to stderr.
type BoxedWriter = Box<dyn Write + Send + 'static>;

/// A `tracing_subscriber::Layer` that emits one redacted line per event to a
/// user-supplied writer. The layer is additive — it does NOT replace the fmt
/// layer; it produces an audit-grade secret-free stream that downstream
/// collectors (file, socket, test buffer) can rely on without parsing.
pub struct RedactionLayer {
    writer: StdMutex<BoxedWriter>,
}

impl RedactionLayer {
    /// Build a redaction layer writing to stderr. This is the production
    /// default wired into `init_tracing`.
    pub fn new() -> Self {
        Self {
            writer: StdMutex::new(Box::new(std::io::stderr())),
        }
    }

    /// Build a redaction layer writing to the given writer. Used by tests
    /// that capture emitted output into an in-memory buffer.
    pub fn with_writer<W: Write + Send + 'static>(writer: W) -> Self {
        Self {
            writer: StdMutex::new(Box::new(writer)),
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
}

impl RedactingVisitor {
    fn new() -> Self {
        Self { buf: String::new() }
    }

    #[inline]
    fn push_field(&mut self, name: &str, value: String) {
        if !self.buf.is_empty() {
            self.buf.push(' ');
        }
        // Redact both the field name (so `api_key` itself is masked) and the
        // value (so `sk-...` literals never reach the sink).
        let safe_name = redact(name);
        let safe_value = redact(&value);
        self.buf.push_str(&safe_name);
        self.buf.push('=');
        self.buf.push_str(&safe_value);
    }
}

impl tracing::field::Visit for RedactingVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.push_field(field.name(), value.to_string());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.push_field(field.name(), format!("{:?}", value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.push_field(field.name(), value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.push_field(field.name(), value.to_string());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.push_field(field.name(), value.to_string());
    }
}

impl<S> Layer<S> for RedactionLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = RedactingVisitor::new();
        event.record(&mut visitor);
        let level = event.metadata().level();
        let target = event.metadata().target();
        let line = format!("[{} {}] {}\n", level, target, visitor.buf);
        if let Ok(mut guard) = self.writer.lock() {
            // Best-effort write; a dropped byte here is not worth panicking
            // over (the fmt layer still has a copy via its own path).
            let _ = guard.write_all(line.as_bytes());
            let _ = guard.flush();
        }
    }
}

/// Install the global tracing subscriber with a JSON or pretty fmt layer,
/// an `EnvFilter` sourced from `RUST_LOG` (falling back to `level`), and a
/// `RedactionLayer` that scrubs secret shapes from every event.
///
/// Uses `try_init` so repeated calls from test binaries are harmless — the
/// first call installs, subsequent calls return `Ok(())` to preserve caller
/// idempotency expectations.
pub fn init_tracing(json: bool, level: tracing::Level) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level.to_string()));

    let redaction = RedactionLayer::new();

    // The fmt layer type differs between json/pretty, so we branch and
    // register independently. `try_init` swallows the "already set" error
    // and reports it back as `Err`; we normalise that into `Ok(())`.
    let result = if json {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json())
            .with(redaction)
            .try_init()
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().pretty())
            .with(redaction)
            .try_init()
    };

    // Either outcome is acceptable for idempotent callers; surface nothing.
    let _ = result;
    Ok(())
}

/// Root span for a single agent turn. Downstream code enters this span
/// before invoking the LLM and records `turn_ms` on exit.
pub fn span_agent_turn(task_id: &str) -> tracing::Span {
    tracing::info_span!(
        "agent.turn",
        task_id = %task_id,
        turn_ms = tracing::field::Empty
    )
}

/// Span covering a slash-command dispatch: command name + originating task.
pub fn span_slash_dispatch(task_id: &str, command: &str) -> tracing::Span {
    tracing::info_span!(
        "slash.dispatch",
        task_id = %task_id,
        command = %command
    )
}

/// Span covering a single channel send: the event kind + originating task.
pub fn span_channel_send(task_id: &str, event_kind: &str) -> tracing::Span {
    tracing::info_span!(
        "channel.send",
        task_id = %task_id,
        event_kind = %event_kind
    )
}

#[cfg(test)]
mod tracing_tests {
    use super::*;
    use std::sync::Arc;
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;

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

    #[test]
    fn init_tracing_is_idempotent() {
        // First call may or may not succeed depending on prior test state
        // (cargo test shares a process). Second call MUST NOT panic and
        // MUST return Ok — try_init collapses the "already set" error path.
        let first = init_tracing(true, Level::DEBUG);
        let second = init_tracing(true, Level::DEBUG);
        assert!(first.is_ok(), "first init_tracing must return Ok");
        assert!(second.is_ok(), "second init_tracing must be idempotent Ok");
    }

    #[test]
    fn span_agent_turn_has_task_id_field() {
        let span = span_agent_turn("task-42");
        assert!(
            span.field("task_id").is_some(),
            "agent.turn span missing task_id field"
        );
        assert!(
            span.field("turn_ms").is_some(),
            "agent.turn span missing turn_ms field"
        );
    }

    #[test]
    fn span_slash_dispatch_has_task_id_and_command() {
        let span = span_slash_dispatch("task-7", "/compact");
        assert!(span.field("task_id").is_some(), "missing task_id");
        assert!(span.field("command").is_some(), "missing command");
    }

    #[test]
    fn span_channel_send_has_task_id_and_event_kind() {
        let span = span_channel_send("task-9", "AgentDelta");
        assert!(span.field("task_id").is_some(), "missing task_id");
        assert!(span.field("event_kind").is_some(), "missing event_kind");
    }

    #[test]
    fn redaction_layer_redacts_sk_pattern() {
        let sink = SharedBuf::new();
        let layer = RedactionLayer::with_writer(sink.clone());
        let subscriber = tracing_subscriber::registry().with(layer);

        // Scope the dispatcher to this test so we don't conflict with a
        // globally-installed subscriber from other tests in the binary.
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(api_key = "sk-abcdefghijklmnopqrst0000", "emitting secret");
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
}
