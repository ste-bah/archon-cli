//! A host-installed capability, scoped to one audit attempt. No path arguments or shell access.
use crate::tool::{
    PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult, WorkingTreeEffect,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;
/// A landing session exhausted its bounded final-answer corrections.
#[derive(Debug, Clone, thiserror::Error)]
#[error("incomplete landed artifact: {0}")]
pub struct IncompleteLandedArtifact(pub String);
/// The provider spent its output budget without delivering text or tools.
#[derive(Debug, Clone, thiserror::Error)]
#[error("output limit: provider returned an empty reply; finish_reason={0}")]
pub struct EmptyOutputLimit(pub String);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyReplyPolicy {
    Resend,
    RetainedCompletion,
}

pub trait LandingHost: Send + Sync {
    fn accepts_record_arrays(&self) -> bool;
    fn empty_reply_policy(&self) -> EmptyReplyPolicy;
    fn receipt_envelope_pointer(&self) -> &'static str;
    fn progress_key_fields(&self) -> &'static [&'static str];
    fn allows_unreceipted_reply(&self) -> bool;
    fn normalize_record_reply(&self) -> bool;
    fn tool_name(&self) -> &'static str;
    fn schema(&self) -> Option<serde_json::Value>;
    fn land(&self, record: Value) -> Result<String, String>;
    fn hint(&self) -> Result<String, String>;
    fn complete(&self, value: &Value) -> Result<(), String>;
    /// Use the host's result-envelope parser before checking a receipt.
    fn normalize_reply(&self, text: &str) -> Option<Value> {
        serde_json::from_str(text).ok()
    }
    /// Issue-550: the final reply the host can give from retained records
    /// alone when the session ends without one; `None` when it cannot.
    fn retained_completion(&self) -> Option<Value> {
        None
    }
}
struct Progress {
    last_landed: Instant,
    last_nudge: Option<Instant>,
    nudges: u8,
}
pub struct AuditLanding {
    host: Arc<dyn LandingHost>,
    path_timeout: Option<Duration>,
    progress: Mutex<Progress>,
    landed: Mutex<std::collections::BTreeMap<String, Value>>,
}
impl AuditLanding {
    pub fn new(host: Arc<dyn LandingHost>, seconds: Option<u64>) -> Self {
        Self {
            host,
            path_timeout: seconds.map(Duration::from_secs),
            progress: Mutex::new(Progress {
                last_landed: Instant::now(),
                last_nudge: None,
                nudges: 0,
            }),
            landed: Mutex::new(Default::default()),
        }
    }
    pub fn tool_name(&self) -> &'static str {
        self.host.tool_name()
    }
    pub fn accepts_record_arrays(&self) -> bool {
        self.host.accepts_record_arrays()
    }
    pub fn empty_reply_policy(&self) -> EmptyReplyPolicy {
        self.host.empty_reply_policy()
    }
    pub fn receipt_envelope_pointer(&self) -> &'static str {
        self.host.receipt_envelope_pointer()
    }
    pub fn progress_key_fields(&self) -> &'static [&'static str] {
        self.host.progress_key_fields()
    }
    pub fn allows_unreceipted_reply(&self) -> bool {
        self.host.allows_unreceipted_reply()
    }
    pub fn normalize_record_reply(&self) -> bool {
        self.host.normalize_record_reply()
    }
    pub fn schema(&self) -> Option<Value> {
        self.host.schema()
    }
    pub fn hint(&self) -> Result<String, String> {
        self.host.hint()
    }
    /// Consult only at a completed turn boundary, never as an inference timeout.
    pub fn progress_message(&self) -> Result<Option<String>, String> {
        let Some(interval) = self.path_timeout else {
            return Ok(None);
        };
        let mut progress = self.progress.lock().unwrap();
        let elapsed = progress.last_landed.elapsed();
        let since = progress
            .last_nudge
            .unwrap_or(progress.last_landed)
            .elapsed();
        if since < interval {
            return Ok(None);
        }
        if progress.nudges >= 2 {
            return Err(format!(
                "audit progress deadline ({} s since last new record key): two consecutive nudges produced no new landing keys",
                elapsed.as_secs()
            ));
        }
        progress.nudges += 1;
        progress.last_nudge = Some(Instant::now());
        Ok(Some(format!(
            "No new record key landed for {} s. Land any established records under keys not yet landed in this call, or send the final receipt when complete. Progress nudge {}/2.",
            elapsed.as_secs(),
            progress.nudges
        )))
    }
    pub fn complete(&self, value: &Value) -> Result<(), String> {
        self.host.complete(value)
    }
    pub fn normalize_reply(&self, text: &str) -> Option<Value> {
        self.host.normalize_reply(text)
    }
    pub fn retained_completion(&self) -> Option<Value> {
        self.host.retained_completion()
    }
    /// Land one record, whether it came through the tool or as a final reply.
    pub fn land(&self, input: Value) -> Result<String, String> {
        self.land_with_progress(input).map(|(hint, _)| hint)
    }
    pub fn land_with_progress(&self, input: Value) -> Result<(String, bool), String> {
        let path = self
            .host
            .progress_key_fields()
            .iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        let hint = self.host.land(input.clone())?;
        let changed = self.landed.lock().unwrap().insert(path, input).is_none();
        if changed {
            *self.progress.lock().unwrap() = Progress {
                last_landed: Instant::now(),
                last_nudge: None,
                nudges: 0,
            };
        }
        Ok((hint, changed))
    }
}
tokio::task_local! { static CURRENT: Arc<AuditLanding>; }
pub async fn scope<T>(landing: Arc<AuditLanding>, work: impl std::future::Future<Output = T>) -> T {
    CURRENT.scope(landing, work).await
}
pub fn current() -> Option<Arc<AuditLanding>> {
    CURRENT.try_with(Clone::clone).ok()
}
pub struct LandAuditRecordTool;
#[async_trait::async_trait]
impl Tool for LandAuditRecordTool {
    fn name(&self) -> &str {
        "land-audit-record"
    }
    fn description(&self) -> &str {
        "Persist one established AuditRecord to the host's snapshot-bound audit store. No repository mutation. Returns the remaining paths. Land each record as soon as established."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","required":["declared_path","verdict","equivalents","required_action","reason"],"additionalProperties":false,"properties":{
        "declared_path":{"type":"string"},"verdict":{"type":"string","enum":["exists_as_declared","absent","exists_elsewhere","unreachable"]},
        "equivalents":{"type":"array","items":{"type":"string"}},"required_action":{"type":"string","enum":["none","deliver","wire_or_migrate"]},"reason":{"type":"string"}}})
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let Some(landing) = &ctx.audit_landing else {
            return ToolResult::refusal("land-audit-record requires host audit authority");
        };
        match landing.land(input) {
            Ok(hint) => ToolResult::success(hint),
            Err(error) => ToolResult::error(error),
        }
    }
    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }
    fn permission_level(&self, _: &Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::ExternalOnly
    }
}

