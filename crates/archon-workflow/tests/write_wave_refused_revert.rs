//! Batch L (L1) end to end: a remediation whose verifier refuses it leaves
//! nothing behind -- its landed commit and its project data are reverted by
//! the host, recorded, and given to the next attempt as a finding; a resume
//! reverts a refused landing an earlier host left in the tree before any
//! acceptance round; accepted work that landed after it is kept; and a
//! revert that would destroy accepted work fails closed as a finding.
//!
//! The real prelude, the real write wave (Git, project-input capture and
//! landing) and the live host's two revert points: right after a refusing
//! verdict is recorded, and at the start of every acceptance round
//! (`revert_refused_landings`, which `run_acceptance_stage` calls first).
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use archon_workflow::acceptance_scratch::ScratchPolicy;
use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::script::refused_landings::{
    RefusedLandingRevert, refused_landing_reverts, revert_refused_landings,
};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, at_head, run};
use serde_json::json;
use support::{Edits, Fixture, git};

const INPUT: &str = ".archon/lab/data";
const REGISTRY: &str = ".archon/lab/data/registry.json";
const A_FILE: &str = "crates/a/src/lib.rs";
const B_FILE: &str = "crates/b/src/lib.rs";
const A_FIX: &str = "review-remediate-task-a-1-1";
const A_VERDICT: &str = "verification-wave-review-verify-task-a-1-2";

/// A remediation as the acceptance stage routes it: its units name the
/// observation they fix (`observedBy`). The first round's failing check,
/// already observed.
fn script(rounds: u32, with_b: bool, b_targets: &[&str]) -> String {
    script_routed(rounds, with_b, b_targets, true)
}

fn script_routed(rounds: u32, with_b: bool, b_targets: &[&str], routed: bool) -> String {
    let observed = if routed {
        "observedBy: ['acceptance-contract-run-1'], "
    } else {
        ""
    };
    let mut findings = vec![json!({"id": "gate", "canonical_task_ids": ["TASK-A"],
        "severity": "high", "claim": "register only real data"})];
    if with_b {
        findings.push(json!({"id": "doc", "canonical_task_ids": ["TASK-B"],
            "severity": "medium", "claim": "document it"}));
    }
    format!(
        r#"export const meta = {{ name: 'refused', description: 'd', phases: [] }}
const tasks = [
  {{ id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['{A_FILE}'] }},
  {{ id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: {b} }},
]
const byId = (id) => tasks.find((t) => t.id === id) || {{}}
return await remediateFindings({findings}, {{ {observed}maxRounds: {rounds}, taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles }})
"#,
        b = json!(b_targets),
        findings = json!(findings),
    )
}

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

/// Two tasks' crates in the repository; the project's data registry is an
/// input the acceptance policy names. TASK-B may also own A's file.
fn fixture(scratch: &Path, b_owns: &[&str]) -> Fixture {
    let mut f = Fixture::new();
    std::fs::write(f.repo.join(".gitignore"), ".archon/*\n").unwrap();
    for (path, content) in [(A_FILE, "// a\n"), (B_FILE, "// b\n")] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crates"]);
    let registry = project_root(&f).join(REGISTRY);
    std::fs::create_dir_all(registry.parent().unwrap()).unwrap();
    std::fs::write(&registry, "seed\n").unwrap();
    let metadata = json!({"observer_snapshot": {"native_execution": {
        "policy": policy(&f, scratch), "source_commit": git(&f.repo, &["rev-parse", "HEAD"])}}});
    let path = f.store.run_dir(&f.run).join("v2/generated-metadata.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let task = |id: &str, owns: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![task("TASK-A", &[A_FILE]), task("TASK-B", b_owns)],
    });
    f
}

/// A's round 1 changes its code and registers an entry; its round 2 only
/// changes its code. B changes `B_FILE`, or A's file, and registers too.
fn edits(key: &str, round: u64, _escalated: bool) -> Edits {
    match (key, round) {
        ("TASK-A", 1) => Edits {
            files: vec![
                (A_FILE, "// A round 1\n"),
                (
                    "",
                    "\u{0}run:printf 'unverified\\n' >> .archon/lab/data/registry.json",
                ),
            ],
            report: vec![A_FILE],
            via_adapter: false,
        },
        ("TASK-A", _) => Edits {
            files: vec![(A_FILE, "// A round 2\n")],
            report: vec![A_FILE],
            via_adapter: false,
        },
        _ => Edits {
            files: vec![
                (B_FILE, "// B\n"),
                (
                    "",
                    "\u{0}run:printf 'b\\n' >> .archon/lab/data/registry.json",
                ),
            ],
            report: vec![B_FILE],
            via_adapter: false,
        },
    }
}

