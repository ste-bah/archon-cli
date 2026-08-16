use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::guardrail::{
    GuardrailFinalStatus, VerificationStatus, WorldGuardedAction, WorldGuardrailOutcome,
};
use crate::schema::{WorldLabelSet, WorldTraceRow};

pub const MATERIALIZED_LABELS_LEDGER: &str = "world-materialized-labels.jsonl";
/// Bumped to 2 by #184 M9, which added `merge_conflict`, `claim_overlap` and
/// `isolated` to [`WorldLabelSet`].
///
/// The materialization key is `(action_attempt_id, label_definition_version)`.
/// Leaving it at 1 would have rows labelled under the old definition and rows
/// labelled under the new one sharing a key, so the second silently overwrites
/// the first and the corpus mixes two definitions with no way to tell them
/// apart afterwards.
pub const LABEL_DEFINITION_VERSION: u32 = 2;

pub fn with_materialization_lock<T>(
    root: &Path,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let lock_path = root.join("ledgers").join("world-materialized-labels.lock");
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock
        .try_write()
        .map_err(|error| anyhow::anyhow!("materialized label lock unavailable: {error}"))?;
    operation()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterializedLabelRecord {
    pub action_attempt_id: String,
    pub trace_row_id: String,
    pub prediction_id: Option<String>,
    pub outcome_id: String,
    pub verification_keys: Vec<String>,
    pub label_definition_version: u32,
    pub labels: WorldLabelSet,
}

impl MaterializedLabelRecord {
    fn materialization_key(&self) -> String {
        format!(
            "{}:{}",
            self.action_attempt_id, self.label_definition_version
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelContradiction {
    pub action_attempt_id: String,
    pub label: String,
    pub heuristic_value: bool,
    pub classified_value: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MaterializedLabels {
    pub records: Vec<MaterializedLabelRecord>,
    pub contradictions: Vec<LabelContradiction>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BinarySuccessLabels {
    pub labels: Vec<bool>,
    pub known: usize,
    pub unknown: usize,
    pub total: usize,
}

pub fn materialize_verified_labels(
    rows: &[WorldTraceRow],
    actions: &[WorldGuardedAction],
    outcomes: &[WorldGuardrailOutcome],
    verifications: &[crate::guardrail::VerificationOutcome],
    prediction_attempts: &BTreeMap<String, String>,
) -> Result<MaterializedLabels> {
    let actions = unique_actions(actions)?;
    let outcomes = unique_outcomes(outcomes)?;
    let verifications = unique_verifications(verifications)?;
    let mut records = Vec::new();
    let mut contradictions = Vec::new();
    let mut seen_attempts = BTreeSet::new();

    for row in rows {
        let Some(action_attempt_id) = row.action_attempt_id.as_deref() else {
            continue;
        };
        if !seen_attempts.insert(action_attempt_id.to_string()) {
            bail!("duplicate trace action attempt identity: {action_attempt_id}");
        }
        let action = actions
            .get(action_attempt_id)
            .ok_or_else(|| anyhow::anyhow!("missing guarded action: {action_attempt_id}"))?;
        let outcome = outcomes
            .get(action_attempt_id)
            .ok_or_else(|| anyhow::anyhow!("missing guardrail outcome: {action_attempt_id}"))?;
        validate_prediction_reference(outcome, prediction_attempts)?;
        let joined_verifications = verifications
            .values()
            .filter(|verification| verification.action_id == action_attempt_id)
            .copied()
            .collect::<Vec<_>>();
        validate_embedded_verifications(outcome, &verifications, action_attempt_id)?;
        let labels = structured_labels(
            row,
            action,
            outcome,
            &joined_verifications,
            &mut contradictions,
        );
        records.push(MaterializedLabelRecord {
            action_attempt_id: action_attempt_id.to_string(),
            trace_row_id: row.row_id.clone(),
            prediction_id: outcome.prediction_id.clone(),
            outcome_id: outcome.outcome_id.clone(),
            verification_keys: joined_verifications
                .iter()
                .map(|verification| verification.idempotency_key.clone())
                .collect(),
            label_definition_version: LABEL_DEFINITION_VERSION,
            labels,
        });
    }

    records.sort_by(|left, right| left.action_attempt_id.cmp(&right.action_attempt_id));
    contradictions.sort_by(|left, right| left.action_attempt_id.cmp(&right.action_attempt_id));
    Ok(MaterializedLabels {
        records,
        contradictions,
    })
}

pub fn binary_success_labels(records: &[MaterializedLabelRecord]) -> BinarySuccessLabels {
    let labels = records
        .iter()
        .filter_map(|record| record.labels.success)
        .collect::<Vec<_>>();
    BinarySuccessLabels {
        known: labels.len(),
        unknown: records.len().saturating_sub(labels.len()),
        total: records.len(),
        labels,
    }
}

pub fn append_materialized_labels_locked(
    root: &Path,
    records: &[MaterializedLabelRecord],
) -> Result<PathBuf> {
    let path = root.join("ledgers").join(MATERIALIZED_LABELS_LEDGER);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut existing = BTreeMap::new();
    for record in load_materialized_labels(root)? {
        let key = record.materialization_key();
        if let Some(previous) = existing.insert(key.clone(), record.clone())
            && previous != record
        {
            bail!("conflicting materialized label for {key}");
        }
    }
    let mut additions = Vec::new();
    for record in records {
        let key = record.materialization_key();
        if let Some(previous) = existing.get(&key) {
            if previous != record {
                bail!("conflicting materialized label for {key}");
            }
            continue;
        }
        existing.insert(key, record.clone());
        additions.push(record);
    }
    if !additions.is_empty() {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        for record in additions {
            serde_json::to_writer(&mut file, record)?;
            file.write_all(b"\n")?;
        }
    }
    Ok(path)
}

pub fn append_materialized_labels(
    root: &Path,
    records: &[MaterializedLabelRecord],
) -> Result<PathBuf> {
    with_materialization_lock(root, || append_materialized_labels_locked(root, records))
}

pub fn load_materialized_labels(root: &Path) -> Result<Vec<MaterializedLabelRecord>> {
    let path = root.join("ledgers").join(MATERIALIZED_LABELS_LEDGER);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(&path).with_context(|| format!("open {}", path.display()))?;
    std::io::BufReader::new(file)
        .lines()
        .filter(|line| line.as_ref().map_or(true, |line| !line.trim().is_empty()))
        .map(|line| Ok(serde_json::from_str(&line?)?))
        .collect()
}

fn structured_labels(
    row: &WorldTraceRow,
    action: &WorldGuardedAction,
    outcome: &WorldGuardrailOutcome,
    verifications: &[&crate::guardrail::VerificationOutcome],
    contradictions: &mut Vec<LabelContradiction>,
) -> WorldLabelSet {
    let mut labels = row.labels.clone();
    labels.success = verified_success(action, outcome, verifications);
    labels.failure = labels.success == Some(false);
    labels.user_correction = outcome.user_correction_observed;
    if row.labels.user_correction != labels.user_correction {
        contradictions.push(LabelContradiction {
            action_attempt_id: action.action_id.clone(),
            label: "user_correction".into(),
            heuristic_value: row.labels.user_correction,
            classified_value: labels.user_correction,
        });
    }
    labels.plan_drift = outcome.plan_drift_observed;
    labels.provider_incident |= outcome.provider_incident_observed;
    labels.retry |= outcome.retry_count > 0;
    labels
}

fn verified_success(
    action: &WorldGuardedAction,
    outcome: &WorldGuardrailOutcome,
    verifications: &[&crate::guardrail::VerificationOutcome],
) -> Option<bool> {
    if matches!(
        outcome.final_status,
        GuardrailFinalStatus::Failed | GuardrailFinalStatus::BlockedFailedVerification
    ) || verifications
        .iter()
        .any(|verification| verification.status == VerificationStatus::Failed)
    {
        return Some(false);
    }
    let required = action
        .verification_plan
        .iter()
        .filter(|requirement| requirement.required_for_final)
        .collect::<Vec<_>>();
    if required.is_empty()
        || !matches!(
            outcome.final_status,
            GuardrailFinalStatus::CompletedVerified
        )
    {
        return None;
    }
    let all_passed = required.iter().all(|requirement| {
        let statuses = verifications
            .iter()
            .filter(|verification| verification.requirement_id == requirement.requirement_id)
            .map(|verification| verification.status)
            .collect::<Vec<_>>();
        !statuses.is_empty()
            && statuses
                .iter()
                .all(|status| *status == VerificationStatus::Passed)
    });
    all_passed.then_some(true)
}

fn validate_embedded_verifications(
    outcome: &WorldGuardrailOutcome,
    persisted: &BTreeMap<&str, &crate::guardrail::VerificationOutcome>,
    action_attempt_id: &str,
) -> Result<()> {
    for embedded in &outcome.verification_outcomes {
        let stored = persisted
            .get(embedded.idempotency_key.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "missing persisted verification: {}",
                    embedded.idempotency_key
                )
            })?;
        if *stored != embedded || stored.action_id != action_attempt_id {
            bail!(
                "conflicting persisted verification: {}",
                embedded.idempotency_key
            );
        }
    }
    Ok(())
}

fn validate_prediction_reference(
    outcome: &WorldGuardrailOutcome,
    prediction_attempts: &BTreeMap<String, String>,
) -> Result<()> {
    if let Some(prediction_id) = outcome.prediction_id.as_deref() {
        let action_attempt_id = prediction_attempts.get(prediction_id).ok_or_else(|| {
            anyhow::anyhow!(
                "missing persisted prediction {prediction_id} for action {}",
                outcome.action_id
            )
        })?;
        if action_attempt_id != &outcome.action_id {
            bail!(
                "prediction {prediction_id} references action {action_attempt_id}, expected {}",
                outcome.action_id
            );
        }
    }
    Ok(())
}

fn unique_verifications(
    verifications: &[crate::guardrail::VerificationOutcome],
) -> Result<BTreeMap<&str, &crate::guardrail::VerificationOutcome>> {
    let mut result = BTreeMap::new();
    for verification in verifications {
        if verification.idempotency_key.is_empty() {
            bail!(
                "verification has empty identity for action {}",
                verification.action_id
            );
        }
        if let Some(previous) = result.insert(verification.idempotency_key.as_str(), verification)
            && previous != verification
        {
            bail!(
                "conflicting persisted verification: {}",
                verification.idempotency_key
            );
        }
    }
    Ok(result)
}

fn unique_actions(actions: &[WorldGuardedAction]) -> Result<BTreeMap<&str, &WorldGuardedAction>> {
    let mut result = BTreeMap::new();
    for action in actions {
        if let Some(previous) = result.insert(action.action_id.as_str(), action)
            && previous != action
        {
            bail!("conflicting guarded action: {}", action.action_id);
        }
    }
    Ok(result)
}

fn unique_outcomes(
    outcomes: &[WorldGuardrailOutcome],
) -> Result<BTreeMap<&str, &WorldGuardrailOutcome>> {
    let mut result = BTreeMap::new();
    for outcome in outcomes {
        if let Some(previous) = result.insert(outcome.action_id.as_str(), outcome)
            && previous != outcome
        {
            bail!("conflicting guardrail outcome: {}", outcome.action_id);
        }
    }
    Ok(result)
}
