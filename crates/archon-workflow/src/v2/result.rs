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

/// Evidence classification.
///
/// The `Other` arm existed from the start, but the derived deserializer could
/// never reach it — an unrecognised kind aborted the whole result instead. A
/// verification branch was lost to the word `build`, which is legal on
/// `commands_run[].kind` and so an entirely reasonable thing for an agent to
/// write here. Labelling is cosmetic, so unknown input lands on `Other` and the
/// evidence survives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

/// Coerce an evidence kind, defaulting to `Other` for anything unrecognised.
fn evidence_kind_from_str(raw: &str) -> WorkflowV2EvidenceKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "inspection" | "inspect" | "task_file" | "task_files" | "source_file" | "source_files" => {
            WorkflowV2EvidenceKind::Inspection
        }
        "implementation" => WorkflowV2EvidenceKind::Implementation,
        "test" => WorkflowV2EvidenceKind::Test,
        "review" => WorkflowV2EvidenceKind::Review,
        "remediation" => WorkflowV2EvidenceKind::Remediation,
        "blocker" => WorkflowV2EvidenceKind::Blocker,
        "artifact" => WorkflowV2EvidenceKind::Artifact,
        _ => WorkflowV2EvidenceKind::Other,
    }
}

impl<'de> Deserialize<'de> for WorkflowV2EvidenceKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(evidence_kind_from_str(&raw))
    }
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

/// Outcome of a single recorded command.
///
/// Aliases only — deliberately *not* the tolerant coercion the kind enums use.
/// `invented_status_values_still_rejected` fixes that policy: a status we cannot
/// read must fail loudly rather than be normalised, because normalising is how a
/// schema slip becomes a false pass. What this enum lacked was the synonyms
/// agents actually reach for; it had none at all, so a plain `"success"` — the
/// word the sibling status enums already accept — destroyed the whole result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowV2CommandStatus {
    #[serde(
        alias = "success",
        alias = "ok",
        alias = "passed",
        alias = "pass",
        alias = "complete",
        alias = "completed",
        alias = "done"
    )]
    Succeeded,
    #[serde(alias = "failure", alias = "error", alias = "failed_closed")]
    Failed,
    #[serde(alias = "skip", alias = "not_run", alias = "notrun")]
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
    #[serde(
        // A task whose work failed is a task that is NOT covered, so `failed`
        // reads as Missing — the fail-closed direction. Mapping it to Partial
        // or Accepted would credit work that did not land.
        //
        // Agents reach for "failed" constantly because it is the word the
        // result envelope's own status field uses; only this narrower
        // coverage enum disagreed. Observed live: a dependency-graph reduce
        // emitted `task_coverage[].status = "failed"`, was rejected with
        // "unknown variant `failed`", and burned a whole repair iteration
        // without ever reaching a verdict on the graph it had correctly
        // built.
        alias = "failure",
        alias = "failed",
        alias = "error"
    )]
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

    /// `commands_run[].status` had no aliases at all, while both sibling status
    /// enums accept `success`. An agent using one word for all three fields
    /// therefore destroyed the result on the only field that had never been
    /// taught the synonym.
    #[test]
    fn command_status_accepts_the_synonyms_its_siblings_already_take() {
        for raw in ["\"success\"", "\"ok\"", "\"passed\"", "\"done\""] {
            let parsed: WorkflowV2CommandStatus = serde_json::from_str(raw)
                .unwrap_or_else(|error| panic!("{raw} must parse as succeeded: {error}"));
            assert_eq!(parsed, WorkflowV2CommandStatus::Succeeded, "{raw}");
        }
        let failed: WorkflowV2CommandStatus = serde_json::from_str("\"error\"").unwrap();
        assert_eq!(failed, WorkflowV2CommandStatus::Failed);
        let skipped: WorkflowV2CommandStatus = serde_json::from_str("\"skip\"").unwrap();
        assert_eq!(skipped, WorkflowV2CommandStatus::Skipped);
    }

    /// `build` is legal on `commands_run[].kind`, so agents reach for it on
    /// evidence too. It used to abort the entire `WorkflowV2Result` — one word
    /// cost a whole verification branch. Labelling is cosmetic, so it now lands
    /// on `Other` and the evidence survives.
    #[test]
    fn unknown_evidence_kind_degrades_to_other_instead_of_killing_the_result() {
        let kind: WorkflowV2EvidenceKind = serde_json::from_str("\"build\"")
            .expect("`build` must not abort the result — it is legal on command kind");
        assert_eq!(kind, WorkflowV2EvidenceKind::Other);

        let invented: WorkflowV2EvidenceKind = serde_json::from_str("\"whatever_the_model_said\"")
            .expect("an unknown evidence label must never abort the result");
        assert_eq!(invented, WorkflowV2EvidenceKind::Other);
    }

    /// The tolerance above must not swallow the labels that carry meaning —
    /// `blocker` in particular drives routing.
    #[test]
    fn known_evidence_kinds_still_parse_exactly() {
        for (raw, expected) in [
            ("\"blocker\"", WorkflowV2EvidenceKind::Blocker),
            ("\"test\"", WorkflowV2EvidenceKind::Test),
            ("\"artifact\"", WorkflowV2EvidenceKind::Artifact),
            ("\"inspect\"", WorkflowV2EvidenceKind::Inspection),
            ("\"remediation\"", WorkflowV2EvidenceKind::Remediation),
        ] {
            let parsed: WorkflowV2EvidenceKind = serde_json::from_str(raw).unwrap();
            assert_eq!(parsed, expected, "{raw}");
        }
    }

    /// A whole result carrying the exact shape that killed the live branch must
    /// now parse — the enum fix is only worth anything at this level.
    #[test]
    fn result_with_build_evidence_kind_parses_end_to_end() {
        let raw = serde_json::json!({
            "status": "needs_review",
            "summary": "verification ran",
            "evidence": [{"kind": "build", "summary": "cargo build succeeded"}],
            "commands_run": [{
                "kind": "build",
                "command": "cargo build",
                "status": "success",
                "exit_code": 0,
                "output_summary": "ok"
            }]
        });
        let parsed: WorkflowV2Result =
            serde_json::from_value(raw).expect("the shape that killed the live branch must parse");
        assert_eq!(parsed.evidence[0].kind, WorkflowV2EvidenceKind::Other);
        assert_eq!(parsed.commands_run[0].kind, WorkflowV2CommandKind::Build);
        assert_eq!(
            parsed.commands_run[0].status,
            WorkflowV2CommandStatus::Succeeded
        );
    }
}

#[cfg(test)]
mod coverage_failed_alias_tests {
    use super::*;

    /// `failed` is the word the result envelope's own status uses, so agents
    /// reach for it in task_coverage too. It must land on Missing — the
    /// fail-closed reading — never on Partial or Accepted.
    #[test]
    fn failed_task_coverage_status_reads_as_missing() {
        for raw in ["failed", "failure", "error"] {
            let value = serde_json::json!(raw);
            let status: WorkflowV2TaskCoverageStatus =
                serde_json::from_value(value).expect("alias parses");
            assert_eq!(
                status,
                WorkflowV2TaskCoverageStatus::Missing,
                "{raw} must be Missing, not a credit"
            );
        }
    }
}
