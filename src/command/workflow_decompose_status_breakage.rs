//! Read-only diagnostic proof status.
use crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR;
use anyhow::{Context, Result};
use archon_workflow::baseline_results::results_digest;
use archon_workflow::breakage_proof::{BREAKAGE_SCHEMA, BreakageProofs, selection::estimate};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, AcceptanceLock,
    TASK_SKELETON_LOCK_FILE,
};
use archon_workflow::task_skeleton::TaskSkeletonLock;
use std::collections::BTreeMap;
use std::path::Path;
pub(super) fn render(project: &Path, task: &Path) -> String {
    match read(project, task) {
        Ok(s) => s,
        Err(e) => format!("breakage: proofs=unreadable ({e:#})\n"),
    }
}
fn label(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}
fn read(project: &Path, task: &Path) -> Result<String> {
    let bytes = match std::fs::read(task.join(TASK_SKELETON_LOCK_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e.into()),
    };
    let lock: TaskSkeletonLock = serde_json::from_slice(&bytes)?;
    let Some(classification) = lock.classification else {
        return Ok(String::new());
    };
    let acceptance: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(task.join(ACCEPTANCE_LOCK_FILE))?)?;
    let contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(task.join(ACCEPTANCE_CONTRACT_FILE))?)?;
    let repository = archon_workflow::repository_record::read_repository_record(task)?;
    let root = repository
        .as_ref()
        .map(|r| Path::new(&r.repository_root))
        .unwrap_or(project);
    let estimate = estimate(&classification, &contract, &acceptance, root);
    let commit = acceptance.baseline_commit.as_deref().unwrap_or("");
    let digest = &classification.acceptance_digest;
    let safe = !commit.is_empty()
        && commit.bytes().all(|b| b.is_ascii_hexdigit())
        && digest.len() >= 16
        && digest.bytes().all(|b| b.is_ascii_hexdigit());
    let dir = project
        .join(archon_workflow::base_snapshot::key_path(FREEZE_CACHE_DIR))
        .join("breakage")
        .join(commit);
    let path = dir
        .join(digest.get(..16).unwrap_or(""))
        .join("summary.json");
    let bytes = if safe {
        match std::fs::read(path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context("reading proofs"),
        }
    } else {
        None
    };
    let Some(bytes) = bytes else {
        return Ok(format!(
            "breakage: proofs=none estimate_runs={estimate} runs_executed=0 runs_reused=0 stale=false\n"
        ));
    };
    let proofs: BreakageProofs = serde_json::from_slice(&bytes).context("parsing proofs")?;
    anyhow::ensure!(
        proofs.schema == BREAKAGE_SCHEMA,
        "incompatible proof schema"
    );
    let summary = proofs.summary.context("proof summary absent")?;
    let stale = summary.acceptance_digest != acceptance.digest
        || summary.baseline_results_digest != results_digest(&acceptance.baseline_results)
        || classification
            .base_snapshot
            .as_ref()
            .is_none_or(|s| s.digest != summary.snapshot_digest || !s.valid_digest());
    let mut counts = BTreeMap::new();
    for pair in &summary.entries {
        *counts.entry(label(&pair.state)).or_insert(0u64) += 1;
    }
    let counts = counts
        .into_iter()
        .map(|(state, n)| format!("{state}={n}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = format!(
        "breakage: proofs=recorded estimate_runs={estimate} runs_executed={} runs_reused={} {counts} stale={stale}\n",
        summary.runs_executed, summary.runs_reused
    );
    for pair in summary.entries {
        let kills = if pair.counted_kills.is_empty() {
            "-".into()
        } else {
            pair.counted_kills.join(",")
        };
        let evidence = proofs
            .runs
            .values()
            .find(|r| r.task_id == pair.task_id && r.requirement_id == pair.id)
            .and_then(|r| {
                r.evidence.patch.rsplit_once('/').map(|(p, _)| {
                    dir.join(archon_workflow::base_snapshot::key_path(p))
                        .display()
                        .to_string()
                })
            })
            .unwrap_or_else(|| "-".into());
        out.push_str(&format!(
            "breakage {} {}: {} kills={kills} checks={} evidence={evidence}\n",
            pair.task_id,
            pair.id,
            label(&pair.state),
            pair.checks.join(",")
        ));
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_and_unreadable_locks() {
        let dir = tempfile::tempdir().unwrap();
        assert!(render(dir.path(), dir.path()).is_empty());
        std::fs::write(dir.path().join(TASK_SKELETON_LOCK_FILE), b"invalid").unwrap();
        assert!(render(dir.path(), dir.path()).starts_with("breakage: proofs=unreadable ("));
    }
    #[test]
    fn classification_estimate_recorded_summary_and_staleness() {
        use archon_workflow::base_snapshot::{
            BaseSnapshot, LiveState, SnapshotEntry, SnapshotKind,
        };
        use archon_workflow::baseline_results::{
            BaselineCheckResult, BaselineOutcome, check_digest,
        };
        use archon_workflow::breakage_proof::*;
        use archon_workflow::v2::requirement_classes::RequirementClassification;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut skeleton: TaskSkeletonLock = serde_json::from_slice(include_bytes!(
            "../../crates/archon-workflow/tests/fixtures/classification/skeleton.lock"
        ))
        .unwrap();
        std::fs::write(
            root.join(TASK_SKELETON_LOCK_FILE),
            serde_json::to_vec(&skeleton).unwrap(),
        )
        .unwrap();
        assert!(render(root, root).is_empty());
        let contract:AcceptanceContract=serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"input","digest":"anchor"},"gap_policy":{},"acceptance":[{"id":"check","criterion":"item","check":{"kind":"command","command":"verify value","cwd":"repo_root"},"covers":["item"],"judgment":{"verdict":"accepted","counterexample":"fixture","reason":"fixture","host_call_id":"test"}}]})).unwrap();
        let mut acceptance: AcceptanceLock = serde_json::from_slice(include_bytes!(
            "../../crates/archon-workflow/tests/fixtures/classification/acceptance.lock"
        ))
        .unwrap();
        acceptance.digest = archon_workflow::task_set_contract::content_digest(b"acceptance");
        acceptance.baseline_commit = Some("a".repeat(40));
        acceptance.baseline_results = [(
            "check".into(),
            BaselineCheckResult {
                check_digest: check_digest(&contract.acceptance[0].check),
                outcome: BaselineOutcome::Passed,
            },
        )]
        .into();
        let mut snapshot = BaseSnapshot {
            commit: acceptance.baseline_commit.clone().unwrap(),
            digest: String::new(),
            entries: [(
                "value".into(),
                SnapshotEntry::Tracked {
                    kind: SnapshotKind::Blob,
                    object: "object".into(),
                    live: LiveState::Same,
                    live_note: None,
                },
            )]
            .into(),
        };
        snapshot.digest = snapshot.recomputed_digest();
        let entry = serde_json::json!({"task_id":"task","id":"item","class":"candidate_done","reason":null,"fallback":null,"checks":{"check":"passed"},"deliverables":[{"artifact_path":"value","at_base":true,"live_only":false,"shared":false,"text":false}],"text_item":false,"passing_check_without_deliverable":false});
        let classes:RequirementClassification=serde_json::from_value(serde_json::json!({"schema_version":2,"baseline_commit":acceptance.baseline_commit,"acceptance_digest":acceptance.digest,"baseline_results_digest":results_digest(&acceptance.baseline_results),"error":null,"entries":[entry.clone(),entry],"base_snapshot":snapshot})).unwrap();
        skeleton.classification = Some(classes.clone());
        for (name, bytes) in [
            (
                TASK_SKELETON_LOCK_FILE,
                serde_json::to_vec(&skeleton).unwrap(),
            ),
            (
                ACCEPTANCE_LOCK_FILE,
                serde_json::to_vec(&acceptance).unwrap(),
            ),
            (
                ACCEPTANCE_CONTRACT_FILE,
                serde_json::to_vec(&contract).unwrap(),
            ),
        ] {
            std::fs::write(root.join(name), bytes).unwrap();
        }
        assert!(render(root, root).contains("proofs=none estimate_runs=14"));
        let summary = ProofSummary {
            acceptance_digest: acceptance.digest.clone(),
            baseline_results_digest: classes.baseline_results_digest.clone(),
            snapshot_digest: classes.base_snapshot.as_ref().unwrap().digest.clone(),
            runs_executed: 4,
            runs_reused: 2,
            refusals: Vec::new(),
            entries: vec![RequirementProof {
                task_id: "task".into(),
                id: "item".into(),
                state: ProofState::BehaviourKill,
                counted_kills: vec!["m-1".into()],
                checks: vec!["check".into()],
            }],
        };
        let path = root
            .join(archon_workflow::base_snapshot::key_path(FREEZE_CACHE_DIR))
            .join("breakage")
            .join(acceptance.baseline_commit.as_ref().unwrap())
            .join(&acceptance.digest[..16])
            .join("summary.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut proofs = BreakageProofs {
            schema: BREAKAGE_SCHEMA,
            base_commit: acceptance.baseline_commit.unwrap(),
            controls: BTreeMap::new(),
            runs: BTreeMap::new(),
            summary: Some(summary.clone()),
        };
        std::fs::write(&path, serde_json::to_vec(&proofs).unwrap()).unwrap();
        let text = render(root, root);
        assert!(text.contains("behaviour_kill=1 stale=false"));
        assert!(
            text.contains("breakage task item: behaviour_kill kills=m-1 checks=check evidence=-")
        );
        for changed in ["acceptance", "results", "snapshot"] {
            let mut s = summary.clone();
            match changed {
                "acceptance" => s.acceptance_digest = "different".into(),
                "results" => s.baseline_results_digest = "different".into(),
                _ => s.snapshot_digest = "different".into(),
            };
            proofs.summary = Some(s);
            std::fs::write(&path, serde_json::to_vec(&proofs).unwrap()).unwrap();
            assert!(render(root, root).contains("stale=true"));
        }
        std::fs::write(&path, b"invalid").unwrap();
        assert!(render(root, root).starts_with("breakage: proofs=unreadable ("));
    }
}
