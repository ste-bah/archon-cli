use std::collections::HashMap;

use archon_tools::agent_tool::SubagentRequest;
use chrono::{DateTime, Utc};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Subagent status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentStatus {
    Running,
    Completed,
    TimedOut,
    Failed(String),
}

// ---------------------------------------------------------------------------
// Subagent info — tracks a single subagent's lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SubagentInfo {
    pub id: String,
    pub request: SubagentRequest,
    pub status: SubagentStatus,
    pub created_at: DateTime<Utc>,
    pub result: Option<String>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SubagentError {
    #[error("subagent not found: {0}")]
    NotFound(String),

    #[error("max concurrent subagents reached ({0})")]
    MaxConcurrent(usize),

    #[error("subagent {0} is not in Running state")]
    NotRunning(String),
}

// ---------------------------------------------------------------------------
// SubagentManager — manages subagent lifecycle
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SubagentManager {
    agents: HashMap<String, SubagentInfo>,
    max_concurrent: usize,
}

impl SubagentManager {
    /// Default maximum concurrent subagents.
    pub const DEFAULT_MAX_CONCURRENT: usize = 4;

    pub fn new(max_concurrent: usize) -> Self {
        Self {
            agents: HashMap::new(),
            max_concurrent,
        }
    }

    /// Register a new subagent request.  Returns the UUID assigned.
    pub fn register(&mut self, request: SubagentRequest) -> Result<String, SubagentError> {
        let active = self
            .agents
            .values()
            .filter(|a| a.status == SubagentStatus::Running)
            .count();

        if active >= self.max_concurrent {
            return Err(SubagentError::MaxConcurrent(self.max_concurrent));
        }

        let id = Uuid::new_v4().to_string();
        let info = SubagentInfo {
            id: id.clone(),
            request,
            status: SubagentStatus::Running,
            created_at: Utc::now(),
            result: None,
        };
        self.agents.insert(id.clone(), info);
        Ok(id)
    }

    /// Get the status of a subagent by id.
    pub fn get_status(&self, id: &str) -> Option<&SubagentInfo> {
        self.agents.get(id)
    }

    /// List all currently-running subagents.
    pub fn list_active(&self) -> Vec<&SubagentInfo> {
        self.agents
            .values()
            .filter(|a| a.status == SubagentStatus::Running)
            .collect()
    }

    /// Mark a subagent as completed with the given result.
    pub fn complete(&mut self, id: &str, result: String) -> Result<(), SubagentError> {
        let info = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SubagentError::NotFound(id.to_string()))?;

        if info.status != SubagentStatus::Running {
            return Err(SubagentError::NotRunning(id.to_string()));
        }

        info.status = SubagentStatus::Completed;
        info.result = Some(result);
        Ok(())
    }

    /// Mark a subagent as timed out.
    pub fn mark_timed_out(&mut self, id: &str) -> Result<(), SubagentError> {
        let info = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SubagentError::NotFound(id.to_string()))?;

        if info.status != SubagentStatus::Running {
            return Err(SubagentError::NotRunning(id.to_string()));
        }

        info.status = SubagentStatus::TimedOut;
        Ok(())
    }

    /// Mark a subagent as failed with a reason.
    pub fn mark_failed(&mut self, id: &str, reason: String) -> Result<(), SubagentError> {
        let info = self
            .agents
            .get_mut(id)
            .ok_or_else(|| SubagentError::NotFound(id.to_string()))?;

        if info.status != SubagentStatus::Running {
            return Err(SubagentError::NotRunning(id.to_string()));
        }

        info.status = SubagentStatus::Failed(reason);
        Ok(())
    }

    /// Total number of tracked subagents (all statuses).
    pub fn total_count(&self) -> usize {
        self.agents.len()
    }
}

impl Default for SubagentManager {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_CONCURRENT)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> SubagentRequest {
        SubagentRequest {
            prompt: "Analyze the codebase".into(),
            model: Some("claude-sonnet-4-6".into()),
            allowed_tools: vec!["Read".into(), "Glob".into()],
            max_turns: 10,
            timeout_secs: 300,
        }
    }

    #[test]
    fn register_returns_uuid() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).expect("should register");

        // UUID v4 format: 8-4-4-4-12 hex chars
        assert_eq!(id.len(), 36);
        assert!(id.contains('-'));
    }

    #[test]
    fn get_status_returns_running() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        let info = mgr.get_status(&id).expect("should exist");
        assert_eq!(info.status, SubagentStatus::Running);
        assert_eq!(info.request.prompt, "Analyze the codebase");
        assert!(info.result.is_none());
    }

    #[test]
    fn list_active_only_returns_running() {
        let mut mgr = SubagentManager::default();
        let id1 = mgr.register(sample_request()).unwrap();
        let _id2 = mgr.register(sample_request()).unwrap();

        assert_eq!(mgr.list_active().len(), 2);

        mgr.complete(&id1, "done".into()).unwrap();
        assert_eq!(mgr.list_active().len(), 1);
    }

    #[test]
    fn complete_sets_result() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        mgr.complete(&id, "task finished successfully".into())
            .unwrap();

        let info = mgr.get_status(&id).unwrap();
        assert_eq!(info.status, SubagentStatus::Completed);
        assert_eq!(info.result.as_deref(), Some("task finished successfully"));
    }

    #[test]
    fn complete_nonexistent_returns_error() {
        let mut mgr = SubagentManager::default();
        let err = mgr.complete("fake-id", "result".into()).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn complete_already_completed_returns_error() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();
        mgr.complete(&id, "first".into()).unwrap();

        let err = mgr.complete(&id, "second".into()).unwrap_err();
        assert!(err.to_string().contains("not in Running state"));
    }

    #[test]
    fn max_concurrent_enforced() {
        let mut mgr = SubagentManager::new(2);
        mgr.register(sample_request()).unwrap();
        mgr.register(sample_request()).unwrap();

        let err = mgr.register(sample_request()).unwrap_err();
        assert!(err.to_string().contains("max concurrent"));
    }

    #[test]
    fn mark_timed_out_works() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        mgr.mark_timed_out(&id).unwrap();
        assert_eq!(
            mgr.get_status(&id).unwrap().status,
            SubagentStatus::TimedOut
        );
    }

    #[test]
    fn mark_failed_works() {
        let mut mgr = SubagentManager::default();
        let id = mgr.register(sample_request()).unwrap();

        mgr.mark_failed(&id, "something went wrong".into()).unwrap();
        assert_eq!(
            mgr.get_status(&id).unwrap().status,
            SubagentStatus::Failed("something went wrong".into())
        );
    }

    #[test]
    fn total_count_tracks_all() {
        let mut mgr = SubagentManager::default();
        assert_eq!(mgr.total_count(), 0);

        let id = mgr.register(sample_request()).unwrap();
        assert_eq!(mgr.total_count(), 1);

        mgr.complete(&id, "done".into()).unwrap();
        // completed agents still tracked
        assert_eq!(mgr.total_count(), 1);

        mgr.register(sample_request()).unwrap();
        assert_eq!(mgr.total_count(), 2);
    }

    #[test]
    fn get_status_nonexistent_returns_none() {
        let mgr = SubagentManager::default();
        assert!(mgr.get_status("nonexistent").is_none());
    }
}