/// B rewrites A's own line, over A's refused change.
fn b_over_a(key: &str, round: u64, escalated: bool) -> Edits {
    if key == "TASK-B" {
        return Edits {
            files: vec![
                (A_FILE, "// B over A\n"),
                (
                    "",
                    "\u{0}run:printf 'b\\n' >> .archon/lab/data/registry.json",
                ),
            ],
            report: vec![A_FILE],
            via_adapter: false,
        };
    }
    edits(key, round, escalated)
}

fn host(f: Fixture, edits: fn(&str, u64, bool) -> Edits) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(f, store, Box::new(edits)))
}

fn registry(f: &Fixture) -> String {
    std::fs::read_to_string(project_root(f).join(REGISTRY)).unwrap()
}

fn reverts(f: &Fixture) -> Vec<RefusedLandingRevert> {
    refused_landing_reverts(&f.store.run_dir(&f.run)).unwrap()
}

fn head_subject(repo: &Path) -> String {
    git(repo, &["log", "-1", "--format=%s"])
}

fn into_fixture(host: Rc<Host>) -> Fixture {
    let Ok(host) = Rc::try_unwrap(host) else {
        panic!("session still referenced")
    };
    host.f
}

/// Both halves of the refused landing are out, each logged against the
/// verdict that refused it.
fn assert_a_reverted(f: &Fixture) {
    assert_eq!(at_head(&f.repo, A_FILE), "// a", "A's code is reverted");
    assert_eq!(registry(f), "seed\n", "A's registered entry is reverted");
    let log = reverts(f);
    let commit = log
        .iter()
        .find(|line| line.kind == "commit")
        .unwrap_or_else(|| panic!("no commit revert logged: {log:#?}"));
    assert_eq!(commit.outcome, "reverted", "{commit:#?}");
    assert_eq!(commit.fix_call_id, A_FIX);
    assert_eq!(commit.verdict_call_id, A_VERDICT);
    assert_eq!(commit.paths, vec![A_FILE.to_string()]);
    let revert = git(
        &f.repo,
        &["show", "-s", "--format=%s%n%b", &commit.revert_commit],
    );
    assert!(
        revert.starts_with(&format!(
            "archon: revert refused landing (run {}, stage {A_FIX})",
            f.run
        )) && revert.contains(&format!("Reverts-landing: {}", commit.landing)),
        "{revert}"
    );
    let landed = git(&f.repo, &["show", "-s", "--format=%s", &commit.landing]);
    assert!(
        landed.starts_with("archon: wave ") && landed.ends_with(&format!("stage {A_FIX})")),
        "{landed}"
    );
    let data = log
        .iter()
        .find(|line| line.kind == "project_input")
        .unwrap_or_else(|| panic!("no data revert logged: {log:#?}"));
    assert_eq!(data.outcome, "reverted", "{data:#?}");
    assert_eq!(data.paths, vec![REGISTRY.to_string()]);
    let inputs = std::fs::read_to_string(
        f.store
            .run_dir(&f.run)
            .join("write-coordination/project-inputs.jsonl"),
    )
    .unwrap();
    assert!(inputs.contains("\"outcome\":\"reverted\""), "{inputs}");
}

/// The live shape: acceptance round 1 fails a check TASK-A owns, TASK-A's
/// unit lands code and data, its verifier refuses, and acceptance goes on.
fn acceptance_script() -> String {
    format!(
        r#"export const meta = {{ name: 'refused', description: 'd', phases: [] }}
const tasks = [
  {{ id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['{A_FILE}'] }},
  {{ id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: ['{B_FILE}'] }},
]
const byId = (id) => tasks.find((t) => t.id === id) || {{}}
return await acceptance({{ maxRounds: 3, taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles }})
"#
    )
}

/// An acceptance round's reply: AC-1, owned by TASK-A, failed.
fn failing(round: u64) -> serde_json::Value {
    json!({ "round": round, "final": false, "passed": [], "operational_errors": [],
        "contract_present": true,
        "failing": [{ "check_id": "AC-1", "criterion": "every registered entry has data",
            "kind": "command", "status": "failed", "exit_code": 1,
            "owning_tasks": ["TASK-A"], "stderr_tail": "entry has no data" }] })
}

fn clean(round: u64) -> serde_json::Value {
    json!({ "round": round, "final": true, "failing": [], "passed": ["AC-1"],
        "operational_errors": [], "contract_present": true })
}

fn finished_ns(f: &Fixture, id: &str) -> i64 {
    let record = f.v2.load_call_record(id).unwrap().expect("recorded");
    chrono::DateTime::parse_from_rfc3339(&record.finished_at)
        .unwrap()
        .timestamp_nanos_opt()
        .unwrap()
}

