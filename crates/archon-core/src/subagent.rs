use std::collections::HashMap;

use archon_tools::agent_tool::SubagentRequest;
use chrono::{DateTime, Utc};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Auto-background configuration (AGT-025)
// ---------------------------------------------------------------------------

/// Default auto-background timeout in milliseconds (120 seconds).
pub const AUTO_BACKGROUND_MS: u64 = 120_000;

/// Check if auto-backgrounding is enabled via environment variable.
///
/// When enabled, foreground sync agents that run longer than `AUTO_BACKGROUND_MS`
/// are automatically converted to background agents. The agent continues running
/// but the parent stops waiting synchronously.
pub fn is_auto_background_enabled() -> bool {
    std::env::var("ARCHON_AUTO_BACKGROUND_TASKS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Returns the auto-background timeout in milliseconds, or 0 if disabled.
pub fn get_auto_background_ms() -> u64 {
    if is_auto_background_enabled() {
        AUTO_BACKGROUND_MS
    } else {
        0
    }
}

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
// Progress tracking (TASK-T3 / G4) — per-agent live progress counters
// ---------------------------------------------------------------------------

/// A single tool invocation recorded on the recent-activity ring.
#[derive(Debug, Clone)]
pub struct ToolActivity {
    pub tool_name: String,
    pub timestamp: DateTime<Utc>,
}

/// Mutable per-agent progress state shared between the runner (writer) and
/// observers (readers).  Wrapped in `std::sync::Mutex` so the runner can
/// update it from inside its synchronous stream-event match arms without
/// holding any lock across an `.await`.
#[derive(Debug, Clone)]
pub struct ProgressTracker {
    pub tool_use_count: u32,
    pub cumulative_input_tokens: u64,
    pub cumulative_output_tokens: u64,
    pub cumulative_cache_creation_tokens: u64,
    pub cumulative_cache_read_tokens: u64,
    /// Bounded ring of the most recent tool dispatches (cap 5).
    pub recent_activities: std::collections::VecDeque<ToolActivity>,
    pub last_update: DateTime<Utc>,
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self {
            tool_use_count: 0,
            cumulative_input_tokens: 0,
            cumulative_output_tokens: 0,
            cumulative_cache_creation_tokens: 0,
            cumulative_cache_read_tokens: 0,
            recent_activities: std::collections::VecDeque::new(),
            last_update: Utc::now(),
        }
    }
}

/// Read-only snapshot returned by `SubagentManager::get_progress`.
#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    pub tool_use_count: u32,
    pub cumulative_input_tokens: u64,
    pub cumulative_output_tokens: u64,
    pub cumulative_cache_creation_tokens: u64,
    pub cumulative_cache_read_tokens: u64,
    pub recent_activities: Vec<ToolActivity>,
    pub last_update: DateTime<Utc>,
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
    /// Flag for graceful shutdown (set by SendMessage shutdown_request).
    pub shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// TASK-T3 (G4): live progress counters shared with the runner.
    pub progress: std::sync::Arc<std::sync::Mutex<ProgressTracker>>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum SubagentError {
    #[error("subagent not found: {0}")]
    NotFound(String),

    #[error("subagent already exists and is running: {0}")]
    AlreadyRunning(String),

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
    /// Name registry: maps agent display names to agent IDs.
    /// Populated when Agent tool spawns a named agent, cleared on completion.
    name_registry: HashMap<String, String>,
    /// Pending messages queued for delivery at next tool round boundary.
    /// Key: agent_id, Value: queued messages (FIFO order).
    pending_messages: HashMap<String, Vec<String>>,
}

impl SubagentManager {
    /// Default maximum concurrent subagents.
    pub const DEFAULT_MAX_CONCURRENT: usize = 4;

    pub fn new(max_concurrent: usize) -> Self {
        Self {
            agents: HashMap::new(),
            max_concurrent,
            name_registry: HashMap::new(),
            pending_messages: HashMap::new(),
        }
    }

    /// The configured maximum number of concurrently running subagents.
    ///
    /// This is the authoritative live cap that `register` enforces. Fan-out
    /// schedulers query it (via the executor) to clamp their semaphore so they
    /// never admit more work than the manager will accept.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// Register a new subagent request.  Returns the UUID assigned.
    pub fn register(&mut self, request: SubagentRequest) -> Result<String, SubagentError> {
        self.register_with_id(Uuid::new_v4().to_string(), request)
    }

    /// Register a subagent under a caller-provided id.
    ///
    /// This keeps the id returned to the model aligned with manager status,
    /// transcript files, progress lookup, SendMessage, and worktree cleanup.
    /// A stopped entry may be reused for resume; a running duplicate is rejected.
    pub fn register_with_id(
        &mut self,
        id: String,
        request: SubagentRequest,
    ) -> Result<String, SubagentError> {
        let active = self
            .agents
            .values()
            .filter(|a| a.status == SubagentStatus::Running)
            .count();

        if let Some(existing) = self.agents.get_mut(&id) {
            if existing.status == SubagentStatus::Running {
                return Err(SubagentError::AlreadyRunning(id));
            }
            if active >= self.max_concurrent {
                return Err(SubagentError::MaxConcurrent(self.max_concurrent));
            }
            self.pending_messages.remove(&id);
            self.name_registry
                .retain(|_, existing_id| existing_id != &id);
            existing.request = request;
            existing.status = SubagentStatus::Running;
            existing.created_at = Utc::now();
            existing.result = None;
            existing.shutdown_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            existing.progress =
                std::sync::Arc::new(std::sync::Mutex::new(ProgressTracker::default()));
            return Ok(id);
        }

        if active >= self.max_concurrent {
            return Err(SubagentError::MaxConcurrent(self.max_concurrent));
        }

        let info = SubagentInfo {
            id: id.clone(),
            request,
            status: SubagentStatus::Running,
            created_at: Utc::now(),
            result: None,
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            progress: std::sync::Arc::new(std::sync::Mutex::new(ProgressTracker::default())),
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

    // -------------------------------------------------------------------
    // Agent name registry (AGT-026)
    // -------------------------------------------------------------------

    /// Register a human-readable name for an agent ID.
    /// Called when Agent tool spawns a named agent.
    pub fn register_name(&mut self, name: String, agent_id: String) {
        self.name_registry.insert(name, agent_id);
    }

    /// Remove a name from the registry. Called when agent completes.
    pub fn unregister_name(&mut self, name: &str) {
        self.name_registry.remove(name);
    }

    /// Resolve an agent name to its ID, if registered.
    pub fn resolve_name(&self, name: &str) -> Option<&str> {
        self.name_registry.get(name).map(|s| s.as_str())
    }

    /// Check if an agent ID is currently running.
    pub fn is_running(&self, agent_id: &str) -> bool {
        self.agents
            .get(agent_id)
            .map(|info| info.status == SubagentStatus::Running)
            .unwrap_or(false)
    }

    /// Check if an agent ID exists in state (running or stopped).
    pub fn has_agent(&self, agent_id: &str) -> bool {
        self.agents.contains_key(agent_id)
    }

    // -------------------------------------------------------------------
    // Pending messages (AGT-026)
    // -------------------------------------------------------------------

    /// Queue a message for delivery to a running agent.
    /// Messages are delivered at the next tool round boundary.
    pub fn queue_pending_message(&mut self, agent_id: &str, message: String) {
        self.pending_messages
            .entry(agent_id.to_string())
            .or_default()
            .push(message);
    }

    /// Drain all pending messages for an agent (take + clear).
    /// Returns empty vec if no messages queued.
    pub fn drain_pending_messages(&mut self, agent_id: &str) -> Vec<String> {
        self.pending_messages.remove(agent_id).unwrap_or_default()
    }

    /// Request graceful shutdown of a running agent.
    pub fn request_shutdown(&self, agent_id: &str) -> bool {
        if let Some(info) = self.agents.get(agent_id) {
            info.shutdown_flag
                .store(true, std::sync::atomic::Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Get a clone of the shutdown flag for a registered agent.
    pub fn get_shutdown_flag(
        &self,
        agent_id: &str,
    ) -> Option<std::sync::Arc<std::sync::atomic::AtomicBool>> {
        self.agents
            .get(agent_id)
            .map(|info| std::sync::Arc::clone(&info.shutdown_flag))
    }

    // -------------------------------------------------------------------
    // Progress tracking (TASK-T3 / G4)
    // -------------------------------------------------------------------

    /// Clone the progress tracker `Arc` for an agent so the runner can write
    /// to it directly.  Returns `None` if the agent is unknown.
    pub fn get_progress_tracker_arc(
        &self,
        id: &str,
    ) -> Option<std::sync::Arc<std::sync::Mutex<ProgressTracker>>> {
        self.agents.get(id).map(|info| info.progress.clone())
    }

    /// Take a read-only snapshot of an agent's current progress.
    /// Returns `None` if the agent is unknown or the inner mutex is poisoned.
    pub fn get_progress(&self, id: &str) -> Option<ProgressSnapshot> {
        let info = self.agents.get(id)?;
        let guard = info.progress.lock().ok()?;
        Some(ProgressSnapshot {
            tool_use_count: guard.tool_use_count,
            cumulative_input_tokens: guard.cumulative_input_tokens,
            cumulative_output_tokens: guard.cumulative_output_tokens,
            cumulative_cache_creation_tokens: guard.cumulative_cache_creation_tokens,
            cumulative_cache_read_tokens: guard.cumulative_cache_read_tokens,
            recent_activities: guard.recent_activities.iter().cloned().collect(),
            last_update: guard.last_update,
        })
    }

    /// Clean up all state for an agent on completion:
    /// drops pending messages, removes from name registry if name matches.
    pub fn cleanup_agent(&mut self, agent_id: &str) {
        self.pending_messages.remove(agent_id);
        // Remove from name registry if this ID is registered
        self.name_registry.retain(|_, id| id != agent_id);
    }
}

impl Default for SubagentManager {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_CONCURRENT)
    }
}

// ---------------------------------------------------------------------------
// SubagentRunner — multi-turn agentic loop with tool dispatch (AGT-009)
// ---------------------------------------------------------------------------

pub mod runner;

#[cfg(test)]
mod tests;
