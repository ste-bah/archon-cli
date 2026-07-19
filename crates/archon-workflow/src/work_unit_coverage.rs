use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use crate::spec::StageSpec;

mod evidence_parse;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CoverageVerdict {
    Accepted,
    Incomplete,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct EvidenceItem {
    pub kind: String,
    pub role: Option<String>,
    pub path: Option<String>,
    pub artifact_path: Option<String>,
    pub command: Option<String>,
    pub exit_status: Option<i32>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct EvidenceBundle {
    pub work_unit_ids: Vec<String>,
    pub status: String,
    pub evidence: Vec<EvidenceItem>,
    pub residual_gaps: Vec<String>,
    pub source_item_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct WorkUnitCoverage {
    pub schema: String,
    pub run_id: String,
    pub stage_id: String,
    pub attempt_id: String,
    pub input_hash: String,
    pub required_work_units: Vec<String>,
    pub satisfied_work_units: Vec<String>,
    pub blocked_work_units: Vec<String>,
    pub missing_work_units: Vec<String>,
    pub evidence_bundles: Vec<EvidenceBundle>,
    pub verdict: CoverageVerdict,
}

pub(crate) fn stage_required_units(stage: &StageSpec) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for key in [
        "completion_task_ids",
        "required_work_units",
        "work_unit_ids",
        "task_ids",
        "canonical_task_ids",
    ] {
        collect_strings(stage.extra.get(key), &mut out);
        collect_strings(stage.input.get(key), &mut out);
    }
    for key in ["work_unit_id", "task_id", "canonical_task_id"] {
        collect_string(stage.extra.get(key), &mut out);
        collect_string(stage.input.get(key), &mut out);
    }
    out
}

pub(crate) fn item_required_units(payload: &Value) -> Vec<String> {
    let mut out = BTreeSet::new();
    for key in [
        "work_unit_ids",
        "task_ids",
        "canonical_task_ids",
        "implemented_work_unit_ids",
        "implemented_task_ids",
        "implemented_canonical_task_ids",
        "completed_work_unit_ids",
        "completed_task_ids",
        "completed_canonical_task_ids",
    ] {
        collect_strings(payload.get(key), &mut out);
    }
    for key in [
        "work_unit_id",
        "task_id",
        "canonical_task_id",
        "implemented_work_unit_id",
        "implemented_task_id",
        "implemented_canonical_task_id",
        "completed_work_unit_id",
        "completed_task_id",
        "completed_canonical_task_id",
    ] {
        collect_string(payload.get(key), &mut out);
    }
    out.into_iter().collect()
}

pub(crate) fn evaluate(
    run_id: &str,
    stage_id: &str,
    attempt_id: &str,
    required: BTreeSet<String>,
    mut bundles: Vec<EvidenceBundle>,
) -> WorkUnitCoverage {
    for bundle in &mut bundles {
        bundle.work_unit_ids.sort();
        bundle.work_unit_ids.dedup();
    }
    let mut satisfied = BTreeSet::new();
    let mut blocked = BTreeSet::new();
    let mut failed = false;
    for unit in &required {
        for bundle in bundles.iter().filter(|b| b.work_unit_ids.contains(unit)) {
            if bundle_failed(bundle) {
                failed = true;
                continue;
            }
            if bundle_blocked(bundle) {
                if blocked_bundle_has_attempts(bundle) {
                    blocked.insert(unit.clone());
                }
                continue;
            }
            if bundle_satisfies_unit(bundle) {
                satisfied.insert(unit.clone());
            }
        }
    }
    let missing = required
        .difference(&satisfied)
        .filter(|unit| !blocked.contains(*unit))
        .cloned()
        .collect::<BTreeSet<_>>();
    let verdict = if failed {
        CoverageVerdict::Failed
    } else if !missing.is_empty() {
        CoverageVerdict::Incomplete
    } else if !blocked.is_empty() {
        CoverageVerdict::Blocked
    } else {
        CoverageVerdict::Accepted
    };
    let record = json!({
        "required": required,
        "bundles": bundles,
    });
    WorkUnitCoverage {
        schema: "archon.workflow.work_unit_coverage.v1".into(),
        run_id: run_id.into(),
        stage_id: stage_id.into(),
        attempt_id: attempt_id.into(),
        input_hash: blake3::hash(&serde_json::to_vec(&record).unwrap_or_default())
            .to_hex()
            .to_string(),
        required_work_units: required.into_iter().collect(),
        satisfied_work_units: satisfied.into_iter().collect(),
        blocked_work_units: blocked.into_iter().collect(),
        missing_work_units: missing.into_iter().collect(),
        evidence_bundles: bundles,
        verdict,
    }
}

pub(crate) fn bundles_from_agent_records(
    run_dir: &Path,
    stage_id: &str,
    item_ids: impl IntoIterator<Item = String>,
) -> BTreeMap<String, Vec<EvidenceBundle>> {
    item_ids
        .into_iter()
        .map(|item_id| {
            let path = run_dir
                .join("agent-outputs")
                .join(stage_id)
                .join(format!("{item_id}.json"));
            let bundles = std::fs::read_to_string(path)
                .ok()
                .and_then(|body| serde_json::from_str::<Value>(&body).ok())
                .map(|value| evidence_parse::value_to_bundles(&value, Some(item_id.clone())))
                .unwrap_or_default();
            (item_id, bundles)
        })
        .collect()
}

fn bundle_satisfies_unit(bundle: &EvidenceBundle) -> bool {
    non_blocking_status(&bundle.status)
        && bundle.residual_gaps.is_empty()
        && has_file_or_artifact(&bundle.evidence)
        && has_verification_evidence(&bundle.evidence)
}

fn bundle_failed(bundle: &EvidenceBundle) -> bool {
    let status = normalized(&bundle.status);
    status.contains("failed")
        || status.contains("rejected")
        || status.contains("unverifiable")
        || status.contains("notverified")
}

fn bundle_blocked(bundle: &EvidenceBundle) -> bool {
    normalized(&bundle.status).contains("blocked")
}

fn non_blocking_status(status: &str) -> bool {
    let status = normalized(status);
    !(status.contains("blocked")
        || status.contains("failed")
        || status.contains("rejected")
        || status.contains("unverifiable")
        || status.contains("notverified"))
}

fn blocked_bundle_has_attempts(bundle: &EvidenceBundle) -> bool {
    bundle.evidence.iter().any(|item| {
        item.command
            .as_deref()
            .is_some_and(|cmd| !cmd.trim().is_empty())
            || item
                .summary
                .as_deref()
                .is_some_and(|s| s.contains("attempt"))
    })
}

fn has_file_or_artifact(items: &[EvidenceItem]) -> bool {
    items.iter().any(|item| {
        item.path.as_deref().is_some_and(|v| !v.trim().is_empty())
            || item
                .artifact_path
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
    })
}

fn has_verification_evidence(items: &[EvidenceItem]) -> bool {
    items.iter().any(|item| {
        let kind = normalized(&item.kind);
        if kind == "command" || kind == "test" {
            return item
                .command
                .as_deref()
                .is_some_and(|cmd| !command_is_list_only(cmd))
                && item.exit_status.unwrap_or(0) == 0
                && item.role.as_deref().map(normalized) != Some("discovery".into());
        }
        matches!(
            kind.as_str(),
            "review" | "schema" | "render" | "manualgate" | "manual_gate"
        ) && item
            .summary
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
    })
}

fn command_is_list_only(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    ["--list", "-list", "list-tests", "test list", "collect-only"]
        .iter()
        .any(|needle| command.contains(needle))
}

fn collect_strings(value: Option<&Value>, out: &mut BTreeSet<String>) {
    match value {
        Some(Value::Array(values)) => {
            for value in values {
                collect_string(Some(value), out);
            }
        }
        Some(Value::String(text)) => {
            for part in text.split(',') {
                let part = part.trim();
                if !part.is_empty() {
                    out.insert(part.to_string());
                }
            }
        }
        _ => {}
    }
}

fn collect_string(value: Option<&Value>, out: &mut BTreeSet<String>) {
    if let Some(text) = value.and_then(Value::as_str).map(str::trim)
        && !text.is_empty()
    {
        out.insert(text.to_string());
    }
}

fn normalized(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect()
}
