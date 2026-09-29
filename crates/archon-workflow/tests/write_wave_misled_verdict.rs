//! Batch M end to end: one remediation fix id executed three times. Its
//! first execution committed code and landed a sample file into the
//! project's data; the sample was taken out by hand before the last
//! execution was seeded; the last execution committed its own fix and kept
//! a tracked input in step. The verdict that followed was shown (by the
//! pre-Batch-M stamp) the long-gone sample as the unit's work, judged it not
//! legitimate and refused -- while judging what the project holds
//! legitimate.
//!
//! That verdict judged data its tree did not hold. The real refused-landing
//! pass (`revert_refused_landings`, over real Git and the real project-input
//! ledger) must revert nothing and name the verdict as misled; the verdict
//! reuse gate must not replay it. The same records with a verdict that
//! refuses what the project does hold still revert everything, and that
//! verdict still replays.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::path::{Path, PathBuf};

use archon_workflow::acceptance_scratch::ScratchPolicy;
use archon_workflow::v2::result_store::ReplayedFix;
use archon_workflow::v2::script::refused_landings::{
    refused_landing_reverts, revert_refused_landings,
};
use archon_workflow::v2::script::resume_verdict::{
    remediation_round_key, verdict_vouches_for_session_fix,
};
use archon_workflow::*;
use serde_json::{Value, json};
use support::{Fixture, git};

const INPUT: &str = ".archon/lab/data";
const GONE: &str = ".archon/lab/data/sets/sample/rows.csv";
const HELD: &str = ".archon/lab/data/spec.json";
const CODE: &str = "crates/a/src/lib.rs";
const FIX: &str = "review-remediate-task-a-1-1";
const VERIFY: &str = "verification-wave-review-verify-task-a-1-2";

fn project_root(f: &Fixture) -> PathBuf {
    PathBuf::from(
        project_artifact_context_from_v2_root(f.v2.root())
            .project_root
            .expect("the run has a project root"),
    )
}

fn policy(f: &Fixture, scratch: &Path) -> ScratchPolicy {
    let project = project_root(f).canonicalize().unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    ScratchPolicy {
        repository: f.repo.canonicalize().unwrap(),
        project: project.clone(),
        task_root: project.join("tasks"),
        scratch_parent: scratch.to_path_buf(),
        project_inputs: vec![PathBuf::from(INPUT)],
        project_input_excludes: vec![],
        combined: true,
        toolchain_path: std::env::join_paths([archon_shell::resolve_posix_shell()
            .parent()
            .unwrap()])
        .unwrap()
        .into_string()
        .unwrap(),
        environment: Default::default(),
        environment_allowlist: vec![],
        cargo_seed: None,
        timeout_secs: 60,
        output_bytes: 4096,
        scratch_bytes: 1 << 30,
        build_cache: None,
    }
}

fn hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap()
}

fn pause() {
    std::thread::sleep(std::time::Duration::from_millis(30));
}

fn run_dir(f: &Fixture) -> PathBuf {
    f.store.run_dir(&f.run)
}

fn log(f: &Fixture, path: &str, outcome: &str, after: &str) {
    let line = json!({ "stage_id": FIX, "item_id": format!("{FIX}-0"), "task_ids": ["TASK-A"],
        "path": path, "outcome": outcome, "before": "absent", "after": after, "at": now() });
    let ledger = run_dir(f).join("write-coordination/project-inputs.jsonl");
    std::fs::create_dir_all(ledger.parent().unwrap()).unwrap();
    let mut text = std::fs::read_to_string(&ledger).unwrap_or_default();
    text.push_str(&format!("{line}\n"));
    std::fs::write(ledger, text).unwrap();
}

/// A landing commit as the host writes one.
fn land(f: &Fixture, content: &str) -> String {
    std::fs::write(f.repo.join(CODE), content).unwrap();
    git(&f.repo, &["add", CODE]);
    let subject = format!("archon: wave 0 outputs (run {}, stage {FIX})", f.run);
    git(
        &f.repo,
        &[
            "-c",
            "user.name=archon-workflow",
            "-c",
            "user.email=archon-workflow@local",
            "commit",
            "-qm",
            &subject,
        ],
    );
    git(&f.repo, &["rev-parse", "HEAD"])
}

fn call(id: &str, stage: &str) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        json!({ "version": 1, "stage": stage, "taskId": "TASK-A", "round": 1, "maxRounds": 1,
            "sourceReduceCallIds": ["reduce"], "observedBy": ["acceptance-contract-run-1"] }),
    );
    let fix = stage == "remediate";
    WorkflowV2HostCall {
        id: id.into(),
        method: if fix {
            WorkflowV2HostMethod::Fanout
        } else {
            WorkflowV2HostMethod::Parallel
        },
        write_mode: fix.then_some(WorkflowV2WriteMode::Worktree),
        options,
    }
}

struct Run {
    f: Fixture,
    _scratch: tempfile::TempDir,
    last: String,
    records: Vec<WorkflowV2CallRecord>,
}

