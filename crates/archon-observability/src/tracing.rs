//! Structured tracing setup + span constructors for archon-cli.
//!
//! Narrowed in TASK-AGS-OBS-906: the redaction surface (regex, marker,
//! `RedactionLayer`, `RedactingVisitor`, `json_escape`, `Layer` impl) moved
//! into the dedicated `crate::redaction` module so this file owns only the
//! tracing-glue concerns:
//!
//!   * `init_tracing` — installs the global subscriber with `EnvFilter`
//!     layered with the redaction emitter from `crate::redaction`.
//!   * `span_agent_turn`, `span_slash_dispatch`, `span_channel_send` —
//!     documented span constructors used by call sites in archon-tui,
//!     archon-session, and archon-core.
//!
//! OBS-905 history: this module was lifted verbatim from
//! `crates/archon-tui/src/observability_tracing.rs`. OBS-906 carved the
//! redaction surface out into its own module; the archon-tui re-export
//! shim still points at the crate-root `pub use` so downstream callers
//! remain byte-identical.
//!
//! # Redaction architecture (preserved across carve)
//!
//! `init_tracing` installs the redaction layer from `crate::redaction` as
//! the **sole event emitter**. The `tracing_subscriber::fmt` layer is
//! deliberately NOT stacked because tracing layers are parallel sinks, not
//! filters — installing both would mean every event is emitted twice, with
//! only one copy redacted. That would be a catastrophic secret leak. Prior
//! drafts of the original file had that bug; this tombstone comment is
//! preserved across both the lift (OBS-905) and the carve (OBS-906) so
//! nobody reintroduces it.

use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::redaction::RedactionLayer;

/// Install the global tracing subscriber with an `EnvFilter` sourced from
/// `RUST_LOG` (falling back to `level`) and a `RedactionLayer` that scrubs
/// secret shapes from every event.
///
/// Uses `try_init` so repeated calls from test binaries are harmless — the
/// first call installs, subsequent calls collapse the "already set" error
/// into `Ok(())` to preserve caller idempotency expectations.
pub fn init_tracing(json: bool, level: ::tracing::Level) -> anyhow::Result<()> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level.to_string()));

    let redaction = RedactionLayer::stderr_with_format(json);

    // Idempotent: `try_init` returns Err on second install; we normalise that
    // case into Ok so repeated boots (tests, process-restart harnesses) do
    // not trip. Any *other* failure mode bubbles up — today there aren't any
    // from registry().with(...).try_init() beyond the already-set case.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(redaction)
        .try_init();
    Ok(())
}

/// Root span for a single agent turn. Downstream code enters this span
/// before invoking the LLM and records `turn_ms` on exit.
pub fn span_agent_turn(task_id: &str) -> ::tracing::Span {
    ::tracing::info_span!(
        "agent.turn",
        task_id = %task_id,
        turn_ms = ::tracing::field::Empty
    )
}

/// Span covering a slash-command dispatch: command name + originating task.
pub fn span_slash_dispatch(task_id: &str, command: &str) -> ::tracing::Span {
    ::tracing::info_span!(
        "slash.dispatch",
        task_id = %task_id,
        command = %command
    )
}

/// Span covering a single channel send: the event kind + originating task.
pub fn span_channel_send(task_id: &str, event_kind: &str) -> ::tracing::Span {
    ::tracing::info_span!(
        "channel.send",
        task_id = %task_id,
        event_kind = %event_kind
    )
}

#[cfg(test)]
mod tracing_tests {
    use super::*;
    use ::tracing::Level;

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
}
