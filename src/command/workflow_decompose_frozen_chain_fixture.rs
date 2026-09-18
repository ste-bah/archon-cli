//! A synthetic frozen chain for tests: contract, lock, skeleton, lock and
//! pin, written the way the freeze commands publish them. The PRD at `prd`
//! must define exactly the acceptance id `AC-X-001`.

use std::collections::BTreeSet;
use std::path::Path;

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceCheck, AcceptanceContract,
    AcceptanceCriterion, AcceptanceLock, AcceptancePin, FreezeGateMode, FreezeGateStamp, GapPolicy,
    JudgeDecision, JudgeVerdict, PrdIdentity, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    TrustedCwd, content_digest, empty_gate_findings_digest,
};
use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton, TaskSkeletonLock};

pub(crate) fn stamp() -> FreezeGateStamp {
    FreezeGateStamp {
        mode: FreezeGateMode::Observe,
        finding_count: 0,
        findings_digest: empty_gate_findings_digest(),
        binary_commit: "test-revision".into(),
        evaluated_at: "2026-08-27T00:00:00Z".into(),
    }
}

pub(crate) fn contract_bytes(prd_digest: &str) -> Vec<u8> {
    let contract = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prds/PRD-X.md".into(),
            digest: prd_digest.into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: BTreeSet::new(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: vec![AcceptanceCriterion {
            id: "AC-X-001".into(),
            criterion: "The fixture is proven.".into(),
            check: AcceptanceCheck::Command {
                command: "true".into(),
                cwd: TrustedCwd::ProjectRoot,
            },
            gap_permitted: false,
            judgment: JudgeVerdict {
                verdict: JudgeDecision::Accepted,
                counterexample: "missing output".into(),
                reason: "the declared check rejects it".into(),
                sampling: None,
                host_call_id: "judge-1".into(),
            },
        }],
        supplementary: Vec::new(),
    };
    serde_json::to_vec_pretty(&contract).unwrap()
}

fn write_pin(
    project: &Path,
    tasks: &Path,
    acceptance_digest: &str,
    skeleton_digest: Option<String>,
) {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project, tasks);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    std::fs::write(
        pin_path,
        serde_json::to_vec_pretty(&AcceptancePin {
            task_root: tasks.canonicalize().unwrap().display().to_string(),
            acceptance_digest: acceptance_digest.into(),
            freeze_event_id: "acceptance-freeze-fixture".into(),
            acceptance_gate: stamp(),
            skeleton_gate: skeleton_digest.as_ref().map(|_| stamp()),
            skeleton_digest,
            fidelity_waivers: Vec::new(),
        })
        .unwrap(),
    )
    .unwrap();
}

/// Freeze the acceptance contract against the PRD at `prd`; returns the
/// frozen acceptance digest.
pub(crate) fn freeze_acceptance(project: &Path, prd: &Path, tasks: &Path) -> String {
    let bytes = contract_bytes(&content_digest(&std::fs::read(prd).unwrap()));
    let acceptance_digest = content_digest(&bytes);
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), bytes).unwrap();
    std::fs::write(
        tasks.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&AcceptanceLock {
            algorithm: "blake3".into(),
            digest: acceptance_digest.clone(),
            gate: stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    write_pin(project, tasks, &acceptance_digest, None);
    acceptance_digest
}

/// Freeze a skeleton of `ids` (file `<id>.md`, each implementing AC-X-001)
/// on top of the frozen acceptance contract.
pub(crate) fn freeze_skeleton(project: &Path, tasks: &Path, acceptance_digest: &str, ids: &[&str]) {
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: acceptance_digest.into(),
        tasks: ids
            .iter()
            .map(|id| FrozenTask {
                task_id: (*id).into(),
                file_name: format!("{id}.md"),
                depends_on: Vec::new(),
                blocks: Vec::new(),
                implements: vec!["AC-X-001".into()],
                deliverable_contracts: Vec::new(),
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    let digest = content_digest(&bytes);
    std::fs::write(tasks.join(TASK_SKELETON_FILE), bytes).unwrap();
    std::fs::write(
        tasks.join(TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: digest.clone(),
            acceptance_digest: acceptance_digest.into(),
            gate: stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    write_pin(project, tasks, acceptance_digest, Some(digest));
}

/// Acceptance and skeleton frozen together.
pub(crate) fn freeze_chain(project: &Path, prd: &Path, tasks: &Path, ids: &[&str]) {
    let acceptance_digest = freeze_acceptance(project, prd, tasks);
    freeze_skeleton(project, tasks, &acceptance_digest, ids);
}
