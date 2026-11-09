//! Read-only rendering of diagnostic classification records.
use anyhow::{Context, Result};
use archon_workflow::baseline_results::results_digest;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_LOCK_FILE, AcceptanceLock, TASK_SKELETON_LOCK_FILE,
};
use archon_workflow::task_skeleton::TaskSkeletonLock;
use archon_workflow::v2::requirement_classes::{RequirementClass, RequirementClassification};
use std::path::Path;

pub(super) fn render(task_root: &Path) -> String {
    match read(task_root) {
        Ok(text) => format!("{text}{}", states::render(task_root)),
        Err(error) => format!(
            "classification: unreadable ({error:#})\n{}",
            states::render(task_root)
        ),
    }
}

fn read(task_root: &Path) -> Result<String> {
    let path = task_root.join(TASK_SKELETON_LOCK_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error).context("reading classification lock"),
    };
    let lock: TaskSkeletonLock =
        serde_json::from_slice(&bytes).context("parsing classification lock")?;
    let Some(record) = lock.classification else {
        return Ok(String::new());
    };
    let acceptance: AcceptanceLock = serde_json::from_slice(
        &std::fs::read(task_root.join(ACCEPTANCE_LOCK_FILE))
            .context("reading classification predecessor")?,
    )
    .context("parsing classification predecessor")?;
    let repository = archon_workflow::repository_record::read_repository_record(task_root)?;
    let root = repository.as_ref().map(|r| Path::new(&r.repository_root));
    let mut display = record.clone();
    for entry in &mut display.entries {
        for deliverable in &mut entry.deliverables {
            deliverable.artifact_path = archon_workflow::base_snapshot::snapshot_path_key(
                root.unwrap_or_else(|| Path::new("")),
                &deliverable.artifact_path,
            );
        }
    }
    Ok(lines(&display, &acceptance))
}

