use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use archon_workflow::{
    WorkflowError, WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
    WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCompletionEvidenceKind,
};

use super::workflow_live_task_universe::WorkflowV2TaskUniverse;

#[derive(Default)]
pub(super) struct CompletionCredit {
    pub(super) implementation: BTreeSet<String>,
    pub(super) verification: BTreeSet<String>,
    pub(super) noop: BTreeSet<String>,
}

impl CompletionCredit {
    pub(super) fn from_store(
        store: &WorkflowV2ResultStore,
    ) -> archon_workflow::WorkflowResult<Self> {
        let mut credit = Self::default();
        for record in store.load_call_records()? {
            credit.record_all(&record.completion_evidence);
        }
        for outcome in store.load_branch_outcomes()? {
            credit.record_all(&outcome.completion_evidence);
        }
        Ok(credit)
    }

    pub(super) fn completed_ids(&self) -> BTreeSet<String> {
        self.noop
            .iter()
            .chain(self.implementation.intersection(&self.verification))
            .cloned()
            .collect()
    }

    fn record_all(&mut self, evidence: &[WorkflowV2TaskCompletionEvidence]) {
        for item in evidence {
            self.record(item);
        }
    }

    pub(super) fn record(&mut self, evidence: &WorkflowV2TaskCompletionEvidence) {
        if !matches!(
            evidence.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ) {
            return;
        }
        match evidence.evidence_kind {
            WorkflowV2TaskCompletionEvidenceKind::VerifiedNoop => {
                self.noop.insert(evidence.task_id.clone());
            }
            WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate
                if evidence.status == WorkflowV2Status::Noop =>
            {
                self.noop.insert(evidence.task_id.clone());
            }
            WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate => {
                self.implementation.insert(evidence.task_id.clone());
            }
            WorkflowV2TaskCompletionEvidenceKind::FocusedVerification => {
                self.verification.insert(evidence.task_id.clone());
            }
        }
    }
}

pub(super) fn prepare_resume_credit(
    store: &WorkflowV2ResultStore,
    universe: &WorkflowV2TaskUniverse,
) -> archon_workflow::WorkflowResult<BTreeSet<String>> {
    let mut credit = CompletionCredit::from_store(store)?;
    let verified_noops = verified_noop_task_ids(store, universe)?;
    credit
        .noop
        .retain(|task_id| verified_noops.contains(task_id));
    let mut completed = credit.completed_ids();
    apply_terminal_report_credit(store, &mut completed)?;
    let authoritative = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect::<BTreeSet<_>>();
    completed.retain(|task_id| authoritative.contains(task_id));
    archive_terminal_results(store)?;
    Ok(completed)
}

