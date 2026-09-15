//! Human-facing effective allowances and durable consumption.
use super::{budget::Limit, runtime::AuditState};
use crate::WorkflowResult;
use serde_json::{Value, json};

fn limit(value: Limit) -> Value {
    match value { Limit::Finite(n) => json!(n), Limit::Unlimited => json!("unlimited") }
}
impl AuditState {
    pub fn status(&self) -> WorkflowResult<Value> {
        let budget = &self.budget;
        let remaining_time = match budget.policy.total_time_secs {
            Limit::Unlimited => json!("unlimited"),
            Limit::Finite(seconds) => json!(seconds.checked_mul(1000)
                .ok_or_else(|| crate::WorkflowError::StateCorrupt("audit duration overflow".into()))?
                .saturating_sub(budget.spent_ms)),
        };
        let remaining_refreshes = match budget.policy.unexpected_change_refreshes {
            Limit::Unlimited => json!("unlimited"),
            Limit::Finite(n) => json!(n.saturating_sub(budget.unexpected_refreshes)),
        };
        let unresolved = self.snapshot.as_ref().map(|snapshot| self.ledger.unresolved(&snapshot.identity)).transpose()?;
        Ok(json!({"schema_version":1,"generation":self.generation,
            "policy_provenance":self.policy_provenance,"attempt_timeout_secs":limit(budget.policy.attempt_timeout_secs),
            "total_time_secs":limit(budget.policy.total_time_secs),
            "unexpected_change_refreshes":limit(budget.policy.unexpected_change_refreshes),
            "spent_ms":budget.spent_ms,"spent_unexpected_refreshes":budget.unexpected_refreshes,
            "remaining_time_ms":remaining_time,"remaining_unexpected_refreshes":remaining_refreshes,
            "active_attempt":budget.active,"attempts":self.attempts,"last_error":self.last_error,
            "snapshot":self.snapshot,"declared_paths":self.declared_paths,"ignored_paths":self.ledger.ignored_paths,
            "unresolved_paths":unresolved,"waivers":self.ledger.waivers,
            "corrections":self.ledger.corrections,"reassessments":self.ledger.reassessments,"operator_controls":self.operator_controls,
            "final_receipt":self.final_receipt,"recovered_attempts":budget.recovered_attempts,
            "usage_note":"Persisted execution time includes provider waits and retries; active usage updates every five seconds."}))
    }
}