#[tokio::test]
async fn a_refused_units_code_and_data_are_reverted_before_the_next_acceptance_round() {
    let temp = tempfile::tempdir().unwrap();
    let session = host(fixture(&temp.path().join("scratch"), &[B_FILE]), edits);
    session.verdicts("TASK-A", vec![Verdict::Refuse(vec![])]);
    session
        .acceptance
        .borrow_mut()
        .extend([failing(1), clean(2)]);
    run(&acceptance_script(), NEW_PRELUDE, session.clone()).await;
    assert_eq!(
        session
            .answers
            .borrow()
            .iter()
            .find(|(id, _)| id == A_FIX)
            .map(|(_, answer)| answer.clone()),
        Some(Answer::Ran)
    );
    let f = into_fixture(session);
    assert_a_reverted(&f);
    // Taken out before round 2 was answered, not after.
    let reverted_at = reverts(&f).iter().map(|line| line.at).max().unwrap();
    assert!(reverted_at < finished_ns(&f, "acceptance-contract-run-2"));
    // A later sweep finds nothing left to do.
    let head = git(&f.repo, &["rev-parse", "HEAD"]);
    let report = revert_refused_landings(&f.v2, Some(&f.repo));
    assert!(
        report.decisions.is_empty() && report.findings.is_empty(),
        "{report:?}"
    );
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), head);
}

#[tokio::test]
async fn the_next_attempt_starts_without_the_refused_change_and_is_given_the_refusal() {
    let temp = tempfile::tempdir().unwrap();
    let session = host(fixture(&temp.path().join("scratch"), &[B_FILE]), edits);
    session.verdicts("TASK-A", vec![Verdict::Refuse(vec![]), Verdict::Accept]);
    session
        .acceptance
        .borrow_mut()
        .extend([failing(1), failing(2), clean(3)]);
    run(&acceptance_script(), NEW_PRELUDE, session.clone()).await;
    let prompts = session.prompts.borrow().clone();
    assert_eq!(prompts.len(), 2, "two attempts dispatched: {prompts:#?}");
    let (second, prompt) = &prompts[1];
    assert_ne!(second, A_FIX);
    assert!(
        prompt.contains("Earlier Remediation Refused And Reverted")
            && prompt.contains(A_FIX)
            && prompt.contains(A_VERDICT),
        "{prompt}"
    );
    assert!(!prompts[0].1.contains("Earlier Remediation Refused"));
    let f = into_fixture(session);
    // The second attempt landed on the reverted tree and was accepted.
    assert_eq!(at_head(&f.repo, A_FILE), "// A round 1");
    assert_eq!(registry(&f), "seed\nunverified\n");
    let log = reverts(&f);
    assert!(
        log.iter()
            .all(|line| line.fix_call_id == A_FIX && line.outcome == "reverted"),
        "only the refused attempt was reverted: {log:#?}"
    );
}

#[tokio::test]
async fn a_resume_reverts_a_refused_landing_an_earlier_host_left_and_it_still_stands_for_replay() {
    let temp = tempfile::tempdir().unwrap();
    let first = host(fixture(&temp.path().join("scratch"), &[B_FILE]), edits);
    // The earlier host never reverted (or stopped before it could).
    first.revert_refused.set(false);
    first.verdicts("TASK-A", vec![Verdict::Refuse(vec![])]);
    run(&script(1, false, &[B_FILE]), NEW_PRELUDE, first.clone()).await;
    let f = into_fixture(first);
    assert_eq!(at_head(&f.repo, A_FILE), "// A round 1", "left in the tree");
    assert_eq!(registry(&f), "seed\nunverified\n");
    // The resumed session's first acceptance round sweeps, from a fresh
    // store, before any check runs.
    let resumed = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let report = revert_refused_landings(&resumed, Some(&f.repo));
    assert!(report.findings.is_empty(), "{report:?}");
    assert_a_reverted(&f);
    // The reverted fix's landing still stands in the run's own landing
    // order, so a later resume replays it rather than dispatching it again.
    let manifest: archon_workflow::write_coordinator::PatchManifest =
        serde_json::from_value(f.manifest(A_FIX, &format!("{A_FIX}-0"))).unwrap();
    archon_workflow::v2::branch_cache::landing::landing_holds(&f.repo, &manifest)
        .expect("the reverted landing still stands in the run's order");
}

