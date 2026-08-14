use archon_completion::RequiredEvidenceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Complete,
    Skipped,
}

impl std::fmt::Display for PlanStepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Complete => write!(f, "complete"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

impl PlanStepStatus {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "complete" => Self::Complete,
            "skipped" => Self::Skipped,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    #[default]
    Draft,
    Approved,
    #[serde(alias = "active")]
    Executing,
    #[serde(alias = "complete")]
    Completed,
    Abandoned,
}

impl std::fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Draft => "draft",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanApprovalDecision {
    Approve,
    ApproveAcceptEdits,
    Reject { reason: String },
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanApprovalSource {
    Interactive,
    NonInteractive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanReconciliationStatus {
    Completed,
    Deviated,
    Omitted,
    UnplannedExtra,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanApproval {
    pub decision: PlanApprovalDecision,
    pub source: PlanApprovalSource,
    pub decided_at: String,
    pub user_edited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanApprovalRecord {
    pub plan_id: String,
    pub session_id: String,
    pub approval: PlanApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlanStepDependency {
    pub step: u32,
    #[serde(default)]
    pub blocked_by: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStepReconciliation {
    pub step: Option<u32>,
    pub status: PlanReconciliationStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub number: u32,
    pub description: String,
    pub affected_files: Vec<String>,
    pub status: PlanStepStatus,
    #[serde(default)]
    pub blocked_by: Vec<u32>,
    #[serde(default)]
    pub required_evidence: Vec<RequiredEvidenceKind>,
    #[serde(default)]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDocument {
    pub id: String,
    pub title: String,
    pub steps: Vec<PlanStep>,
    pub risks: Vec<String>,
    pub questions: Vec<String>,
    #[serde(default)]
    pub status: PlanStatus,
    #[serde(default)]
    pub approval: Option<PlanApproval>,
    #[serde(default)]
    pub reconciliation: Vec<PlanStepReconciliation>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub commits: Vec<String>,
    #[serde(default)]
    pub user_edited: bool,
}

impl PlanDocument {
    pub fn new(id: &str, title: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            steps: Vec::new(),
            risks: Vec::new(),
            questions: Vec::new(),
            status: PlanStatus::Draft,
            approval: None,
            reconciliation: Vec::new(),
            session_id: None,
            branch: None,
            commits: Vec::new(),
            user_edited: false,
        }
    }

    /// Serialize the plan to JSON for storage.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Deserialize from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Get completion percentage.
    pub fn completion_pct(&self) -> f32 {
        if self.steps.is_empty() {
            return 0.0;
        }
        let done = self
            .steps
            .iter()
            .filter(|s| matches!(s.status, PlanStepStatus::Complete | PlanStepStatus::Skipped))
            .count();
        (done as f32 / self.steps.len() as f32) * 100.0
    }

    /// Format as human-readable text for injection into context.
    pub fn to_context_string(&self) -> String {
        let mut out = format!(
            "## Plan: {}\nStatus: {} ({:.0}% complete)\n\n",
            self.title,
            self.status,
            self.completion_pct()
        );
        for step in &self.steps {
            let marker = match step.status {
                PlanStepStatus::Complete => "[x]",
                PlanStepStatus::InProgress => "[>]",
                PlanStepStatus::Skipped => "[-]",
                PlanStepStatus::Pending => "[ ]",
            };
            out.push_str(&format!(
                "{} {}. {}\n",
                marker, step.number, step.description
            ));
            if !step.affected_files.is_empty() {
                out.push_str(&format!("    Files: {}\n", step.affected_files.join(", ")));
            }
        }
        if !self.risks.is_empty() {
            out.push_str("\nRisks:\n");
            for risk in &self.risks {
                out.push_str(&format!("  - {risk}\n"));
            }
        }
        if !self.questions.is_empty() {
            out.push_str("\nOpen questions:\n");
            for question in &self.questions {
                out.push_str(&format!("  - {question}\n"));
            }
        }
        out
    }
}