/// Definition and execution share the same capability, so prompts cannot advertise a different tool.
pub struct ScopedLandingTool(pub Arc<AuditLanding>);
#[async_trait::async_trait]
impl Tool for ScopedLandingTool {
    fn name(&self) -> &str {
        self.0.tool_name()
    }
    fn description(&self) -> &str {
        "Land one typed evidence record in this call's host-owned store. Does not write repository files. Returns remaining subjects; final submission references records_landed."
    }
    fn input_schema(&self) -> Value {
        self.0
            .schema()
            .unwrap_or_else(|| LandAuditRecordTool.input_schema())
    }
    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        if !ctx
            .audit_landing
            .as_ref()
            .is_some_and(|cap| Arc::ptr_eq(cap, &self.0))
        {
            return ToolResult::refusal("record landing capability does not belong to this call");
        }
        LandAuditRecordTool.execute(input, ctx).await
    }
    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }
    fn permission_level(&self, _: &Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::ExternalOnly
    }
}

#[cfg(test)]
mod progress_tests {
    use super::*;
    struct Host;
    impl LandingHost for Host {
        fn accepts_record_arrays(&self) -> bool {
            false
        }
        fn empty_reply_policy(&self) -> crate::audit_landing::EmptyReplyPolicy {
            crate::audit_landing::EmptyReplyPolicy::Resend
        }
        fn receipt_envelope_pointer(&self) -> &'static str {
            "/data/repository_audit"
        }
        fn progress_key_fields(&self) -> &'static [&'static str] {
            &["declared_path"]
        }
        fn allows_unreceipted_reply(&self) -> bool {
            true
        }
        fn normalize_record_reply(&self) -> bool {
            true
        }
        fn tool_name(&self) -> &'static str {
            "land-audit-record"
        }
        fn schema(&self) -> Option<serde_json::Value> {
            None
        }
        fn land(&self, _: Value) -> Result<String, String> {
            Ok("landed".into())
        }
        fn hint(&self) -> Result<String, String> {
            Ok("remaining".into())
        }
        fn complete(&self, _: &Value) -> Result<(), String> {
            Ok(())
        }
    }
    #[tokio::test]
    async fn landing_without_host_authority_is_a_guard_refusal() {
        let result = LandAuditRecordTool
            .execute(
                json!({"declared_path":"src/main.rs"}),
                &ToolContext::default(),
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("requires host audit authority"));
        assert!(result.is_guard_refusal());
    }

    #[tokio::test]
    async fn landing_with_another_calls_capability_is_a_guard_refusal() {
        let owned = Arc::new(AuditLanding::new(Arc::new(Host), Some(900)));
        let foreign = Arc::new(AuditLanding::new(Arc::new(Host), Some(900)));
        let tool = ScopedLandingTool(owned);
        let result = tool
            .execute(
                json!({"declared_path":"src/main.rs"}),
                &ToolContext {
                    audit_landing: Some(foreign),
                    ..Default::default()
                },
            )
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("does not belong to this call"));
        assert!(result.is_guard_refusal());
    }

    #[tokio::test(start_paused = true)]
    async fn audit_landing_progress_uses_host_keys_for_distinct_records() {
        struct UnitHost;
        impl LandingHost for UnitHost {
            fn accepts_record_arrays(&self) -> bool {
                false
            }
            fn empty_reply_policy(&self) -> crate::audit_landing::EmptyReplyPolicy {
                crate::audit_landing::EmptyReplyPolicy::Resend
            }
            fn receipt_envelope_pointer(&self) -> &'static str {
                "/data/repository_audit"
            }
            fn allows_unreceipted_reply(&self) -> bool {
                true
            }
            fn normalize_record_reply(&self) -> bool {
                true
            }
            fn tool_name(&self) -> &'static str {
                "land-audit-record"
            }
            fn schema(&self) -> Option<serde_json::Value> {
                None
            }
            fn progress_key_fields(&self) -> &'static [&'static str] {
                &["unit"]
            }
            fn land(&self, _: Value) -> Result<String, String> {
                Ok("landed".into())
            }
            fn hint(&self) -> Result<String, String> {
                Ok(String::new())
            }
            fn complete(&self, _: &Value) -> Result<(), String> {
                Ok(())
            }
        }
        let landing = AuditLanding::new(Arc::new(UnitHost), Some(10));
        landing.land(json!({"unit":"first"})).unwrap();
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("1/2"));
        landing.land(json!({"unit":"second"})).unwrap();
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("1/2"));
        // Rewording a landed key cannot renew the deadline.
        assert!(
            !landing
                .land_with_progress(json!({"unit":"second","summary":"reworded"}))
                .unwrap()
                .1
        );
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("2/2"));
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(landing.progress_message().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn progress_nudges_twice_before_failure_and_new_landing_resets() {
        let landing = Arc::new(AuditLanding::new(Arc::new(Host), Some(900)));
        tokio::time::advance(Duration::from_secs(720)).await;
        assert!(landing.progress_message().unwrap().is_none());
        tokio::time::advance(Duration::from_secs(180)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("1/2"));
        assert!(landing.progress_message().unwrap().is_none());
        tokio::time::advance(Duration::from_secs(900)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("2/2"));
        let ctx = ToolContext {
            audit_landing: Some(landing.clone()),
            ..Default::default()
        };
        assert!(
            !LandAuditRecordTool
                .execute(json!({"declared_path":"new"}), &ctx)
                .await
                .is_error
        );
        tokio::time::advance(Duration::from_secs(900)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("1/2"));
        tokio::time::advance(Duration::from_secs(900)).await;
        assert!(landing.progress_message().unwrap().unwrap().contains("2/2"));
        tokio::time::advance(Duration::from_secs(900)).await;
        let error = landing.progress_message().unwrap_err();
        assert!(error.contains("audit progress deadline (2700 s"));
        assert!(!error.contains("wall-clock"));
    }
}