/// The three executions and the verdict answering `answers`.
fn three_executions(answers: Value) -> Run {
    let scratch = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    std::fs::write(f.repo.join(".gitignore"), ".archon/*\n").unwrap();
    std::fs::create_dir_all(f.repo.join(CODE).parent().unwrap()).unwrap();
    std::fs::write(f.repo.join(CODE), "// a\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crates"]);
    let project = project_root(&f);
    std::fs::create_dir_all(project.join(INPUT)).unwrap();
    let metadata = json!({"observer_snapshot": {"native_execution": {
        "policy": policy(&f, scratch.path()), "source_commit": git(&f.repo, &["rev-parse", "HEAD"])}}});
    let path = run_dir(&f).join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    // Execution 1: code, and a sample file landed as project data.
    land(&f, "// attempt 1\n");
    std::fs::create_dir_all(project.join(GONE).parent().unwrap()).unwrap();
    std::fs::write(project.join(GONE), "rows").unwrap();
    log(&f, GONE, "applied", &hash("rows"));
    pause();
    // Taken out by hand, with no record.
    std::fs::remove_file(project.join(GONE)).unwrap();
    pause();
    // Execution 3: seeded from the project as it now is, then landed.
    let seed = json!({ "project": project, "worktree": project, "inputs": [INPUT],
        "ignored_roots": [], "files": {} });
    let seed_path = run_dir(&f).join(format!(
        "write-coordination/stages/{FIX}/project-inputs/{FIX}-0/seed.json"
    ));
    std::fs::create_dir_all(seed_path.parent().unwrap()).unwrap();
    std::fs::write(&seed_path, seed.to_string()).unwrap();
    pause();
    let last = land(&f, "// attempt 3\n");
    std::fs::write(project.join(HELD), "held").unwrap();
    log(&f, HELD, "synced", &hash("held"));
    pause();
    let fix = WorkflowV2CallRecord::new(
        &f.run,
        call(FIX, "remediate"),
        3,
        "fix".into(),
        WorkflowV2Result::accepted("fixed"),
        vec![],
    );
    f.v2.save_call_record(&fix).unwrap();
    pause();
    let mut result = WorkflowV2Result::accepted("refused");
    result.status = WorkflowV2Status::NeedsReview;
    result.data = json!({ "items": [{ "data": { "project_data_landings": answers } }] });
    let mut verdict = WorkflowV2CallRecord::new(
        &f.run,
        call(VERIFY, "verify"),
        1,
        "v".into(),
        result,
        vec![],
    );
    verdict.status = WorkflowV2Status::NeedsReview;
    f.v2.save_call_record(&verdict).unwrap();
    let records = f.v2.load_call_records().unwrap();
    Run {
        f,
        _scratch: scratch,
        last,
        records,
    }
}

fn answer(path: &str, legitimate: bool) -> Value {
    json!({ "path": path, "legitimate": legitimate, "provenance": "judged" })
}

fn vouches(run: &Run) -> bool {
    let fix = run.records.iter().find(|r| r.call.id == FIX).unwrap();
    let verdict = run.records.iter().find(|r| r.call.id == VERIFY).unwrap();
    let key = remediation_round_key(&verdict.call).unwrap();
    // A resumed session: the records are the earlier sessions'.
    let resumed = WorkflowV2ResultStore::new(run.f.v2.root().to_path_buf());
    resumed.note_fix_lineage(
        &key,
        Some(ReplayedFix {
            call_id: FIX.into(),
            finished_at: fix.finished_at.clone(),
        }),
    );
    verdict_vouches_for_session_fix(verdict, &run.records, &resumed)
}

#[test]
fn a_refusal_of_only_data_gone_before_the_verdict_reverts_nothing_and_is_asked_again() {
    let run = three_executions(json!([answer(GONE, false), answer(HELD, true)]));
    let project = project_root(&run.f);
    let report = revert_refused_landings(&run.f.v2, Some(&run.f.repo));
    assert!(
        report.decisions.is_empty() && report.findings.is_empty(),
        "{report:?}"
    );
    assert_eq!(report.misled, vec![VERIFY.to_string()]);
    assert_eq!(git(&run.f.repo, &["rev-parse", "HEAD"]), run.last);
    assert_eq!(
        std::fs::read_to_string(run.f.repo.join(CODE)).unwrap(),
        "// attempt 3\n"
    );
    assert_eq!(std::fs::read_to_string(project.join(HELD)).unwrap(), "held");
    assert!(
        refused_landing_reverts(&run_dir(&run.f))
            .unwrap()
            .is_empty()
    );
    assert!(!vouches(&run), "a misled verdict is never replayed");
}

#[test]
fn a_refusal_of_data_the_project_holds_still_reverts_every_landing_and_replays() {
    let run = three_executions(json!([answer(GONE, false), answer(HELD, false)]));
    let project = project_root(&run.f);
    let report = revert_refused_landings(&run.f.v2, Some(&run.f.repo));
    assert!(report.misled.is_empty(), "{report:?}");
    assert!(report.findings.is_empty(), "{report:?}");
    let reverted: Vec<_> = (report.decisions.iter())
        .filter(|d| d.kind == "commit" && d.outcome == "reverted")
        .collect();
    assert_eq!(reverted.len(), 2, "{report:?}");
    assert_eq!(
        std::fs::read_to_string(run.f.repo.join(CODE)).unwrap(),
        "// a\n"
    );
    assert!(
        !project.join(HELD).exists(),
        "the held landing is taken out"
    );
    assert!(vouches(&run), "a real refusal replays as it was");
}

#[test]
fn a_refusal_naming_no_data_is_never_misled() {
    let run = three_executions(json!([answer(HELD, true)]));
    let report = revert_refused_landings(&run.f.v2, Some(&run.f.repo));
    assert!(report.misled.is_empty(), "{report:?}");
    assert!(
        report
            .decisions
            .iter()
            .any(|d| d.kind == "commit" && d.outcome == "reverted"),
        "{report:?}"
    );
}
