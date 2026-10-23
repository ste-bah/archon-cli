use super::acceptance;

pub(super) const STATE_PATH: &str = "decomposition/state.json";
pub(super) const REPORT_PATH: &str = "decomposition/reverify/reports";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ReverifyReport {
    pub(super) schema_version: u32,
    pub(super) id: String,
    pub(super) supersedes_id: String,
    pub(super) supersedes_digest: String,
    pub(super) outcome: String,
    #[serde(default)]
    pub(super) report_event_seq: u64,
    #[serde(default)]
    pub(super) call_record_ids_at_report: Vec<String>,
    pub(super) calls: Vec<String>,
    pub(super) subjects: Vec<SubjectResult>,
    pub(super) set_gates: Vec<GateResult>,
    pub(super) frozen_gates: Vec<GateResult>,
    pub(super) checks: Vec<acceptance::CheckResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) contract_digest: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(super) judged_check_digests: std::collections::BTreeMap<String, String>,
    /// The entry identity `judged_check_digests` uses; absent on a report a
    /// previous build wrote, whose digests cover the whole entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) judged_check_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) operational_failure_cause: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SubjectResult {
    pub(super) task_id: String,
    pub(super) file_name: String,
    pub(super) disk_digest: String,
    pub(super) judged_digest: String,
    pub(super) logic_version: Option<u32>,
    pub(super) logic_digest: Option<String>,
    pub(super) build: Option<String>,
    pub(super) verdict: String,
    pub(super) findings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GateResult {
    pub(crate) command_id: String,
    pub(crate) call_id: String,
    pub(crate) logic_version: Option<u32>,
    pub(crate) logic_digest: Option<String>,
    pub(crate) build: Option<String>,
    pub(crate) verdict: String,
    pub(crate) findings: Vec<String>,
    pub(crate) subjects: Vec<String>,
}
