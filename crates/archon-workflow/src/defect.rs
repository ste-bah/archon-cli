//! Stable identities owned by deterministic validators, separate from diagnostics.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefectProvenance {
    HostValidator,
}

/// How far a candidate got through deterministic validation, in pipeline
/// order. Fixing every defect of one stage lets the next stage run, and that
/// stage may report more defects than the last one did: the author loop
/// measures progress per stage, so moving to a later stage is progress even
/// when the count rises (Issue 261). The fixed decomposition script mirrors
/// these names in `DEFECT_STAGES`; a test fails if they drift.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DefectStage {
    /// The reply or file could not be read or parsed at all. Also the stage of
    /// a record written before stages existed: it never claims a later one.
    #[default]
    Parse,
    /// It parsed, but not into the required shape (serde shape, headings).
    Shape,
    /// The shape is right, but identities, names or the set itself are not.
    Structure,
    /// Declared tool obligations.
    Tools,
    /// Dependency edges and the graph they form.
    Graph,
    /// Deliverable contracts, obligations and checks.
    Contracts,
    /// Agreement with frozen predecessors (pins, digests, frozen fields).
    Freeze,
}

impl DefectStage {
    pub const ALL: [DefectStage; 7] = [
        DefectStage::Parse,
        DefectStage::Shape,
        DefectStage::Structure,
        DefectStage::Tools,
        DefectStage::Graph,
        DefectStage::Contracts,
        DefectStage::Freeze,
    ];

    pub fn name(self) -> &'static str {
        match self {
            DefectStage::Parse => "parse",
            DefectStage::Shape => "shape",
            DefectStage::Structure => "structure",
            DefectStage::Tools => "tools",
            DefectStage::Graph => "graph",
            DefectStage::Contracts => "contracts",
            DefectStage::Freeze => "freeze",
        }
    }

    /// The stage a validator code belongs to. Semantic checks not named in
    /// [`STAGED_CODES`] (verifier strength, coverage, obligations) are
    /// contract checks.
    pub fn of(code: &str) -> Self {
        STAGED_CODES
            .iter()
            .find(|(known, _)| *known == code)
            .map_or(DefectStage::Contracts, |(_, stage)| *stage)
    }
}

