//! TASK-P0-B.6b PushNotification tool.
//!
//! Emits a structured notification via `tracing::event!(target =
//! "archon::notification", ...)`. Downstream subscribers (TUI
//! notification queue, observability layer) filter by this target. No
//! network IO, no filesystem writes, no platform-native desktop
//! notification dep (can be added in a follow-up without changing the
//! tool's public API).
//!
//! Input schema:
//! ```json
//! {
//!   "title": "string (required)",
//!   "body":  "string (optional)",
//!   "level": "'info' | 'warn' | 'error' (optional, default 'info')"
//! }
//! ```
//!
//! Permission level is `Safe` — the tool has no side effects beyond
//! structured logging.

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

/// Tracing target that observers subscribe to in order to pick up
/// notification events. Kept as a single `const` so TUI/obs code can
/// refer to the same value without drift.
pub const NOTIFICATION_TARGET: &str = "archon::notification";

pub struct PushNotificationTool;

#[async_trait::async_trait]
impl Tool for PushNotificationTool {
    fn name(&self) -> &str {
        "PushNotification"
    }

    fn description(&self) -> &str {
        "Emit a user-visible notification (e.g. 'long-running task \
         complete', 'permissions changed', 'reminder'). Implementation \
         is a structured tracing event on the `archon::notification` \
         target; TUI/observers subscribe to pick it up. No network IO, \
         no filesystem writes."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Short notification title (required)."
                },
                "body": {
                    "type": "string",
                    "description": "Optional longer notification body."
                },
                "level": {
                    "type": "string",
                    "enum": ["info", "warn", "error"],
                    "description": "Notification severity. Defaults to 'info'."
                }
            },
            "required": ["title"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        // ---- title (required, non-empty) ----
        let title = match input.get("title").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t,
            Some(_) => return ToolResult::error("title must be a non-empty string"),
            None => return ToolResult::error("title is required and must be a string"),
        };

        // ---- body (optional) ----
        let body = input
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // ---- level (optional, default "info") ----
        let level_str = input
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("info");
        let level = match NotificationLevel::parse(level_str) {
            Some(l) => l,
            None => {
                return ToolResult::error(format!(
                    "level must be one of 'info', 'warn', 'error' (got '{level_str}')"
                ));
            }
        };

        // Emit the structured tracing event. Each branch uses an
        // explicit `tracing::<level>!` macro with `target =
        // NOTIFICATION_TARGET`. Using the macros (vs `event!`) keeps
        // the call sites grep-able and lets `tracing-test` /
        // `tracing-subscriber` layers filter by `Level` cleanly.
        let canonical = level.as_str();
        match level {
            NotificationLevel::Info => {
                tracing::info!(
                    target: "archon::notification",
                    title = %title,
                    body = %body,
                    level = %canonical,
                    "notification"
                );
            }
            NotificationLevel::Warn => {
                tracing::warn!(
                    target: "archon::notification",
                    title = %title,
                    body = %body,
                    level = %canonical,
                    "notification"
                );
            }
            NotificationLevel::Error => {
                tracing::error!(
                    target: "archon::notification",
                    title = %title,
                    body = %body,
                    level = %canonical,
                    "notification"
                );
            }
        }

        ToolResult::success(format!("Notification emitted: [{canonical}] {title}"))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

/// Notification severity. Maps 1:1 to the `tracing::Level`s we emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotificationLevel {
    Info,
    Warn,
    Error,
}

