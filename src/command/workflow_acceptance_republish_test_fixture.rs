//! A synthetic, fully frozen task set whose contract carries a refuted check.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceCheck, AcceptanceContract,
    AcceptanceCriterion, AcceptanceLock, AcceptancePin, FreezeGateMode, FreezeGateStamp, GapPolicy,
    JudgeDecision, JudgeVerdict, PrdIdentity, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    TrustedCwd, content_digest,
};
use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton, TaskSkeletonLock};

pub(crate) struct FrozenSet {
    pub(crate) project: tempfile::TempDir,
    pub(crate) tasks: PathBuf,
    pub(crate) prd: PathBuf,
}

impl FrozenSet {
    pub(crate) fn pin_path(&self) -> PathBuf {
        crate::command::workflow_task_set::acceptance_pin_path(self.project.path(), &self.tasks)
    }

    pub(crate) fn contract_bytes(&self) -> Vec<u8> {
        std::fs::read(self.tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap()
    }

    pub(crate) fn contract(&self) -> AcceptanceContract {
        serde_json::from_slice(&self.contract_bytes()).unwrap()
    }

    pub(crate) fn pin(&self) -> AcceptancePin {
        serde_json::from_slice(&std::fs::read(self.pin_path()).unwrap()).unwrap()
    }

    /// Every chain file, for asserting nothing was written.
    pub(crate) fn chain_bytes(&self) -> Vec<Vec<u8>> {
        [
            self.tasks.join(ACCEPTANCE_CONTRACT_FILE),
            self.tasks.join(ACCEPTANCE_LOCK_FILE),
            self.tasks.join(TASK_SKELETON_FILE),
            self.tasks.join(TASK_SKELETON_LOCK_FILE),
            self.pin_path(),
        ]
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect()
    }
}

fn stamp(finding_count: usize) -> FreezeGateStamp {
    FreezeGateStamp {
        mode: FreezeGateMode::Observe,
        finding_count,
        findings_digest: if finding_count == 0 {
            archon_workflow::task_set_contract::empty_gate_findings_digest()
        } else {
            content_digest(b"fixture findings")
        },
        binary_commit: "fixture".into(),
        evaluated_at: "2026-09-01T00:00:00Z".into(),
    }
}

pub(crate) fn criterion(id: &str, command: &str, accepted: bool) -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: id.into(),
        criterion: format!("criterion text for {id}"),
        check: AcceptanceCheck::Command {
            command: command.into(),
            cwd: TrustedCwd::ProjectRoot,
        },
        gap_permitted: false,
        judgment: JudgeVerdict {
            verdict: if accepted {
                JudgeDecision::Accepted
            } else {
                JudgeDecision::Refuted
            },
            counterexample: format!("closest passing-but-false state for {id}"),
            reason: format!("judge reasoning for {id}"),
            host_call_id: format!("acceptance-judge-batch:{id}"),
            sampling: None,
        },
    }
}

/// A frozen set with one task per check: `checks` are (id, command, accepted).
pub(crate) fn frozen_set(checks: &[(&str, &str, bool)]) -> FrozenSet {
    let project = tempfile::tempdir().unwrap();
    let tasks = project.path().join("tasks/PRD-F");
    std::fs::create_dir_all(&tasks).unwrap();
    let prd = project.path().join("prds/PRD-F.md");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    let mut table = String::from("## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n");
    for (id, _, _) in checks {
        table.push_str(&format!("| {id} | criterion text for {id} |\n"));
    }
    std::fs::write(&prd, &table).unwrap();
    let contract = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prds/PRD-F.md".into(),
            digest: content_digest(table.as_bytes()),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: Default::default(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: checks
            .iter()
            .map(|(id, command, accepted)| criterion(id, command, *accepted))
            .collect(),
        supplementary: Vec::new(),
    };
    let refuted = checks.iter().filter(|(_, _, accepted)| !accepted).count();
    write_chain(project.path(), &tasks, &contract, refuted);
    FrozenSet {
        project,
        tasks,
        prd,
    }
}

/// Write contract, lock, skeleton (one task per check), skeleton lock and pin.
pub(crate) fn write_chain(
    project: &Path,
    tasks: &Path,
    contract: &AcceptanceContract,
    findings: usize,
) {
    let bytes = serde_json::to_vec_pretty(contract).unwrap();
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
    let digest = content_digest(&bytes);
    let lock = AcceptanceLock {
        algorithm: "blake3".into(),
        digest: digest.clone(),
        gate: stamp(findings),
    };
    std::fs::write(
        tasks.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
    let frozen = contract
        .acceptance
        .iter()
        .enumerate()
        .map(|(index, entry)| FrozenTask {
            task_id: format!("TASK-F-{:03}", index + 1),
            file_name: format!("TASK-F-{:03}.md", index + 1),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: vec![entry.id.clone()],
            deliverable_contracts: Vec::new(),
        })
        .collect::<Vec<_>>();
    for task in &frozen {
        std::fs::write(tasks.join(&task.file_name), format!("# {}\n", task.task_id)).unwrap();
    }
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: digest.clone(),
        tasks: frozen,
    };
    let skeleton_bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    let skeleton_digest = content_digest(&skeleton_bytes);
    std::fs::write(tasks.join(TASK_SKELETON_FILE), &skeleton_bytes).unwrap();
    std::fs::write(
        tasks.join(TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: skeleton_digest.clone(),
            acceptance_digest: digest.clone(),
            gate: stamp(0),
        })
        .unwrap(),
    )
    .unwrap();
    let pin = AcceptancePin {
        task_root: tasks.canonicalize().unwrap().display().to_string(),
        acceptance_digest: digest.clone(),
        freeze_event_id: format!("acceptance-freeze-{}", &digest[..12]),
        acceptance_gate: stamp(findings),
        skeleton_digest: Some(skeleton_digest),
        skeleton_gate: Some(stamp(0)),
        fidelity_waivers: Vec::new(),
    };
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project, tasks);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
}

/// Every byte outside the named entries is the frozen file's: swapping the
/// frozen entries back into the republished contract reproduces the frozen
/// bytes exactly.
pub(crate) fn assert_only_named_entries_changed(
    before: &[u8],
    after: &[u8],
    named: &BTreeSet<String>,
) {
    let old: AcceptanceContract = serde_json::from_slice(before).unwrap();
    let mut new: AcceptanceContract = serde_json::from_slice(after).unwrap();
    assert_eq!(serde_json::to_vec_pretty(&new).unwrap(), after, "canonical");
    for entry in new.acceptance.iter_mut().chain(&mut new.supplementary) {
        let frozen = old
            .acceptance
            .iter()
            .chain(&old.supplementary)
            .find(|old| old.id == entry.id)
            .unwrap();
        if named.contains(&entry.id) {
            assert_ne!(entry, frozen, "{} was re-authored", entry.id);
            *entry = frozen.clone();
        } else {
            assert_eq!(entry, frozen, "{} is untouched", entry.id);
        }
    }
    assert_eq!(serde_json::to_vec_pretty(&new).unwrap(), before);
}
