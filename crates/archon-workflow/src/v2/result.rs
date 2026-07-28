use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2Status {
    Pending,
    Running,
    #[serde(
        alias = "complete",
        alias = "completed",
        alias = "done",
        alias = "success",
        // The prompt teaches "succeeded" for commands_run[].status, so agents
        // reach for it here too. branch_evidence already treats the two as
        // synonyms; only this deserializer disagreed, and the mismatch burned
        // attempts on schema repair that never reached a verdict on the code.
        alias = "succeeded"
    )]
    Accepted,
    Noop,
    Failed,
    Blocked,
    #[serde(
        alias = "needs-review",
        alias = "review",
        alias = "review_required",
        alias = "completed_with_gaps",
        alias = "accepted_with_gaps",
        alias = "partial",
        alias = "partial_success",
        alias = "incomplete",
        alias = "warning"
    )]
    NeedsReview,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkflowV2Result {
    pub status: WorkflowV2Status,
    pub summary: String,
    pub evidence: Vec<WorkflowV2Evidence>,
    pub artifacts: Vec<WorkflowV2Artifact>,
    pub commands_run: Vec<WorkflowV2CommandRecord>,
    pub files_read: Vec<WorkflowV2FileRecord>,
    pub files_changed: Vec<WorkflowV2FileRecord>,
    pub task_coverage: Vec<WorkflowV2TaskCoverage>,
    pub residual_gaps: Vec<WorkflowV2ResidualGap>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub data: serde_json::Value,
}

impl WorkflowV2Result {
    pub fn accepted(summary: impl Into<String>) -> Self {
        Self {
            status: WorkflowV2Status::Accepted,
            summary: summary.into(),
            ..Self::default()
        }
    }

    pub fn noop(summary: impl Into<String>) -> Self {
        Self {
            status: WorkflowV2Status::Noop,
            summary: summary.into(),
            ..Self::default()
        }
    }
}

impl Default for WorkflowV2Result {
    fn default() -> Self {
        Self {
            status: WorkflowV2Status::Pending,
            summary: String::new(),
            evidence: Vec::new(),
            artifacts: Vec::new(),
            commands_run: Vec::new(),
            files_read: Vec::new(),
            files_changed: Vec::new(),
            task_coverage: Vec::new(),
            residual_gaps: Vec::new(),
            data: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2Evidence {
    pub kind: WorkflowV2EvidenceKind,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl WorkflowV2Evidence {
    pub fn new(kind: WorkflowV2EvidenceKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            summary: summary.into(),
            source: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2EvidenceKind {
    #[serde(
        alias = "inspect",
        alias = "task_file",
        alias = "task_files",
        alias = "source_file",
        alias = "source_files"
    )]
    Inspection,
    Implementation,
    Test,
    Review,
    Remediation,
    Blocker,
    Artifact,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2Artifact {
    pub id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2CommandRecord {
    pub kind: WorkflowV2CommandKind,
    pub command: String,
    pub status: WorkflowV2CommandStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub output_summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2CommandKind {
    #[serde(alias = "inspection")]
    Inspect,
    Test,
    Build,
    Format,
    Review,
    Other,
}

impl<'de> Deserialize<'de> for WorkflowV2CommandKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(command_kind_from_str(&raw))
    }
}

fn command_kind_from_str(raw: &str) -> WorkflowV2CommandKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "inspect" | "inspection" => WorkflowV2CommandKind::Inspect,
        "test" => WorkflowV2CommandKind::Test,
        "build" => WorkflowV2CommandKind::Build,
        "format" => WorkflowV2CommandKind::Format,
        "review" => WorkflowV2CommandKind::Review,
        _ => WorkflowV2CommandKind::Other,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2CommandStatus {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2FileRecord {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

impl WorkflowV2FileRecord {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            purpose: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2TaskCoverage {
    pub task_id: String,
    pub status: WorkflowV2TaskCoverageStatus,
    pub summary: String,
    #[serde(default)]
    pub evidence: Vec<WorkflowV2Evidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2TaskCoverageStatus {
    #[serde(
        alias = "complete",
        alias = "completed",
        alias = "done",
        alias = "success",
        // The prompt teaches "succeeded" for commands_run[].status, so agents
        // reach for it here too. branch_evidence already treats the two as
        // synonyms; only this deserializer disagreed, and the mismatch burned
        // attempts on schema repair that never reached a verdict on the code.
        alias = "succeeded"
    )]
    Accepted,
    Noop,
    #[serde(
        alias = "completed_with_gaps",
        alias = "accepted_with_gaps",
        alias = "partial_success",
        alias = "incomplete",
        alias = "warning"
    )]
    Partial,
    Missing,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2ResidualGap {
    pub id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
}

#[cfg(test)]
mod status_alias_tests {
    use super::*;

    /// The prompt teaches agents "succeeded" for `commands_run[].status`, so they
    /// reach for the same word in the result and task-coverage status fields.
    /// Two of TDL-020's three attempts died in schema repair on exactly this,
    /// consuming the attempt without ever producing a verdict on the code.
    #[test]
    fn succeeded_deserializes_as_accepted_in_both_status_enums() {
        let status: WorkflowV2Status = serde_json::from_str("\"succeeded\"")
            .expect("result status must accept the word the prompt teaches");
        assert_eq!(status, WorkflowV2Status::Accepted);

        let coverage: WorkflowV2TaskCoverageStatus = serde_json::from_str("\"succeeded\"")
            .expect("task_coverage status must accept it too — same trap, one field over");
        assert_eq!(coverage, WorkflowV2TaskCoverageStatus::Accepted);
    }

    /// Guards the fix against a future alias cleanup: `success` and `succeeded`
    /// must stay synonyms, matching `branch_evidence`, which already treats them
    /// as one.
    #[test]
    fn success_and_succeeded_agree() {
        let a: WorkflowV2Status = serde_json::from_str("\"success\"").unwrap();
        let b: WorkflowV2Status = serde_json::from_str("\"succeeded\"").unwrap();
        assert_eq!(a, b);
    }

    /// The other TDL-020 failure was NOT a vocabulary collision — the agent
    /// invented a value no path accepts. That must still fail loudly rather than
    /// be normalised into a false pass.
    #[test]
    fn invented_status_values_still_rejected() {
        assert!(serde_json::from_str::<WorkflowV2Status>("\"invalid_evidence\"").is_err());
        assert!(serde_json::from_str::<WorkflowV2CommandStatus>("\"invalid_evidence\"").is_err());
    }
}
