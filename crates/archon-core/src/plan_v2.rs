use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use archon_session::storage::SessionStore;

// ---------------------------------------------------------------------------
// Plan status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanStatus {
    Draft,
    Approved,
    InProgress,
    Complete,
    Abandoned,
}

// ---------------------------------------------------------------------------
// Step status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    InProgress,
    Complete,
    Skipped,
}

// ---------------------------------------------------------------------------
// Plan step
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// 1-indexed step number.
    pub index: usize,
    pub description: String,
    /// Files affected by this step.
    pub files: Vec<String>,
    /// Complexity: "low", "medium", or "high".
    pub complexity: String,
    pub status: StepStatus,
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub title: String,
    pub steps: Vec<PlanStep>,
    pub status: PlanStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Plan {
    /// Create a new plan with `Draft` status and no steps.
    pub fn new(title: &str) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.to_string(),
            steps: Vec::new(),
            status: PlanStatus::Draft,
            created_at: now,
            updated_at: now,
        }
    }

    /// Add a step to the plan. The index is auto-assigned (1-indexed).
    pub fn add_step(&mut self, description: &str, files: Vec<String>, complexity: &str) {
        let index = self.steps.len() + 1;
        self.steps.push(PlanStep {
            index,
            description: description.to_string(),
            files,
            complexity: complexity.to_string(),
            status: StepStatus::Pending,
        });
        self.updated_at = Utc::now();
    }

    /// Mark a step as in-progress by its 1-based index.
    pub fn mark_step_in_progress(&mut self, index: usize) -> Result<(), String> {
        let step = self
            .find_step_mut(index)
            .ok_or_else(|| format!("step index {index} not found"))?;
        step.status = StepStatus::InProgress;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Mark a step as complete by its 1-based index.
    pub fn mark_step_complete(&mut self, index: usize) -> Result<(), String> {
        let step = self
            .find_step_mut(index)
            .ok_or_else(|| format!("step index {index} not found"))?;
        step.status = StepStatus::Complete;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Mark a step as skipped by its 1-based index.
    pub fn mark_step_skipped(&mut self, index: usize) -> Result<(), String> {
        let step = self
            .find_step_mut(index)
            .ok_or_else(|| format!("step index {index} not found"))?;
        step.status = StepStatus::Skipped;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// Returns `(completed, total)` step counts.
    pub fn overall_progress(&self) -> (usize, usize) {
        let completed = self
            .steps
            .iter()
            .filter(|s| s.status == StepStatus::Complete)
            .count();
        (completed, self.steps.len())
    }

    /// Formatted display for the `/plan` command.
    pub fn to_display(&self) -> String {
        let mut out = format!("Plan: {}\n", self.title);
        let (done, total) = self.overall_progress();
        out.push_str(&format!("Status: {:?}  ({done}/{total} complete)\n\n", self.status));

        for step in &self.steps {
            let icon = match step.status {
                StepStatus::Pending => "\u{25cb}",    // ○
                StepStatus::InProgress => "\u{25d4}",  // ◔
                StepStatus::Complete => "\u{2713}",     // ✓
                StepStatus::Skipped => "\u{2212}",      // −
            };
            let check = match step.status {
                StepStatus::Complete => "[x]",
                _ => "[ ]",
            };
            out.push_str(&format!(
                "  {check} {icon} {}. {} [{}]\n",
                step.index, step.description, step.complexity
            ));
            if !step.files.is_empty() {
                out.push_str(&format!("       files: {}\n", step.files.join(", ")));
            }
        }
        out
    }

    /// Format for injection into a system prompt.
    pub fn to_prompt_block(&self) -> String {
        let mut out = format!("Plan: {}\n", self.title);
        let (done, total) = self.overall_progress();
        out.push_str(&format!("Progress: {done}/{total}\n"));

        for step in &self.steps {
            let marker = match step.status {
                StepStatus::Pending => "PENDING",
                StepStatus::InProgress => "IN_PROGRESS",
                StepStatus::Complete => "COMPLETE",
                StepStatus::Skipped => "SKIPPED",
            };
            out.push_str(&format!(
                "  [{marker}] {}. {}\n",
                step.index, step.description
            ));
        }
        out
    }

    /// Auto-mark a step as in-progress if it lists the edited file
    /// and is currently pending.
    pub fn on_file_edited(&mut self, file_path: &str) {
        let mut changed = false;
        for step in &mut self.steps {
            if step.status == StepStatus::Pending
                && step.files.iter().any(|f| f == file_path)
            {
                step.status = StepStatus::InProgress;
                changed = true;
            }
        }
        if changed {
            self.updated_at = Utc::now();
        }
    }

    /// Create a [`SubagentRequest`] to explore a specific step.
    ///
    /// The subagent is restricted to read-only tools and given a prompt
    /// that asks it to investigate the step's requirements.
    pub fn explore_step(&self, step_index: usize) -> Result<archon_tools::agent_tool::SubagentRequest, String> {
        let step = self.steps.iter().find(|s| s.index == step_index)
            .ok_or_else(|| format!("step index {step_index} not found"))?;

        let files_hint = if step.files.is_empty() {
            String::new()
        } else {
            format!("\n\nRelevant files: {}", step.files.join(", "))
        };

        let query = format!(
            "Explore step {} of plan '{}': {}{files_hint}",
            step.index, self.title, step.description
        );

        Ok(crate::plan_explore::create_explore_request(&query))
    }

    // ── private helpers ────────────────────────────────────────────

    fn find_step_mut(&mut self, index: usize) -> Option<&mut PlanStep> {
        self.steps.iter_mut().find(|s| s.index == index)
    }
}

// ---------------------------------------------------------------------------
// Persistence — uses session messages table as a key-value store
// ---------------------------------------------------------------------------

/// The message index used to store the plan JSON inside the session.
const PLAN_MESSAGE_INDEX: u64 = u64::MAX - 1;

/// Prefix prepended to the serialized plan for identification.
const PLAN_PREFIX: &str = "__archon_plan__:";

/// Save a plan to session storage.
pub fn save_plan(store: &SessionStore, session_id: &str, plan: &Plan) -> Result<(), String> {
    let json = serde_json::to_string(plan).map_err(|e| format!("serialize plan: {e}"))?;
    let content = format!("{PLAN_PREFIX}{json}");
    store
        .save_message(session_id, PLAN_MESSAGE_INDEX, &content)
        .map_err(|e| format!("save plan to session store: {e}"))
}

/// Load a plan from session storage. Returns `None` if no plan is stored.
pub fn load_plan(store: &SessionStore, session_id: &str) -> Result<Option<Plan>, String> {
    let messages = store
        .load_messages(session_id)
        .map_err(|e| format!("load messages: {e}"))?;

    // The plan is stored at a very high message index, so it will be the last
    // entry if present.  Scan from the end for the prefix.
    for msg in messages.iter().rev() {
        if let Some(json_str) = msg.strip_prefix(PLAN_PREFIX) {
            let plan: Plan =
                serde_json::from_str(json_str).map_err(|e| format!("deserialize plan: {e}"))?;
            return Ok(Some(plan));
        }
    }
    Ok(None)
}
