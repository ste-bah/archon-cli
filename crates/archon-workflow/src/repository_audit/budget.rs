//! Durable accounting shared by every assessment in one run.
use crate::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Limit {
    Finite(u64),
    Unlimited,
}
impl Limit {
    pub fn finite(self) -> Option<u64> {
        match self {
            Self::Finite(n) => Some(n),
            Self::Unlimited => None,
        }
    }
    fn milliseconds(self) -> WorkflowResult<Option<u64>> {
        self.finite()
            .map(|n| {
                n.checked_mul(1000)
                    .filter(|v| *v > 0 && *v <= i64::MAX as u64)
                    .ok_or_else(|| {
                        WorkflowError::StateCorrupt(
                            "audit duration is invalid or overflows milliseconds".into(),
                        )
                    })
            })
            .transpose()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditPolicy {
    pub attempt_timeout_secs: Limit,
    pub total_time_secs: Limit,
    pub unexpected_change_refreshes: Limit,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveAttempt {
    pub id: String,
    pub started_ms: i64,
    pub accounted_until_ms: i64,
    pub allowance_ms: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditBudget {
    pub policy: AuditPolicy,
    pub spent_ms: u64,
    pub unexpected_refreshes: u64,
    #[serde(default)]
    pub active: Option<ActiveAttempt>,
    #[serde(default)]
    pub recovered_attempts: Vec<String>,
}
fn paused(reason: &str) -> WorkflowError {
    WorkflowError::ControlPaused(format!(
        "repository audit budget: {reason}; use an authorized operator budget extension"
    ))
}
impl AuditBudget {
    pub fn new(policy: AuditPolicy) -> Self {
        Self {
            policy,
            spent_ms: 0,
            unexpected_refreshes: 0,
            active: None,
            recovered_attempts: vec![],
        }
    }
    /// The store must hold the run lock while reserving and persisting this.
    /// One assessment at a time coalesces boundaries and avoids double spending.
    pub fn begin(
        &mut self,
        id: &str,
        now_ms: i64,
        unexpected: bool,
    ) -> WorkflowResult<Option<u64>> {
        if id.is_empty() || self.active.is_some() {
            return Err(WorkflowError::StateCorrupt(
                "audit attempt missing identity or another attempt is active".into(),
            ));
        }
        let attempt = self.policy.attempt_timeout_secs.milliseconds()?;
        let remaining = self
            .policy
            .total_time_secs
            .milliseconds()?
            .map(|total| total.saturating_sub(self.spent_ms));
        if remaining == Some(0) {
            return Err(paused("cumulative time exhausted"));
        }
        if unexpected
            && self
                .policy
                .unexpected_change_refreshes
                .finite()
                .is_some_and(|limit| self.unexpected_refreshes >= limit)
        {
            return Err(paused("unexpected-change refresh allowance exhausted"));
        }
        let allowance_ms = match (attempt, remaining) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if unexpected {
            self.unexpected_refreshes =
                self.unexpected_refreshes.checked_add(1).ok_or_else(|| {
                    WorkflowError::StateCorrupt("audit refresh count overflow".into())
                })?;
        }
        self.active = Some(ActiveAttempt {
            id: id.into(),
            started_ms: now_ms,
            accounted_until_ms: now_ms,
            allowance_ms,
        });
        Ok(allowance_ms)
    }
    pub fn finish(&mut self, id: &str, now_ms: i64) -> WorkflowResult<()> {
        self.heartbeat(id, now_ms)?;
        self.active = None;
        Ok(())
    }
    pub fn heartbeat(&mut self, id: &str, now_ms: i64) -> WorkflowResult<()> {
        let active =
            self.active.as_mut().filter(|a| a.id == id).ok_or_else(|| {
                WorkflowError::StateCorrupt("audit attempt identity changed".into())
            })?;
        let delta = now_ms
            .checked_sub(active.accounted_until_ms)
            .filter(|n| *n >= 0)
            .ok_or_else(|| {
                WorkflowError::ControlPaused(
                    "audit clock moved backwards; recorded usage retained".into(),
                )
            })? as u64;
        self.spent_ms = self
            .spent_ms
            .checked_add(delta)
            .ok_or_else(|| WorkflowError::StateCorrupt("audit time accounting overflow".into()))?;
        active.accounted_until_ms = now_ms;
        Ok(())
    }
    /// On takeover, charge the unknown interval conservatively and name it.
    /// Only the generation-owning host may recover a prior active attempt.
    pub fn recover_interrupted(&mut self, now_ms: i64) -> WorkflowResult<()> {
        if let Some(active) = self.active.clone() {
            self.finish(&active.id, now_ms)?;
            self.recovered_attempts.push(active.id);
        }
        Ok(())
    }
}
