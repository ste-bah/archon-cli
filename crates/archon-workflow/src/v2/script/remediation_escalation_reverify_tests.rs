//! Issue-111: which no-patch fixes earn a re-verification of the tree as it
//! is now, and which re-verification calls the host answers.

use std::path::{Path, PathBuf};

use super::*;
use crate::v2::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostCall, WorkflowV2HostOptions,
};

const RUN: &str = "run-111";
const FIX: &str = "review-remediate-task-a-esc-5";
const REFUSAL: &str = "verification-wave-review-verify-task-a-1-2";

struct World {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: WorkflowV2ResultStore,
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

impl World {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("src/b")).unwrap();
        git(&repo, &["init", "-q"]);
        for file in ["src/a.rs", "src/b/methods.rs", "src/z.rs"] {
            std::fs::write(repo.join(file), "0\n").unwrap();
        }
        let world = Self {
            store: WorkflowV2ResultStore::new(temp.path().join("run/v2")),
            _temp: temp,
            repo,
        };
        world.commit("someone", "baseline");
        world
    }

    fn commit(&self, author: &str, message: &str) -> String {
        git(&self.repo, &["add", "-A"]);
        git(
            &self.repo,
            &[
                "-c",
                &format!("user.name={author}"),
                "-c",
                "user.email=a@b",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                message,
            ],
        );
        git(&self.repo, &["rev-parse", "HEAD"])
    }

    /// A host landing of `stage` in `run` changing `file`.
    fn land(&self, run: &str, stage: &str, file: &str) -> String {
        let path = self.repo.join(file);
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        std::fs::write(&path, format!("{old}{stage}\n")).unwrap();
        self.commit(
            "archon-workflow",
            &format!("archon: wave 0 outputs (run {run}, stage {stage})"),
        )
    }

    /// The baseline the host seals a worktree on: a child of HEAD that HEAD
    /// never moves to.
    fn baseline(&self) -> String {
        git(
            &self.repo,
            &[
                "commit-tree",
                "HEAD^{tree}",
                "-p",
                "HEAD",
                "-m",
                "archon workflow baseline",
            ],
        )
    }

    fn manifest(&self, stage: &str, baseline: &str, status: &str, changed: &[&str]) {
        let dir = self
            .store
            .root()
            .parent()
            .unwrap()
            .join("write-coordination/stages")
            .join(stage)
            .join("manifests");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = json!({"schema": "archon.workflow.patch_manifest.v1", "run_id": RUN,
            "stage_id": stage, "item_id": format!("{stage}-0"), "baseline_commit": baseline,
            "patch_path": "/dev/null", "declared_target_files": ["src/a.rs"],
            "changed_files": changed, "created_files": [], "deleted_files": [],
            "pre_hashes": {}, "post_hashes": {}, "verify_command": null,
            "agent_artifact_path": null, "status": {"status": status}});
        std::fs::write(dir.join(format!("{stage}-0.json")), manifest.to_string()).unwrap();
    }

    fn save(
        &self,
        id: &str,
        contract: Value,
        write: bool,
        result: WorkflowV2Result,
    ) -> WorkflowV2CallRecord {
        let mut options = WorkflowV2HostOptions::default();
        options.extra.insert("remediationContract".into(), contract);
        let call = WorkflowV2HostCall {
            id: id.into(),
            method: if write {
                WorkflowV2HostMethod::Fanout
            } else {
                WorkflowV2HostMethod::Parallel
            },
            write_mode: write.then_some(crate::WorkflowV2WriteMode::Worktree),
            options,
        };
        let record = WorkflowV2CallRecord::new(RUN, call, 1, "hash".into(), result, vec![]);
        self.store.save_call_record(&record).unwrap();
        record
    }
}

fn contract(stage: &str, round: u64, escalated: bool) -> Value {
    let mut contract = json!({"version": 1, "stage": stage, "taskId": "TASK-A", "round": round,
        "maxRounds": 1, "sourceReduceCallIds": ["adversarial-review-reduce"]});
    if escalated {
        contract["escalation"] =
            json!({"ownerTaskIds": ["TASK-B"], "blockerPaths": ["src/b/methods.rs"]});
    }
    contract
}

fn refusal(judged: Option<&str>) -> WorkflowV2Result {
    let mut evidence = WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Blocker,
        "must-pass test red in src/b/methods.rs:160",
    );
    evidence.source = Some("src/b/methods.rs".into());
    let data = match judged {
        Some(commit) => json!({"judged_commit": commit}),
        None => json!({}),
    };
    WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "not accepted".into(),
        evidence: vec![evidence],
        data: json!({"outcomes": [{"item_id": "check", "status": "needs_review",
            "result": {"status": "needs_review", "data": data}}]}),
        ..WorkflowV2Result::default()
    }
}