impl NotificationLevel {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Record};
    use tracing::{Event, Id, Metadata, Subscriber};

    // ------------------------------------------------------------------
    // Minimal tracing::Subscriber that captures event target + fields.
    // Avoids pulling in `tracing-subscriber` / `tracing-test` as deps —
    // we only need to confirm events fire with the correct target and
    // payload, which the core `tracing` crate alone is enough for.
    // ------------------------------------------------------------------

    #[derive(Default, Clone)]
    struct CapturedEvent {
        target: String,
        fields: Vec<(String, String)>,
    }

    struct CaptureSubscriber {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    impl Subscriber for CaptureSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _span: &Id, _values: &Record<'_>) {}
        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
        fn event(&self, event: &Event<'_>) {
            let mut cap = CapturedEvent {
                target: event.metadata().target().to_string(),
                fields: Vec::new(),
            };
            struct Collector<'a>(&'a mut Vec<(String, String)>);
            impl<'a> Visit for Collector<'a> {
                fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                    self.0
                        .push((field.name().to_string(), format!("{value:?}")));
                }
                fn record_str(&mut self, field: &Field, value: &str) {
                    self.0.push((field.name().to_string(), value.to_string()));
                }
            }
            event.record(&mut Collector(&mut cap.fields));
            self.events.lock().unwrap().push(cap);
        }
        fn enter(&self, _span: &Id) {}
        fn exit(&self, _span: &Id) {}
    }

    fn capture<F, R>(f: F) -> (R, Vec<CapturedEvent>)
    where
        F: FnOnce() -> R,
    {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sub = CaptureSubscriber {
            events: events.clone(),
        };
        let result = tracing::subscriber::with_default(sub, f);
        let captured = events.lock().unwrap().clone();
        (result, captured)
    }

    fn ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            ..Default::default()
        }
    }

    fn field_value<'a>(evt: &'a CapturedEvent, name: &str) -> Option<&'a str> {
        evt.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Build a fresh current-thread runtime for a single test. We do
    /// this (rather than `#[tokio::test]`) because
    /// `tracing::subscriber::with_default` installs the subscriber on
    /// the CURRENT thread only — which needs to be the same thread
    /// that eventually runs the `tracing::*!` macros. A
    /// current-thread runtime guarantees that.
    fn run_with_capture<F>(build: F) -> (ToolResult, Vec<CapturedEvent>)
    where
        F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolResult>>>,
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        capture(|| rt.block_on(build()))
    }

    #[test]
    fn push_notification_emits_with_title_only() {
        let tool = PushNotificationTool;
        let (result, events) = run_with_capture(|| {
            Box::pin(async move {
                let c = ctx();
                PushNotificationTool
                    .execute(json!({ "title": "hello" }), &c)
                    .await
            })
        });
        let _ = tool; // silence unused warning; tool only used for type
        assert!(!result.is_error, "expected success: {}", result.content);
        assert!(
            result.content.contains("[info]"),
            "content: {}",
            result.content
        );
        assert!(result.content.contains("hello"));
        assert_eq!(events.len(), 1, "expected exactly one event");
        assert_eq!(events[0].target, NOTIFICATION_TARGET);
        assert_eq!(field_value(&events[0], "title"), Some("hello"));
        assert_eq!(field_value(&events[0], "level"), Some("info"));
    }

    #[test]
    fn push_notification_emits_with_body_and_level() {
        let (result, events) = run_with_capture(|| {
            Box::pin(async move {
                let c = ctx();
                PushNotificationTool
                    .execute(
                        json!({
                            "title": "task done",
                            "body": "all 3 tests passed",
                            "level": "warn"
                        }),
                        &c,
                    )
                    .await
            })
        });
        assert!(!result.is_error);
        assert!(result.content.contains("[warn]"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].target, NOTIFICATION_TARGET);
        assert_eq!(field_value(&events[0], "title"), Some("task done"));
        assert_eq!(field_value(&events[0], "body"), Some("all 3 tests passed"));
        assert_eq!(field_value(&events[0], "level"), Some("warn"));
    }

    #[test]
    fn push_notification_error_level_emits() {
        let (result, events) = run_with_capture(|| {
            Box::pin(async move {
                let c = ctx();
                PushNotificationTool
                    .execute(json!({ "title": "boom", "level": "error" }), &c)
                    .await
            })
        });
        assert!(!result.is_error);
        assert!(result.content.contains("[error]"));
        assert_eq!(events.len(), 1);
        assert_eq!(field_value(&events[0], "level"), Some("error"));
    }

    #[tokio::test]
    async fn push_notification_rejects_missing_title() {
        let tool = PushNotificationTool;
        let result = tool.execute(json!({}), &ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("title"));
    }

    #[tokio::test]
    async fn push_notification_rejects_empty_title() {
        let tool = PushNotificationTool;
        let result = tool.execute(json!({ "title": "" }), &ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("title"));
    }

    #[tokio::test]
    async fn push_notification_rejects_bad_level() {
        let tool = PushNotificationTool;
        let result = tool
            .execute(json!({ "title": "hi", "level": "debug" }), &ctx())
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("level"));
    }

    #[tokio::test]
    async fn push_notification_rejects_non_string_title() {
        let tool = PushNotificationTool;
        let result = tool.execute(json!({ "title": 42 }), &ctx()).await;
        assert!(result.is_error);
    }

    #[test]
    fn permission_level_is_safe() {
        let tool = PushNotificationTool;
        assert_eq!(
            tool.permission_level(&json!({ "title": "hi" })),
            PermissionLevel::Safe
        );
        // Also safe regardless of level — there are no side effects.
        assert_eq!(
            tool.permission_level(&json!({ "title": "oops", "level": "error" })),
            PermissionLevel::Safe
        );
    }

    #[test]
    fn input_schema_has_required_title() {
        let tool = PushNotificationTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "title"));
        // level enum must cover info / warn / error.
        let lvl_enum = schema["properties"]["level"]["enum"].as_array().unwrap();
        let values: Vec<&str> = lvl_enum.iter().filter_map(|v| v.as_str()).collect();
        assert!(values.contains(&"info"));
        assert!(values.contains(&"warn"));
        assert!(values.contains(&"error"));
    }

    #[test]
    fn notification_level_parse_round_trip() {
        assert_eq!(
            NotificationLevel::parse("info"),
            Some(NotificationLevel::Info)
        );
        assert_eq!(
            NotificationLevel::parse("warn"),
            Some(NotificationLevel::Warn)
        );
        assert_eq!(
            NotificationLevel::parse("error"),
            Some(NotificationLevel::Error)
        );
        assert_eq!(NotificationLevel::parse("trace"), None);
        assert_eq!(NotificationLevel::Info.as_str(), "info");
        assert_eq!(NotificationLevel::Warn.as_str(), "warn");
        assert_eq!(NotificationLevel::Error.as_str(), "error");
    }
}
