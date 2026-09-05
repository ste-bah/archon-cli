//! Strict deliverable declarations shared by tasks and acceptance floors.
use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowV2DeliverableContract {
    pub kind: String,
    pub artifact_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed_verifier_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_source_records_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_artifact_field: Option<String>,
    #[serde(default)]
    pub min_instances: usize,
    #[serde(default)]
    pub required_universe: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub universe_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cell_identity_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_true_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_nonempty_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positive_count_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub minimum_count_fields: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gaps_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_records_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry_key_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry_required_true_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_status_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry_allowed_statuses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_count_field: Option<String>,
    #[serde(default)]
    pub registry_minimum_count: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub registry_identity_fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_format: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub non_constant_fields: Vec<String>,
    /// Format of the declared deliverable: `json` (default) or `text`.
    ///
    /// A contract may legitimately declare a prose or tabular artifact — an
    /// inventory, a written report. Parsing one as JSON fails on line 1 however
    /// good the work is, and a fail-closed gate then demotes it permanently
    /// because no remediation can make markdown parse. A textual deliverable is
    /// checked for existence and non-emptiness, which is all a contract can
    /// honestly assert about unstructured content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_format: Option<String>,
    /// Payload field carrying each record's observation instant. Declared, the
    /// verifier rejects records dated after the verification time: an observed
    /// series has no future records, and fabricated ones routinely overshoot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_time_field: Option<String>,
    /// Weekday indices (Mon=0 … Sun=6) on which the observed venue does not
    /// trade, and specific non-trading dates. When either is declared the
    /// verifier rejects records dated to a closed session. This is external
    /// truth rather than a threshold: an evenly spaced generated series lands on
    /// closed days by construction, and there is no number to fabricate toward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_weekdays: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closed_dates: Vec<String>,
    /// Overrides for the synthetic-series step-variety check, the minimum
    /// percentage of first differences that must be distinct. Left unset in task
    /// specs by design — a declared numeric threshold is a target to fabricate
    /// against, so the defaults live in the engine where the agent cannot read
    /// them. Integer percent rather than a float ratio because this struct
    /// derives Ord and f64 does not implement it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_variety_min_rows: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_variety_min_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series_value_fields: Vec<String>,
    #[serde(default)]
    pub series_overlap_min_rows: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_count_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub response_identity_fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_status_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_checks_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_check_status_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_failed_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_passed_values: Vec<String>,
}