#[tokio::test]
async fn accepted_work_landed_after_the_refused_unit_is_kept() {
    let temp = tempfile::tempdir().unwrap();
    let first = host(fixture(&temp.path().join("scratch"), &[B_FILE]), edits);
    first.revert_refused.set(false);
    first.verdicts("TASK-A", vec![Verdict::Refuse(vec![])]);
    first.verdicts("TASK-B", vec![Verdict::Accept]);
    run(&script(1, true, &[B_FILE]), NEW_PRELUDE, first.clone()).await;
    let f = into_fixture(first);
    assert_eq!(registry(&f), "seed\nunverified\nb\n");
    let report = revert_refused_landings(&f.v2, Some(&f.repo));
    // A's code goes; B's code stays. A's entry cannot go without B's (B
    // registered on top of it): that is a conflict, never an overwrite.
    assert_eq!(at_head(&f.repo, A_FILE), "// a");
    assert_eq!(
        at_head(&f.repo, B_FILE),
        "// B",
        "B's accepted code is kept"
    );
    assert_eq!(
        registry(&f),
        "seed\nunverified\nb\n",
        "B's accepted data is kept"
    );
    assert_eq!(report.findings.len(), 1, "{report:?}");
    assert!(
        report.findings[0].contains(REGISTRY)
            && report.findings[0].contains("a later change stands")
            && report.findings[0].contains(A_VERDICT),
        "{}",
        report.findings[0]
    );
}

#[tokio::test]
async fn a_revert_over_accepted_code_fails_closed_naming_the_conflict() {
    let temp = tempfile::tempdir().unwrap();
    let first = host(fixture(&temp.path().join("scratch"), &[A_FILE]), b_over_a);
    first.revert_refused.set(false);
    first.verdicts("TASK-A", vec![Verdict::Refuse(vec![])]);
    first.verdicts("TASK-B", vec![Verdict::Accept]);
    run(&script(1, true, &[A_FILE]), NEW_PRELUDE, first.clone()).await;
    let f = into_fixture(first);
    assert_eq!(at_head(&f.repo, A_FILE), "// B over A");
    let head = git(&f.repo, &["rev-parse", "HEAD"]);
    let report = revert_refused_landings(&f.v2, Some(&f.repo));
    assert_eq!(
        git(&f.repo, &["rev-parse", "HEAD"]),
        head,
        "nothing committed"
    );
    assert_eq!(at_head(&f.repo, A_FILE), "// B over A", "B's work is kept");
    let code = report
        .findings
        .iter()
        .find(|finding| finding.contains("a later landing that stands"))
        .unwrap_or_else(|| panic!("no code conflict: {report:?}"));
    assert!(code.contains(A_FIX) && code.contains(A_VERDICT), "{code}");
    assert!(head_subject(&f.repo).starts_with("archon: wave "));
    // Logged as a conflict, once, however often the sweep runs.
    revert_refused_landings(&f.v2, Some(&f.repo));
    let conflicts = reverts(&f)
        .into_iter()
        .filter(|line| line.kind == "commit" && line.outcome == "conflict")
        .count();
    assert_eq!(conflicts, 1);
}

/// Batch L3: a refused unit the acceptance stage did not route -- review,
/// residual or contest history -- is out of scope and left as it is.
#[tokio::test]
async fn a_refused_unit_outside_the_acceptance_stage_is_left_alone() {
    let temp = tempfile::tempdir().unwrap();
    let first = host(fixture(&temp.path().join("scratch"), &[B_FILE]), edits);
    first.verdicts("TASK-A", vec![Verdict::Refuse(vec![])]);
    run(
        &script_routed(1, false, &[B_FILE], false),
        NEW_PRELUDE,
        first.clone(),
    )
    .await;
    let f = into_fixture(first);
    let report = revert_refused_landings(&f.v2, Some(&f.repo));
    assert!(
        report.decisions.is_empty() && report.findings.is_empty(),
        "{report:?}"
    );
    assert_eq!(at_head(&f.repo, A_FILE), "// A round 1");
    assert_eq!(registry(&f), "seed\nunverified\n");
}

/// Batch L3: a landing whose verification a pause interrupted (the host stops before REM-13's
/// appended acceptance sweep) was never judged, so it is pending: not reverted, and no finding.
#[tokio::test]
async fn an_unjudged_landing_is_pending_and_not_reverted() {
    let temp = tempfile::tempdir().unwrap();
    let first = host(fixture(&temp.path().join("scratch"), &[B_FILE]), edits);
    first.revert_refused.set(false);
    first.verdicts("TASK-A", vec![Verdict::Refuse(vec![])]);
    run(&script(1, false, &[B_FILE]), NEW_PRELUDE, first.clone()).await;
    let f = into_fixture(first);
    let mut verdict = f.v2.load_call_record(A_VERDICT).unwrap().unwrap();
    verdict.result.data = json!({"interrupted": "paused"});
    f.v2.save_call_record(&verdict).unwrap();
    let report = revert_refused_landings(&f.v2, Some(&f.repo));
    assert!(
        report.decisions.is_empty() && report.findings.is_empty(),
        "{report:?}"
    );
    assert_eq!(at_head(&f.repo, A_FILE), "// A round 1");
    assert_eq!(registry(&f), "seed\nunverified\n");
}
