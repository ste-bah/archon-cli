//! Portable, exact-name contracts that travel with a decomposed task set.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::WorkflowError;
use crate::task_universe::WorkflowV2DeliverableContract;
use crate::verifier_strength::verifier_strength_defect;

#[path = "task_set_contract_policy.rs"]
mod policy;
pub use policy::{
    AcceptancePolicyFinding, CHECK_SHAPE_VOCABULARY, acceptance_policy_findings,
    criterion_prescribes_check_shape,
};
pub const ACCEPTANCE_CONTRACT_FILE: &str = "acceptance-contract.json";
pub const ACCEPTANCE_LOCK_FILE: &str = "acceptance-contract.lock";
pub const TASK_SKELETON_FILE: &str = "task-skeleton.json";
pub const TASK_SKELETON_LOCK_FILE: &str = "task-skeleton.lock";
pub const RESIDUAL_GAPS_FILE: &str = "acceptance-residual-gaps.json";
pub const REQUIRED_RESIDUAL_GAP_FIELDS: [&str; 9] = [
    "id",
    "acceptance_id",
    "area",
    "description",
    "impact",
    "fail_closed_behavior",
    "owner",
    "created_at",
    "fail_closed_check",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceContract {
    pub schema_version: u32,
    pub prd: PrdIdentity,
    pub gap_policy: GapPolicy,
    pub acceptance: Vec<AcceptanceCriterion>,
    #[serde(default)]
    pub supplementary: Vec<AcceptanceCriterion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrdIdentity {
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapPolicy {
    #[serde(default)]
    pub permitted_acceptance_ids: BTreeSet<String>,
    #[serde(default)]
    pub forbidden_phrases: Vec<String>,
    #[serde(default)]
    pub required_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub criterion: String,
    pub check: AcceptanceCheck,
    #[serde(default)]
    pub gap_permitted: bool,
    pub judgment: JudgeVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AcceptanceCheck {
    Command {
        command: String,
        cwd: TrustedCwd,
    },
    Floor {
        contract: WorkflowV2DeliverableContract,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustedCwd {
    ProjectRoot,
    RepoRoot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgeVerdict {
    pub verdict: JudgeDecision,
    pub counterexample: String,
    pub reason: String,
    pub host_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgeDecision {
    Accepted,
    Refuted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreezeGateMode {
    Observe,
    Enforce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreezeGateStamp {
    pub mode: FreezeGateMode,
    pub finding_count: usize,
    pub findings_digest: String,
    pub binary_commit: String,
    pub evaluated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceLock {
    pub algorithm: String,
    pub digest: String,
    pub gate: FreezeGateStamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptancePin {
    pub task_root: String,
    pub acceptance_digest: String,
    pub freeze_event_id: String,
    pub acceptance_gate: FreezeGateStamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_gate: Option<FreezeGateStamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fidelity_waivers: Vec<crate::fidelity_audit::ObligationWaiver>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResidualGapRecord {
    pub id: String,
    pub acceptance_id: String,
    pub area: String,
    pub description: String,
    pub impact: String,
    pub fail_closed_behavior: String,
    pub owner: String,
    pub created_at: String,
    pub fail_closed_check: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSetContractError {
    pub message: String,
}

impl fmt::Display for TaskSetContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TaskSetContractError {}

type ContractResult<T> = Result<T, TaskSetContractError>;

pub fn validate_acceptance_contract(
    contract: &AcceptanceContract,
    expected_acceptance_ids: &BTreeSet<String>,
    require_judgments: bool,
) -> ContractResult<()> {
    validate_acceptance_structure(contract, expected_acceptance_ids, require_judgments)?;
    if let Some(finding) = acceptance_policy_findings(contract).into_iter().next() {
        return invalid(finding.message);
    }
    Ok(())
}

pub fn validate_acceptance_structure(
    contract: &AcceptanceContract,
    expected_acceptance_ids: &BTreeSet<String>,
    require_judgments: bool,
) -> ContractResult<()> {
    if contract.schema_version != 1 {
        return invalid(format!(
            "acceptance contract schema_version must be 1, found {}",
            contract.schema_version
        ));
    }
    if contract.acceptance.is_empty() {
        return invalid("acceptance contract examined zero acceptance checks; this is not a pass");
    }
    let unknown_permissions: Vec<_> = contract
        .gap_policy
        .permitted_acceptance_ids
        .difference(expected_acceptance_ids)
        .cloned()
        .collect();
    if !unknown_permissions.is_empty() {
        return invalid(format!(
            "gap_policy.permitted_acceptance_ids contains unknown ids {}; remove each unknown id or correct it to one defined by the PRD",
            unknown_permissions.join(", ")
        ));
    }
    let mut seen = BTreeSet::new();
    for criterion in &contract.acceptance {
        if !expected_acceptance_ids.contains(&criterion.id) {
            return invalid(format!(
                "acceptance id '{}' is not defined by the PRD; remove it or correct the id",
                criterion.id
            ));
        }
        if !seen.insert(criterion.id.clone()) {
            return invalid(format!(
                "acceptance id '{}' appears more than once; keep exactly one check for this id",
                criterion.id
            ));
        }
        validate_criterion_structure(criterion, require_judgments)?;
        if criterion.gap_permitted
            != contract
                .gap_policy
                .permitted_acceptance_ids
                .contains(&criterion.id)
        {
            return invalid(format!(
                "acceptance id '{}' gap_permitted disagrees with gap_policy; make both declarations match",
                criterion.id
            ));
        }
    }
    let missing: Vec<_> = expected_acceptance_ids.difference(&seen).cloned().collect();
    if !missing.is_empty() {
        return invalid(format!(
            "acceptance contract is missing checks for {}; keep every check already present and add one check for each id listed, so every PRD acceptance id has its own check",
            missing.join(", ")
        ));
    }
    let mut supplementary = BTreeSet::new();
    for criterion in &contract.supplementary {
        if !criterion.id.starts_with("SUP-") {
            return invalid(format!(
                "supplementary check '{}' must use a distinct SUP-* id",
                criterion.id
            ));
        }
        if expected_acceptance_ids.contains(&criterion.id)
            || !supplementary.insert(criterion.id.clone())
        {
            return invalid(format!(
                "supplementary check '{}' collides with an acceptance or supplementary id; choose a distinct SUP-* id",
                criterion.id
            ));
        }
        if criterion.gap_permitted {
            return invalid(format!(
                "supplementary check '{}' cannot be covered by a residual gap; set gap_permitted to false",
                criterion.id
            ));
        }
        validate_criterion_structure(criterion, require_judgments)?;
    }
    Ok(())
}

fn validate_criterion_structure(
    criterion: &AcceptanceCriterion,
    require_judgments: bool,
) -> ContractResult<()> {
    if criterion.criterion.trim().is_empty() {
        return invalid(format!(
            "check '{}' has empty criterion text; copy the exact criterion text",
            criterion.id
        ));
    }
    if let AcceptanceCheck::Floor { contract } = &criterion.check {
        serde_json::to_value(contract).map_err(|error| contract_error(error.to_string()))?;
    }
    if require_judgments {
        for (field, value) in [
            ("counterexample", criterion.judgment.counterexample.as_str()),
            ("reason", criterion.judgment.reason.as_str()),
            ("host_call_id", criterion.judgment.host_call_id.as_str()),
        ] {
            if value.trim().is_empty() {
                return invalid(format!(
                    "check '{}' judgment field '{field}' is empty; re-run freeze-acceptance so the host judge records it",
                    criterion.id
                ));
            }
        }
    }
    Ok(())
}

pub fn validate_acceptance_bundle(
    tasks_root: &Path,
    expected_pin: Option<&AcceptancePin>,
    expected_acceptance_ids: &BTreeSet<String>,
) -> ContractResult<AcceptanceContract> {
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let lock_path = tasks_root.join(ACCEPTANCE_LOCK_FILE);
    let bytes = read_required(&contract_path)?;
    let lock: AcceptanceLock = read_json(&lock_path)?;
    if lock.algorithm != "blake3" {
        return invalid(format!(
            "{} algorithm must be 'blake3'; re-run workflow freeze-acceptance",
            ACCEPTANCE_LOCK_FILE
        ));
    }
    validate_gate_stamp(&lock.gate, ACCEPTANCE_LOCK_FILE)?;
    let actual = blake3::hash(&bytes).to_hex().to_string();
    if actual != lock.digest {
        return invalid(format!(
            "acceptance contract digest mismatch: lock expected {}, actual {}; restore the frozen version or re-run `workflow freeze-acceptance`",
            lock.digest, actual
        ));
    }
    if let Some(pin) = expected_pin {
        let canonical_root = tasks_root.canonicalize().map_err(|error| {
            contract_error(format!(
                "task_root {} could not be canonicalized: {error}; restore it or re-run `workflow freeze-acceptance`",
                tasks_root.display()
            ))
        })?;
        let pinned_root = PathBuf::from(&pin.task_root);
        let canonical_pin = pinned_root.canonicalize().unwrap_or(pinned_root);
        if canonical_pin != canonical_root {
            return invalid(format!(
                "acceptance freeze event '{}' binds task_root {}, but validation is reading {}; restore the frozen task directory or re-run `workflow freeze-acceptance`",
                pin.freeze_event_id,
                canonical_pin.display(),
                canonical_root.display()
            ));
        }
        if pin.acceptance_digest != actual {
            return invalid(format!(
                "acceptance contract differs from freeze event '{}': expected digest {}, actual {}; restore the frozen version or re-run `workflow freeze-acceptance`",
                pin.freeze_event_id, pin.acceptance_digest, actual
            ));
        }
        validate_gate_stamp(&pin.acceptance_gate, "acceptance pin")?;
        if pin.acceptance_gate != lock.gate {
            return invalid(format!(
                "acceptance lock/pin gate provenance mismatch; re-run `workflow freeze-acceptance` with the current binary"
            ));
        }
    }
    let contract: AcceptanceContract = serde_json::from_slice(&bytes)
        .map_err(|error| contract_error(format!("{}: {error}", contract_path.display())))?;
    validate_acceptance_structure(&contract, expected_acceptance_ids, true)?;
    Ok(contract)
}

pub fn validate_residual_gaps(
    policy: &GapPolicy,
    gaps: &[ResidualGapRecord],
) -> ContractResult<()> {
    let mut seen_ids = BTreeSet::new();
    let mut seen_acceptance = BTreeSet::new();
    for gap in gaps {
        if !seen_ids.insert(gap.id.clone()) {
            return invalid(format!(
                "residual gap id '{}' is duplicated; keep one record per id",
                gap.id
            ));
        }
        if !seen_acceptance.insert(gap.acceptance_id.clone()) {
            return invalid(format!(
                "acceptance id '{}' has more than one residual gap; keep exactly one bound record",
                gap.acceptance_id
            ));
        }
        if !policy.permitted_acceptance_ids.contains(&gap.acceptance_id) {
            return invalid(format!(
                "residual gap '{}' binds non-permitted acceptance id '{}'; remove it or bind it to an id permitted by the frozen gap policy",
                gap.id, gap.acceptance_id
            ));
        }
        let fields = residual_gap_fields(gap);
        for required in &policy.required_fields {
            if fields
                .get(required.as_str())
                .is_none_or(|value| value.trim().is_empty())
            {
                return invalid(format!(
                    "residual gap '{}' field '{}' is missing or empty; write the concrete value required by the frozen gap policy",
                    gap.id, required
                ));
            }
        }
        for (field, value) in &fields {
            let lower = value.to_ascii_lowercase();
            for phrase in &policy.forbidden_phrases {
                if lower.contains(&phrase.to_ascii_lowercase()) {
                    return invalid(format!(
                        "residual gap '{}' field '{}' contains forbidden phrase '{}'; rewrite the field concretely",
                        gap.id, field, phrase
                    ));
                }
            }
        }
        if let Some(defect) = verifier_strength_defect(Some(&gap.fail_closed_check), None, None) {
            return invalid(format!(
                "residual gap '{}' fail_closed_check: {defect}",
                gap.id
            ));
        }
    }
    Ok(())
}

fn residual_gap_fields(gap: &ResidualGapRecord) -> BTreeMap<&'static str, &str> {
    BTreeMap::from([
        ("id", gap.id.as_str()),
        ("acceptance_id", gap.acceptance_id.as_str()),
        ("area", gap.area.as_str()),
        ("description", gap.description.as_str()),
        ("impact", gap.impact.as_str()),
        ("fail_closed_behavior", gap.fail_closed_behavior.as_str()),
        ("owner", gap.owner.as_str()),
        ("created_at", gap.created_at.as_str()),
        ("fail_closed_check", gap.fail_closed_check.as_str()),
    ])
}

pub fn content_digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn empty_gate_findings_digest() -> String {
    content_digest(b"[]")
}

fn read_required(path: &Path) -> ContractResult<Vec<u8>> {
    std::fs::read(path).map_err(|error| contract_error(format!("required task-set artifact {} could not be read: {error}; restore it or run the named freeze command", path.display())))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> ContractResult<T> {
    let bytes = read_required(path)?;
    serde_json::from_slice(&bytes).map_err(|error| {
        contract_error(format!(
            "{} is malformed JSON: {error}; repair it or re-run the named freeze command",
            path.display()
        ))
    })
}

pub fn validate_gate_stamp(stamp: &FreezeGateStamp, label: &str) -> ContractResult<()> {
    if stamp.findings_digest.trim().is_empty()
        || stamp.binary_commit.trim().is_empty()
        || chrono::DateTime::parse_from_rfc3339(&stamp.evaluated_at).is_err()
    {
        return invalid(format!(
            "{label} gate provenance is incomplete; re-freeze with the current binary"
        ));
    }
    let empty_digest = empty_gate_findings_digest();
    if stamp.finding_count == 0 && stamp.findings_digest != empty_digest {
        return invalid(format!(
            "{label} gate provenance claims zero findings but does not carry the canonical empty-findings digest; re-freeze with the current binary"
        ));
    }
    if stamp.finding_count > 0 && stamp.findings_digest == empty_digest {
        return invalid(format!(
            "{label} gate provenance claims {} finding(s) but carries the empty-findings digest; re-freeze with the current binary",
            stamp.finding_count
        ));
    }
    if stamp.mode == FreezeGateMode::Enforce && stamp.finding_count > 0 {
        return invalid(format!(
            "{label} gate provenance claims enforce mode with {} finding(s), a state enforcement cannot publish; re-freeze with the current binary",
            stamp.finding_count
        ));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> ContractResult<T> {
    Err(contract_error(message))
}

fn contract_error(message: impl Into<String>) -> TaskSetContractError {
    TaskSetContractError {
        message: message.into(),
    }
}

impl From<TaskSetContractError> for WorkflowError {
    fn from(error: TaskSetContractError) -> Self {
        WorkflowError::SpecInvalid(error.message)
    }
}
