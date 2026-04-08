//! Rehydration Policy — loads exactly 5 items on session resume.
//!
//! Implements PRD Section 10.3: mission brief, current task state,
//! 3-10 relevant memories, latest blocker, wiring checklist.

use std::path::PathBuf;

use anyhow::Result;

use crate::coding::contract::TaskContract;
use crate::coding::wiring::WiringPlan;
use crate::ledgers::{TaskEntry, VerificationEntry};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Context loaded after rehydration — injected into agent prompts.
#[derive(Debug, Clone)]
pub struct RehydrationContext {
    /// Rendered mission brief template (PRD Section 10.4).
    pub mission_brief: String,
    /// Current task state from the task ledger.
    pub current_task_state: Option<TaskEntry>,
    /// 3-10 targeted memory recalls (capped at 10).
    pub relevant_memories: Vec<String>,
    /// Most recent gate failure or compiler error.
    pub latest_blocker: Option<String>,
    /// Wiring plan loaded from file (survives compaction).
    pub wiring_checklist: Option<WiringPlan>,
}

// ---------------------------------------------------------------------------
// Policy
// ---------------------------------------------------------------------------

/// Loads the 5 rehydration items per PRD Section 10.3.
pub struct RehydrationPolicy {
    session_dir: PathBuf,
}

impl RehydrationPolicy {
    pub fn new(session_dir: PathBuf) -> Self {
        Self { session_dir }
    }

    /// Load all 5 rehydration items and assemble a RehydrationContext.
    ///
    /// Missing files produce None values, not errors.
    /// Memory recall is deferred to the caller (runner provides memories).
    pub fn rehydrate(&self, _agent_key: &str, _task_type: &str) -> Result<RehydrationContext> {
        // 1. Load contract -> render mission brief
        let contract = self.load_contract();
        let wiring_plan = self.load_wiring_plan();

        let mission_brief = match &contract {
            Some(c) => render_mission_brief(c, &wiring_plan, 0, "", 0, 0),
            None => String::new(),
        };

        // 2. Load current task state from task ledger
        let current_task_state = self.load_current_task();

        // 3. Relevant memories — placeholder, runner injects from MemoryGraph
        let relevant_memories = Vec::new();

        // 4. Load latest blocker from verification ledger
        let latest_blocker = self.load_latest_blocker();

        Ok(RehydrationContext {
            mission_brief,
            current_task_state,
            relevant_memories,
            latest_blocker,
            wiring_checklist: wiring_plan,
        })
    }

    // -----------------------------------------------------------------------
    // Internal loaders
    // -----------------------------------------------------------------------

    fn load_contract(&self) -> Option<TaskContract> {
        let path = self.session_dir.join("contract.json");
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn load_wiring_plan(&self) -> Option<WiringPlan> {
        let path = self.session_dir.join("wiring-plan.json");
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn load_current_task(&self) -> Option<TaskEntry> {
        let path = self.session_dir.join("ledgers/tasks.json");
        let data = std::fs::read_to_string(&path).ok()?;
        let entries: Vec<TaskEntry> = serde_json::from_str(&data).ok()?;
        // Return the most recent entry (last in the list)
        entries.into_iter().last()
    }

    fn load_latest_blocker(&self) -> Option<String> {
        let path = self.session_dir.join("ledgers/verifications.json");
        let data = std::fs::read_to_string(&path).ok()?;
        let entries: Vec<VerificationEntry> = serde_json::from_str(&data).ok()?;
        // Find the most recent failure (last failed entry)
        entries.iter().rev().find(|e| !e.passed).map(|e| {
            let details = e.failure_details.as_deref().unwrap_or("no details");
            format!("[{}] {}: {}", e.gate_name, e.evidence_summary, details)
        })
    }
}

// ---------------------------------------------------------------------------
// Mission Brief Template (PRD Section 10.4)
// ---------------------------------------------------------------------------

/// Render the canonical mission brief from contract and context.
pub fn render_mission_brief(
    contract: &TaskContract,
    wiring_plan: &Option<WiringPlan>,
    phase: u8,
    agent_key: &str,
    agents_finished: usize,
    agents_total: usize,
) -> String {
    let acceptance = contract
        .acceptance_criteria
        .iter()
        .map(|ac| format!("  - [{}] {}", ac.id, ac.description))
        .collect::<Vec<_>>()
        .join("\n");

    let non_goals = if contract.non_goals.is_empty() {
        "  (none)".to_string()
    } else {
        contract
            .non_goals
            .iter()
            .map(|ng| format!("  - {}", ng))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let wiring = match wiring_plan {
        Some(plan) if !plan.obligations.is_empty() => plan
            .obligations
            .iter()
            .map(|o| {
                format!(
                    "  - [{}] {} -> {:?} ({})",
                    o.id,
                    o.file,
                    o.action,
                    o.status_str()
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => "  No wiring plan yet".to_string(),
    };

    let definition_of = contract
        .definition_of_done
        .iter()
        .map(|d| format!("  - {}", d))
        .collect::<Vec<_>>()
        .join("\n");

    let mut brief = String::new();
    brief.push_str("### Mission Brief\n");
    brief.push_str(&format!("Task: {}\n", contract.goal));
    brief.push_str(&format!("Acceptance Criteria:\n{}\n", acceptance));
    brief.push_str(&format!("Non-Goals:\n{}\n", non_goals));
    brief.push_str(&format!("Wiring Obligations:\n{}\n", wiring));
    brief.push_str(&format!("Completion Criteria:\n{}\n", definition_of));
    brief.push_str(&format!(
        "Current Phase: {} | Agent: {} | Progress: {}/{}",
        phase, agent_key, agents_finished, agents_total,
    ));
    brief
}

// ---------------------------------------------------------------------------
// Helper for WiringObligation status display
// ---------------------------------------------------------------------------

impl crate::coding::wiring::WiringObligation {
    /// Human-readable status string.
    pub fn status_str(&self) -> &'static str {
        match self.status {
            crate::coding::wiring::ObligationStatus::Pending => "Pending",
            crate::coding::wiring::ObligationStatus::Met => "Met",
            crate::coding::wiring::ObligationStatus::Failed => "Failed",
        }
    }
}
