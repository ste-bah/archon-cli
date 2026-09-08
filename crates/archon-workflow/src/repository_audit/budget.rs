use serde::{Deserialize, Serialize};
use crate::WorkflowResult;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Limit { Finite(u64), Unlimited }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditPolicy { pub attempt_timeout_secs: Limit, pub total_time_secs: Limit, pub unexpected_change_refreshes: Limit }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditBudget { pub policy: AuditPolicy, pub spent_ms: u64, pub unexpected_refreshes: u64 }
impl AuditBudget {
    pub fn new(policy: AuditPolicy) -> Self { Self { policy, spent_ms: 0, unexpected_refreshes: 0 } }
    pub fn begin(&mut self, _: &str, _: i64, _: bool) -> WorkflowResult<Option<u64>> { Ok(None) }
    pub fn finish(&mut self, _: &str, _: i64) -> WorkflowResult<()> { Ok(()) }
    pub fn heartbeat(&mut self, _: &str, _: i64) -> WorkflowResult<()> { Ok(()) }
    pub fn recover_interrupted(&mut self, _: i64) -> WorkflowResult<()> { Ok(()) }
}