pub(super) fn noop_acceptance_criteria_satisfied(
    task_id: &str,
    result: Option<&WorkflowV2Result>,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> bool {
    let Some(universe) = universe else {
        return true;
    };
    let Some(task) = universe
        .tasks
        .iter()
        .find(|task| task.canonical_task_id == task_id)
    else {
        return false;
    };
    if task.acceptance_criteria.is_empty() {
        return false;
    }
    let Some(result) = result else {
        return false;
    };
    let mut criterion_results = Vec::new();
    collect_criterion_results(&result.data, &mut criterion_results);
    task.acceptance_criteria.iter().all(|criterion| {
        criterion_results.iter().any(|entry| {
            entry
                .get("criterion")
                .or_else(|| entry.get("acceptance_criterion"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value.trim() == criterion.trim())
                && entry
                    .get("task_id")
                    .or_else(|| entry.get("canonical_task_id"))
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|value| value == task_id)
                && criterion_status_passed(entry)
                && entry.get("evidence_refs").is_some_and(value_present)
        })
    })
}

fn verified_noop_task_ids(
    store: &WorkflowV2ResultStore,
    universe: &WorkflowV2TaskUniverse,
) -> archon_workflow::WorkflowResult<BTreeSet<String>> {
    let mut verified = BTreeSet::new();
    for record in store.load_call_records()? {
        record_verified_noops(
            &mut verified,
            &record.completion_evidence,
            Some(&record.result),
            universe,
        );
    }
    for outcome in store.load_branch_outcomes()? {
        record_verified_noops(
            &mut verified,
            &outcome.completion_evidence,
            outcome.result.as_ref(),
            universe,
        );
    }
    Ok(verified)
}

fn record_verified_noops(
    verified: &mut BTreeSet<String>,
    evidence: &[WorkflowV2TaskCompletionEvidence],
    result: Option<&WorkflowV2Result>,
    universe: &WorkflowV2TaskUniverse,
) {
    for item in evidence.iter().filter(|item| {
        item.evidence_kind == WorkflowV2TaskCompletionEvidenceKind::VerifiedNoop
            || (item.evidence_kind == WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate
                && item.status == WorkflowV2Status::Noop)
    }) {
        if noop_acceptance_criteria_satisfied(&item.task_id, result, Some(universe)) {
            verified.insert(item.task_id.clone());
        }
    }
}

fn collect_criterion_results(value: &serde_json::Value, results: &mut Vec<serde_json::Value>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if matches!(
                    key.as_str(),
                    "acceptance_criteria_results" | "criterion_results"
                ) && let Some(items) = child.as_array()
                {
                    results.extend(items.iter().cloned());
                }
                collect_criterion_results(child, results);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_criterion_results(item, results);
            }
        }
        _ => {}
    }
}

fn criterion_status_passed(value: &serde_json::Value) -> bool {
    value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "accepted" | "complete" | "completed" | "pass" | "passed" | "satisfied"
            )
        })
}

fn value_present(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(value) => !value.trim().is_empty(),
        serde_json::Value::Array(values) => !values.is_empty(),
        serde_json::Value::Object(values) => !values.is_empty(),
        _ => true,
    }
}

fn apply_terminal_report_credit(
    store: &WorkflowV2ResultStore,
    completed: &mut BTreeSet<String>,
) -> archon_workflow::WorkflowResult<()> {
    for record in terminal_records(store)? {
        let data = &record.result.data;
        for task_id in task_ids(data, &["accepted_tasks"]) {
            completed.insert(task_id);
        }
        for task_id in task_ids(data, &["failed_tasks", "blocked_tasks"]) {
            completed.remove(&task_id);
        }
    }
    Ok(())
}

fn terminal_records(
    store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<Vec<archon_workflow::WorkflowV2CallRecord>> {
    let mut records = store
        .load_call_records()?
        .into_iter()
        .filter(|record| record.call.method == WorkflowV2HostMethod::FinalReport)
        .collect::<Vec<_>>();
    records.sort_by(|left, right| left.finished_at.cmp(&right.finished_at));
    Ok(records)
}

fn task_ids(data: &serde_json::Value, keys: &[&str]) -> BTreeSet<String> {
    keys.iter()
        .filter_map(|key| data.get(*key).and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|task_id| !task_id.is_empty())
        .map(str::to_string)
        .collect()
}

fn archive_terminal_results(store: &WorkflowV2ResultStore) -> archon_workflow::WorkflowResult<()> {
    let records = terminal_records(store)?;
    if records.is_empty() {
        return Ok(());
    }
    let archive = resume_archive_dir(store);
    fs::create_dir_all(&archive).map_err(|source| WorkflowError::Io {
        path: archive.clone(),
        source,
    })?;
    for record in records {
        let source = store.result_path(&record.call.id);
        let target = archive.join(source.file_name().unwrap_or_default());
        fs::rename(&source, &target).map_err(|error| WorkflowError::Io {
            path: source.clone(),
            source: error,
        })?;
    }
    Ok(())
}

fn resume_archive_dir(store: &WorkflowV2ResultStore) -> PathBuf {
    let attempt = chrono::Utc::now().format("%Y%m%dT%H%M%S%fZ");
    store
        .root()
        .join("archived-resume-terminals")
        .join(attempt.to_string())
}