fn lines(record: &RequirementClassification, acceptance: &AcceptanceLock) -> String {
    let counts = [
        RequirementClass::NoCheck,
        RequirementClass::Missing,
        RequirementClass::CandidateDone,
        RequirementClass::NoBaseResult,
    ]
    .map(|class| {
        format!(
            "{}={}",
            label(&class),
            record
                .entries
                .iter()
                .filter(|entry| entry.class == class)
                .count()
        )
    })
    .join(" ");
    let stale = record.base_snapshot.as_ref().is_some_and(|s| {
        !s.valid_digest() || Some(&s.commit) != acceptance.baseline_commit.as_ref()
    }) || record.acceptance_digest != acceptance.digest
        || record.baseline_results_digest != results_digest(&acceptance.baseline_results);
    let mut out = format!("classification: {counts} stale={stale} outcomes=unauthenticated\n");
    if let Some(snapshot) = &record.base_snapshot {
        use archon_workflow::base_snapshot::SnapshotEntry;
        let tracked = snapshot
            .entries
            .values()
            .filter(|e| matches!(e, SnapshotEntry::Tracked { .. }))
            .count();
        let unverified = snapshot
            .entries
            .values()
            .filter(|e| matches!(e, SnapshotEntry::Unverified { .. }))
            .count();
        let absent = snapshot.entries.len() - tracked - unverified;
        out.push_str(&format!(
            "snapshot tracked={tracked} unverified={unverified} absent={absent}\n"
        ));
    } else {
        out.push_str("snapshot: not recorded\n");
    }
    if let Some(error) = &record.error {
        out.push_str(&format!("classification: error ({error})\n"));
    }
    for entry in &record.entries {
        let checks = entry
            .checks
            .iter()
            .map(|(id, result)| {
                format!(
                    "{id}:{}",
                    result.as_ref().map(label).unwrap_or_else(|| "none".into())
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let deliverables = entry
            .deliverables
            .iter()
            .map(|d| {
                format!(
                    "{}[at_base={},live_only={},shared={},text={},error={}]{}",
                    d.artifact_path,
                    d.at_base,
                    d.live_only,
                    d.shared,
                    d.text,
                    d.error.as_deref().unwrap_or("none"),
                    snapshot_label(record, &d.artifact_path)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&format!("classification: ({},{}) class={} reason={} fallback={} checks=[{}] deliverables=[{}] text_item={} passing_check_without_deliverable={}\n", entry.task_id, entry.id, label(&entry.class), entry.reason.as_deref().unwrap_or("none"), entry.fallback.as_ref().map(label).unwrap_or_else(|| "none".into()), checks, deliverables, entry.text_item, entry.passing_check_without_deliverable));
    }
    out
}

fn snapshot_label(record: &RequirementClassification, path: &str) -> String {
    use archon_workflow::base_snapshot::{LiveState, SnapshotEntry};
    let Some(snapshot) = &record.base_snapshot else {
        return String::new();
    };
    let entry = snapshot.entries.get(path);
    let text = match entry {
        Some(SnapshotEntry::Tracked { object, live, .. }) => format!(
            "tracked:{}{}",
            object.chars().take(12).collect::<String>(),
            if *live == LiveState::Same {
                String::new()
            } else {
                format!(" live={}", label(live))
            }
        ),
        Some(SnapshotEntry::Unverified { reason, .. }) => format!("unverified:{}", label(reason)),
        Some(SnapshotEntry::Absent) => "absent".into(),
        None => "not recorded".into(),
    };
    format!(" snapshot={text}")
}

fn label(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .expect("serializable label")
        .as_str()
        .expect("string label")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_only_prints_present_classification_and_reports_unreadable() {
        let root = tempfile::tempdir().unwrap();
        assert!(render(root.path()).is_empty());
        std::fs::write(root.path().join(TASK_SKELETON_LOCK_FILE), b"broken").unwrap();
        assert!(render(root.path()).starts_with("classification: unreadable ("));
    }
    #[test]
    fn snapshot_status_counts_live_states_and_staleness() {
        use archon_workflow::base_snapshot::{
            BaseSnapshot, LiveState, SnapshotEntry, SnapshotKind, UnverifiedReason,
        };
        let mut acceptance: AcceptanceLock = serde_json::from_slice(include_bytes!(
            "../../crates/archon-workflow/tests/fixtures/classification/acceptance.lock"
        ))
        .unwrap();
        acceptance.baseline_commit = Some("base".into());
        let mut snapshot = BaseSnapshot {
            landing_verified: Default::default(),
            commit: "base".into(),
            digest: String::new(),
            entries: [
                (
                    "tracked".into(),
                    SnapshotEntry::Tracked {
                        kind: SnapshotKind::Blob,
                        object: "12345678901234567890".into(),
                        live: LiveState::Modified,
                        live_note: None,
                    },
                ),
                (
                    "ignored".into(),
                    SnapshotEntry::Unverified {
                        reason: UnverifiedReason::Ignored,
                        live_digest: None,
                        live_note: None,
                    },
                ),
                ("absent".into(), SnapshotEntry::Absent),
            ]
            .into(),
        };
        snapshot.digest = snapshot.recomputed_digest();
        let mut record = RequirementClassification {
            schema_version: 2,
            baseline_commit: Some("base".into()),
            acceptance_digest: acceptance.digest.clone(),
            baseline_results_digest: results_digest(&acceptance.baseline_results),
            error: None,
            entries: vec![],
            base_snapshot: Some(snapshot),
        };
        let text = lines(&record, &acceptance);
        assert!(
            text.contains("stale=false")
                && text.contains("snapshot tracked=1 unverified=1 absent=1")
        );
        assert_eq!(
            snapshot_label(&record, "tracked"),
            " snapshot=tracked:123456789012 live=modified"
        );
        assert_eq!(
            snapshot_label(&record, "ignored"),
            " snapshot=unverified:ignored"
        );
        assert_eq!(snapshot_label(&record, "absent"), " snapshot=absent");
        for path in ["prefix/tracked", "/elsewhere/tracked", "other/ignored"] {
            assert_eq!(snapshot_label(&record, path), " snapshot=not recorded");
        }
        acceptance.baseline_commit = Some("later".into());
        assert!(lines(&record, &acceptance).contains("stale=true"));
        acceptance.baseline_commit = Some("base".into());
        record.base_snapshot.as_mut().unwrap().digest.push('x');
        assert!(lines(&record, &acceptance).contains("stale=true"));
        record.base_snapshot = None;
        record.schema_version = 1;
        assert!(lines(&record, &acceptance).contains("snapshot: not recorded"));
    }
    #[test]
    fn s4a_status_presence_staleness_budget_and_error() {
        use archon_workflow::baseline_results::{BaselineCheckResult, BaselineOutcome};
        use archon_workflow::v2::requirement_classes::{Fallback, RequirementEntry};
        let root = tempfile::tempdir().unwrap();
        let mut acceptance: AcceptanceLock = serde_json::from_slice(include_bytes!(
            "../../crates/archon-workflow/tests/fixtures/classification/acceptance.lock"
        ))
        .unwrap();
        let mut lock: TaskSkeletonLock = serde_json::from_slice(include_bytes!(
            "../../crates/archon-workflow/tests/fixtures/classification/skeleton.lock"
        ))
        .unwrap();
        std::fs::write(
            root.path().join(TASK_SKELETON_LOCK_FILE),
            serde_json::to_vec(&lock).unwrap(),
        )
        .unwrap();
        assert!(
            render(root.path()).is_empty(),
            "old lock needs no predecessor to render nothing"
        );
        let mut record = RequirementClassification {
            base_snapshot: None,
            schema_version: 1,
            baseline_commit: Some("base".into()),
            acceptance_digest: acceptance.digest.clone(),
            baseline_results_digest: results_digest(&acceptance.baseline_results),
            error: None,
            entries: vec![RequirementEntry {
                task_id: "TASK-A-001".into(),
                id: "REQ-1".into(),
                class: RequirementClass::CandidateDone,
                reason: None,
                fallback: Some(Fallback::Missing),
                checks: [
                    ("AC-1".into(), Some(BaselineOutcome::Passed)),
                    ("AC-2".into(), Some(BaselineOutcome::Failed)),
                    ("AC-3".into(), None),
                ]
                .into(),
                deliverables: Vec::new(),
                text_item: false,
                passing_check_without_deliverable: false,
            }],
        };
        let text = lines(&record, &acceptance);
        assert!(
            text.contains("candidate_done=1")
                && text.contains("stale=false outcomes=unauthenticated\n"),
            "{text}"
        );
        assert!(
            text.contains("fallback=missing checks=[AC-1:passed,AC-2:failed,AC-3:none]"),
            "{text}"
        );
        acceptance.digest = "changed".into();
        assert!(lines(&record, &acceptance).contains("stale=true"));
        acceptance.digest.clone_from(&record.acceptance_digest);
        acceptance.baseline_results.insert(
            "AC-1".into(),
            BaselineCheckResult {
                check_digest: "changed".into(),
                outcome: BaselineOutcome::Passed,
            },
        );
        assert!(lines(&record, &acceptance).contains("stale=true"));
        record.error = Some("exact cause".into());
        record.entries.clear();
        lock.classification = Some(record);
        std::fs::write(
            root.path().join(TASK_SKELETON_LOCK_FILE),
            serde_json::to_vec(&lock).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.path().join(ACCEPTANCE_LOCK_FILE),
            serde_json::to_vec(&acceptance).unwrap(),
        )
        .unwrap();
        let text = render(root.path());
        assert!(
            text.contains("classification: error (exact cause)"),
            "{text}"
        );
    }
}

#[path = "workflow_decompose_status_states.rs"]
mod states;