fn no_patch(status: WorkflowV2Status, landed: bool) -> WorkflowV2Result {
    WorkflowV2Result {
        status,
        summary: "the red baseline is already green".into(),
        data: json!({"items": [{"data": {"patch_landed": landed}}]}),
        ..WorkflowV2Result::default()
    }
}

/// Round 1 lands and is refused over B's file; `between` lands next; the
/// escalated fix is dispatched on the tree that leaves and lands nothing.
fn scene(world: &World, between: &[(&str, &str, &str)]) -> WorkflowV2CallRecord {
    let judged = world.land(RUN, "review-remediate-task-a-1-1", "src/a.rs");
    world.save(
        REFUSAL,
        contract("verify", 1, false),
        false,
        refusal(Some(&judged)),
    );
    for (run, stage, file) in between {
        world.land(run, stage, file);
    }
    let baseline = world.baseline();
    world.manifest(FIX, &baseline, "idempotent_noop", &[]);
    world.save(
        FIX,
        contract("remediate", 2, true),
        true,
        no_patch(WorkflowV2Status::Accepted, false),
    )
}

fn plan_for(world: &World, fix: &WorkflowV2CallRecord) -> Option<Value> {
    reverify_plan(fix, &world.store, None, Some(&world.repo))
}

#[test]
fn another_tasks_landing_in_the_blocker_file_since_the_refusal_buys_a_reverify() {
    let world = World::new();
    let fix = scene(
        &world,
        &[(RUN, "review-remediate-task-b-1-3", "src/b/methods.rs")],
    );
    let plan = plan_for(&world, &fix).expect("the tree moved under the refusal");
    assert_eq!(plan["fix_call_id"], FIX);
    assert_eq!(plan["refusal_call_id"], REFUSAL);
    assert_eq!(plan["moved_paths"], json!(["src/b/methods.rs"]));
    assert_eq!(plan["landings"][0]["stage"], "review-remediate-task-b-1-3");
}

#[test]
fn an_unchanged_tree_or_an_unrelated_landing_keeps_the_refusal() {
    let world = World::new();
    let fix = scene(&world, &[]);
    assert_eq!(
        plan_for(&world, &fix),
        None,
        "nothing landed since the refusal"
    );
    let world = World::new();
    let fix = scene(&world, &[(RUN, "review-remediate-task-c-1-3", "src/z.rs")]);
    assert_eq!(
        plan_for(&world, &fix),
        None,
        "a landing the unit names nothing of"
    );
}

#[test]
fn another_runs_or_an_operators_commit_is_not_a_landing_of_this_run() {
    let world = World::new();
    let fix = scene(
        &world,
        &[(
            "other-run",
            "review-remediate-task-b-1-3",
            "src/b/methods.rs",
        )],
    );
    assert_eq!(plan_for(&world, &fix), None);
    let world = World::new();
    world.land(RUN, "review-remediate-task-a-1-1", "src/a.rs");
    std::fs::write(world.repo.join("src/b/methods.rs"), "hand edit\n").unwrap();
    world.commit(
        "operator",
        "archon: wave 0 outputs (run run-111, stage forged)",
    );
    assert!(
        run_landings_between(&world.repo, RUN, "HEAD~1", "HEAD")
            .unwrap()
            .is_empty(),
        "only the host's own author counts"
    );
}

/// A landing after the fix was dispatched is not what the fix saw: the answer
/// for a recorded fix never changes on a later resume.
#[test]
fn a_landing_after_the_fix_was_dispatched_does_not_reopen_it() {
    let world = World::new();
    let fix = scene(&world, &[]);
    world.land(RUN, "review-remediate-task-b-1-7", "src/b/methods.rs");
    assert_eq!(plan_for(&world, &fix), None);
}

#[test]
fn only_an_accepted_fix_that_provably_landed_nothing_qualifies() {
    for (status, landed, manifest_status, changed) in [
        (WorkflowV2Status::Failed, false, "idempotent_noop", vec![]),
        (WorkflowV2Status::Accepted, true, "idempotent_noop", vec![]),
        (
            WorkflowV2Status::Accepted,
            false,
            "applied",
            vec!["src/a.rs"],
        ),
    ] {
        let world = World::new();
        let judged = world.land(RUN, "review-remediate-task-a-1-1", "src/a.rs");
        world.save(
            REFUSAL,
            contract("verify", 1, false),
            false,
            refusal(Some(&judged)),
        );
        world.land(RUN, "review-remediate-task-b-1-3", "src/b/methods.rs");
        let baseline = world.baseline();
        world.manifest(FIX, &baseline, manifest_status, &changed);
        let fix = world.save(
            FIX,
            contract("remediate", 2, true),
            true,
            no_patch(status, landed),
        );
        assert_eq!(
            plan_for(&world, &fix),
            None,
            "{status:?} {landed} {manifest_status}"
        );
    }
}