/// Every validator code whose stage is not [`DefectStage::Contracts`]. The
/// fixed decomposition script mirrors this table (`STAGE_CODES`) to stage a
/// defect from an envelope written before defects carried their stage.
pub const STAGED_CODES: &[(&str, DefectStage)] = &[
    ("invalid_json", DefectStage::Parse),
    ("unreadable_task_file", DefectStage::Parse),
    ("unparseable_task_file", DefectStage::Parse),
    ("invalid_task_spec", DefectStage::Parse),
    ("unreadable_task_spec", DefectStage::Parse),
    ("unreadable_task_directory", DefectStage::Parse),
    ("invalid_candidate_shape", DefectStage::Shape),
    ("unbound_candidate", DefectStage::Shape),
    ("invalid_schema_version", DefectStage::Shape),
    ("invalid_floor_serialization", DefectStage::Shape),
    ("redacted_executable_value", DefectStage::Shape),
    ("section_heading", DefectStage::Shape),
    ("invalid_declared_status", DefectStage::Shape),
    ("scope_declaration", DefectStage::Shape),
    ("missing_runnable_test", DefectStage::Shape),
    ("invalid_task_id", DefectStage::Structure),
    ("duplicate_task_id", DefectStage::Structure),
    ("invalid_filename", DefectStage::Structure),
    ("duplicate_filename", DefectStage::Structure),
    ("empty_task_set", DefectStage::Structure),
    ("empty_acceptance", DefectStage::Structure),
    ("duplicate_acceptance_id", DefectStage::Structure),
    ("duplicate_supplementary_id", DefectStage::Structure),
    ("invalid_supplementary_id", DefectStage::Structure),
    ("unknown_acceptance_id", DefectStage::Structure),
    ("empty_criterion", DefectStage::Structure),
    ("empty_judgment_field", DefectStage::Structure),
    ("task_file_without_directory", DefectStage::Structure),
    ("candidate_refused", DefectStage::Structure),
    ("tool_obligation", DefectStage::Tools),
    ("duplicate_dependency", DefectStage::Graph),
    ("invalid_edge_declaration", DefectStage::Graph),
    ("empty_consumed_path", DefectStage::Graph),
    ("missing_dependency", DefectStage::Graph),
    ("missing_blocked_task", DefectStage::Graph),
    ("self_block", DefectStage::Graph),
    ("contradictory_edge", DefectStage::Graph),
    ("mutual_blocks", DefectStage::Graph),
    ("dependency_cycle", DefectStage::Graph),
    ("missing_producer", DefectStage::Graph),
    ("missing_consumer_declaration", DefectStage::Graph),
    ("missing_producer_deliverable", DefectStage::Graph),
    ("missing_record_binding", DefectStage::Graph),
    ("missing_producer_record_binding", DefectStage::Graph),
    ("record_binding_mismatch", DefectStage::Graph),
    ("kind_mismatch", DefectStage::Graph),
    ("missing_data_obligation", DefectStage::Graph),
    ("acceptance_digest_mismatch", DefectStage::Freeze),
    ("frozen_field_changed", DefectStage::Freeze),
    ("missing_frozen_task", DefectStage::Freeze),
    ("extra_task", DefectStage::Freeze),
    ("unreadable_acceptance_pin", DefectStage::Freeze),
    ("prd_identity_mismatch", DefectStage::Freeze),
    ("task_absent_from_skeleton", DefectStage::Freeze),
    ("partial_skeleton_freeze", DefectStage::Freeze),
    ("predecessor_findings", DefectStage::Freeze),
    ("invalid_acceptance_bundle", DefectStage::Freeze),
    ("invalid_skeleton_chain", DefectStage::Freeze),
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeterministicDefect {
    pub provenance: DefectProvenance,
    pub code: String,
    /// A valid task/check identity, or a structural slot when the id is rejected.
    pub subject: String,
    /// Validator-owned field/slot, never a submitted filename, path or value.
    pub location: String,
    /// The validation stage that reported it; see [`DefectStage`].
    #[serde(default)]
    pub stage: DefectStage,
}

impl DeterministicDefect {
    pub fn new(code: &str, subject: impl Into<String>, location: impl Into<String>) -> Self {
        Self {
            provenance: DefectProvenance::HostValidator,
            code: code.into(),
            subject: subject.into(),
            location: location.into(),
            stage: DefectStage::of(code),
        }
    }

    /// The same defect reported at `stage`: a predecessor's own defect seen
    /// from a later validator is that validator's freeze-stage defect.
    pub fn at_stage(mut self, stage: DefectStage) -> Self {
        self.stage = stage;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationDefect {
    pub identity: DeterministicDefect,
    pub message: String,
}

impl ValidationDefect {
    pub fn new(code: &str, subject: &str, location: &str, message: impl Into<String>) -> Self {
        Self {
            identity: DeterministicDefect::new(code, subject, location),
            message: message.into(),
        }
    }
}

pub fn defect_message(defects: &[ValidationDefect]) -> String {
    defects
        .iter()
        .map(|defect| defect.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_defect_carries_the_stage_of_its_code_on_the_wire() {
        let shape = DeterministicDefect::new("invalid_candidate_shape", "skeleton", "candidate");
        assert_eq!(shape.stage, DefectStage::Shape);
        let value = serde_json::to_value(&shape).expect("json");
        assert_eq!(value["stage"], "shape");
        let mut legacy = value;
        legacy.as_object_mut().expect("object").remove("stage");
        let legacy: DeterministicDefect = serde_json::from_value(legacy).expect("legacy");
        assert_eq!(legacy.stage, DefectStage::Parse);
        assert!(
            DefectStage::Parse < DefectStage::Shape && DefectStage::Graph < DefectStage::Freeze
        );
        for stage in DefectStage::ALL {
            assert_eq!(serde_json::to_value(stage).expect("json"), stage.name());
        }
        assert_eq!(DefectStage::of("invalid_json"), DefectStage::Parse);
        assert_eq!(DefectStage::of("invalid_filename"), DefectStage::Structure);
        assert_eq!(DefectStage::of("dependency_cycle"), DefectStage::Graph);
        assert_eq!(DefectStage::of("invalid_verifier"), DefectStage::Contracts);
    }
}