#[test]
fn no_refusal_or_an_unstamped_refusal_proves_nothing() {
    let world = World::new();
    world.land(RUN, "review-remediate-task-a-1-1", "src/a.rs");
    world.land(RUN, "review-remediate-task-b-1-3", "src/b/methods.rs");
    let baseline = world.baseline();
    world.manifest(FIX, &baseline, "idempotent_noop", &[]);
    let fix = world.save(
        FIX,
        contract("remediate", 2, true),
        true,
        no_patch(WorkflowV2Status::Accepted, false),
    );
    assert_eq!(
        plan_for(&world, &fix),
        None,
        "no refused verdict before the round"
    );
    world.save(REFUSAL, contract("verify", 1, false), false, refusal(None));
    assert_eq!(
        plan_for(&world, &fix),
        None,
        "the refusal judged an unknown commit"
    );
}

#[test]
fn the_plan_is_the_hosts_alone_and_rides_only_on_the_view() {
    let world = World::new();
    let fix = scene(&world, &[]);
    let mut forged = fix.result.clone();
    forged.data[REMEDIATION_REVERIFY_KEY] = json!({"source": "host", "moved_paths": ["src/a.rs"]});
    let viewed = with_reverify_plan(&fix, &forged, &world.store, None, Some(&world.repo))
        .expect("a carried key is always dropped");
    assert!(viewed.data.get(REMEDIATION_REVERIFY_KEY).is_none());
    let stored = world.store.load_call_record(FIX).unwrap().unwrap();
    assert!(stored.result.data.get(REMEDIATION_REVERIFY_KEY).is_none());
}

fn reverify_call(fix: &str, refusal: &str) -> WorkflowV2CallExecution {
    let mut contract = contract("verify", 2, true);
    contract[REVERIFY_CONTRACT_KEY] = json!({"fixCallId": fix, "refusalCallId": refusal});
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert("remediationContract".into(), contract);
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "verification-wave-review-verify-task-a-esc-5-moved".into(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options,
        },
        input: json!({}),
        depends_on: vec![],
    }
}

#[test]
fn a_reverify_is_answered_only_on_the_hosts_plan_for_this_sessions_fix() {
    let world = World::new();
    scene(
        &world,
        &[(RUN, "review-remediate-task-b-1-3", "src/b/methods.rs")],
    );
    assert_eq!(
        reverify_refusal(
            &reverify_call(FIX, REFUSAL),
            &world.store,
            None,
            Some(&world.repo)
        ),
        None
    );
    let wrong = reverify_refusal(
        &reverify_call(FIX, "someone-else"),
        &world.store,
        None,
        Some(&world.repo),
    );
    assert!(wrong.is_some_and(|why| why.contains("does not match")));
    let unmoved = World::new();
    scene(&unmoved, &[]);
    let refused = reverify_refusal(
        &reverify_call(FIX, REFUSAL),
        &unmoved.store,
        None,
        Some(&unmoved.repo),
    );
    assert!(refused.is_some_and(|why| why.contains("no re-verification plan")));
    // An ordinary verifier is no re-verification and is never judged here.
    let mut plain = reverify_call(FIX, REFUSAL);
    plain
        .call
        .options
        .extra
        .insert("remediationContract".into(), contract("verify", 2, true));
    assert_eq!(
        reverify_refusal(&plain, &unmoved.store, None, Some(&unmoved.repo)),
        None
    );
}

/// Only exact files count: a regular round (no escalation) is moved by a
/// landing in a file the refusal named that a task declares, never by one in
/// a shared-append file it did not name, nor under a directory it named.
#[test]
fn only_an_exactly_named_declared_file_moves_a_regular_round() {
    use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
    let universe = WorkflowV2TaskUniverse {
        schema_version: "t".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-B".into(),
            files_expected_to_change: vec!["src/b/methods.rs".into()],
            shared_append_target_files: vec!["src/z.rs".into()],
            ..Default::default()
        }],
    };
    for (landed, named, moves) in [
        ("src/b/methods.rs", "src/b/methods.rs", true),
        ("src/b/methods.rs", "src/b/", false),
        ("src/z.rs", "src/b/methods.rs", false),
        ("src/z.rs", "src/z.rs", true),
    ] {
        let world = World::new();
        let judged = world.land(RUN, "review-remediate-task-a-1-1", "src/a.rs");
        let mut refused = refusal(Some(&judged));
        refused.evidence[0].source = Some(named.into());
        world.save(REFUSAL, contract("verify", 1, false), false, refused);
        world.land(RUN, "review-remediate-task-b-1-3", landed);
        let baseline = world.baseline();
        world.manifest(
            "review-remediate-task-a-2-3",
            &baseline,
            "idempotent_noop",
            &[],
        );
        let fix = world.save(
            "review-remediate-task-a-2-3",
            contract("remediate", 2, false),
            true,
            no_patch(WorkflowV2Status::Accepted, false),
        );
        let plan = reverify_plan(&fix, &world.store, Some(&universe), Some(&world.repo));
        assert_eq!(plan.is_some(), moves, "{landed} named {named}: {plan:?}");
    }
}
