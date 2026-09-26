//! Issue-111 end to end: a remediation round that lands no patch after a
//! refusal gets one read-only re-verification when the run's own landings
//! changed what the refusal judged -- through the real prelude, the real
//! write wave (Git, manifests, landing commits) and the host's terminal
//! rule -- and keeps the refusal when nothing moved.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::script::{
    AuthoredAcceptanceGateFact, AuthoredRunFacts, authored_call_facts,
    authored_run_terminal_status, writable_task_ids,
};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, OLD_PRELUDE, Verdict, run};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

/// The deployed prelude (deploy-107, 1b6e3c2ca): escalation, no re-verify.
const DEPLOYED_PRELUDE: &str = include_str!("fixtures/v3_primitives_1b6e3c2ca.js");
/// TASK-C's test file, which TASK-A's verifier blames and TASK-C's own
/// remediation fixes.
const C_TEST: &str = "crates/c/src/gate_tests.rs";
const C_FIXED: &str = "// c tests seed a registered fixture\n";

const SCRIPT: &str = r#"export const meta = { name: 'reverify', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs', 'crates/a/src/r2.rs'] },
  { id: 'TASK-C', file: 'tasks/TASK-C.md', targetFiles: ['crates/c/src/lib.rs', 'crates/c/src/gate_tests.rs'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
return await remediateFindings(FINDINGS, { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
"#;

fn findings() -> Value {
    json!([
        {"id": "gate", "canonical_task_ids": ["TASK-A"], "severity": "high", "claim": "the write path must fail closed"},
        {"id": "fixture", "canonical_task_ids": ["TASK-C"], "severity": "high", "claim": "the gate tests must seed a registered fixture"},
    ])
}

fn script() -> String {
    SCRIPT.replace("FINDINGS", &findings().to_string())
}

fn task(id: &str, owns: &[&str], forbids: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    for (path, content) in [
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        ("crates/a/src/lib.rs", "// a\n"),
        ("crates/a/src/r2.rs", "// a r2\n"),
        ("crates/c/Cargo.toml", "[package]\nname = \"c\"\n"),
        ("crates/c/src/lib.rs", "// c\n"),
        (C_TEST, "// c tests\n"),
    ] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crates"]);
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task(
                "TASK-A",
                &["crates/a/src/lib.rs", "crates/a/src/r2.rs"],
                &["`crates/c/src/gate_tests.rs` (TASK-C scope)"],
            ),
            task("TASK-C", &["crates/c/src/lib.rs", C_TEST], &[]),
        ],
    });
    f
}

fn edits_with(files: Vec<(&'static str, &'static str)>) -> Edits {
    Edits {
        report: files.iter().map(|(path, _)| *path).collect(),
        files,
        via_adapter: false,
    }
}

/// A lands each regular round in its own file; the escalated round finds
/// C's fix already there and rewrites it byte for byte, landing nothing. C's
/// fix lands in its test file, or -- `c_fixes_the_blocker` false -- only in
/// its library, leaving the file A was refused over untouched.
fn edits(c_fixes_the_blocker: bool) -> impl Fn(&str, u64, bool) -> Edits {
    move |key, round, escalated| match (key, round, escalated) {
        ("TASK-A", _, true) => edits_with(vec![
            ("crates/a/src/r2.rs", "// TASK-A round 2\n"),
            (
                C_TEST,
                if c_fixes_the_blocker {
                    C_FIXED
                } else {
                    "// c tests\n"
                },
            ),
        ]),
        ("TASK-A", 1, _) => edits_with(vec![("crates/a/src/lib.rs", "// TASK-A round 1\n")]),
        ("TASK-A", _, _) => edits_with(vec![("crates/a/src/r2.rs", "// TASK-A round 2\n")]),
        _ if c_fixes_the_blocker => edits_with(vec![(C_TEST, C_FIXED)]),
        _ => edits_with(vec![("crates/c/src/lib.rs", "// c fixed\n")]),
    }
}

fn host(f: Fixture, edits: impl Fn(&str, u64, bool) -> Edits + 'static) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(f, store, Box::new(edits)))
}

fn next(host: Rc<Host>, edits: impl Fn(&str, u64, bool) -> Edits + 'static) -> Rc<Host> {
    let Ok(host) = Rc::try_unwrap(host) else {
        panic!("the session is still referenced")
    };
    self::host(host.f, edits)
}

fn terminal(host: &Host, remediation: &Value) -> WorkflowV2Status {
    let calls = host.calls.borrow().clone();
    let facts = authored_call_facts(&calls, |id| host.store.load_call_record(id)).unwrap();
    let accounting = json!({"accepted": [], "blocked": [], "adversarial_findings": findings(),
        "uncovered_requirements": [], "review_remediation": remediation})
    .to_string();
    let universe = host.f.universe.as_ref().unwrap();
    let universe_tasks: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|t| t.canonical_task_id.clone())
        .collect();
    let outcome = authored_run_terminal_status(&AuthoredRunFacts {
        accumulated_status: WorkflowV2Status::NeedsReview,
        host_terminal_failure: None,
        script_result: Some(&accounting),
        acceptance_gate: AuthoredAcceptanceGateFact::NotRequired,
        calls: &facts,
        writable_tasks: &writable_task_ids(Some(universe)),
        universe_tasks: &universe_tasks,
    });
    eprintln!("{}", outcome.explanation());
    outcome.status
}

fn not_replayed(host: &Host) -> Vec<(String, Answer)> {
    host.answers
        .borrow()
        .iter()
        .filter(|(_, answer)| *answer != Answer::Replayed)
        .cloned()
        .collect()
}

fn refuse() -> Verdict {
    Verdict::Refuse(vec![C_TEST])
}

fn entry<'a>(result: &'a Value, field: &str, task: &str) -> Option<&'a Value> {
    result[field]
        .as_array()?
        .iter()
        .find(|entry| entry["taskId"] == task)
}

/// The live shape (wf-0ddadd81, TASK-TRADING-012/013). Session 1 ran before
/// escalation existed: A's two rounds were refused over C's file, then C's
/// own round fixed that file. Session 2 (deploy-107) replayed them and spent
/// the escalated round, which found the blocker gone and landed nothing, so
/// the refusal held A open. Session 3 (this prelude) replays every call
/// session 2 made -- the escalated fix as the idempotent no-op it was -- and
/// the only new work is ONE read-only verifier of the tree as it is now,
/// whose verdict resolves A.
#[tokio::test]
async fn a_no_patch_escalated_round_on_a_tree_another_task_fixed_is_reverified_on_resume() {
    let first = host(fixture(), edits(true));
    first.verdicts("TASK-A", vec![refuse(), refuse()]);
    run(&script(), OLD_PRELUDE, first.clone()).await;
    assert_eq!(harness::at_head(&first.f.repo, C_TEST), C_FIXED.trim_end());

    let second = next(first, edits(true));
    let held = run(&script(), DEPLOYED_PRELUDE, second.clone()).await;
    let a = entry(&held, "unresolved", "TASK-A").expect("deploy-107 holds A");
    assert!(
        a["reason"].as_str().unwrap().contains("refusal stands"),
        "{held}"
    );
    assert_eq!(terminal(&second, &held), WorkflowV2Status::NeedsReview);
    let escalated = second
        .answers
        .borrow()
        .iter()
        .find(|(id, _)| id.starts_with("review-remediate-task-a-esc-"))
        .cloned()
        .expect("deploy-107 spent the escalated round");
    assert_eq!(escalated.1, Answer::Ran);
    let manifest = second
        .f
        .manifest(&escalated.0, &format!("{}-0", escalated.0));
    assert_eq!(
        manifest["status"]["status"], "idempotent_noop",
        "{manifest}"
    );

    let third = next(second, edits(true));
    third.verdicts("TASK-A", vec![Verdict::Accept]);
    let after = run(&script(), NEW_PRELUDE, third.clone()).await;
    let reverify = format!(
        "verification-wave-review-verify-task-a-esc-{}-moved",
        escalated.0.rsplit('-').next().unwrap()
    );
    let fresh = not_replayed(&third);
    assert_eq!(
        fresh,
        [
            (
                "review-verify-task-a-3-no-patch".to_string(),
                Answer::Checkpoint
            ),
            (reverify.clone(), Answer::Ran),
        ],
        "{:#?}",
        third.answers.borrow()
    );
    assert!(
        third
            .answers
            .borrow()
            .iter()
            .any(|(id, answer)| *id == escalated.0 && *answer == Answer::Replayed),
        "the escalated fix replays under its own id"
    );
    let record = third.store.load_call_record(&reverify).unwrap().unwrap();
    let contract = &record.call.options.extra["remediationContract"];
    assert_eq!(contract["reverify"]["fixCallId"], escalated.0);
    assert_eq!(contract["round"], 3);
    assert_eq!(
        record.dispatched_items[0].canonical_task_ids,
        ["TASK-A", "TASK-C"]
    );
    let prompt = record.call.options.task.clone().unwrap_or_default();
    assert!(prompt.contains("THIS ROUND LANDED NO PATCH"), "{prompt}");
    assert!(prompt.contains(C_TEST), "{prompt}");
    let a = entry(&after, "resolved", "TASK-A").unwrap_or_else(|| panic!("A resolved: {after}"));
    assert_eq!(a["escalatedTo"], json!(["TASK-C"]));
    assert_eq!(terminal(&third, &after), WorkflowV2Status::Accepted);

    // And a fourth session replays everything, the re-verification included.
    let fourth = next(third, edits(true));
    let again = run(&script(), NEW_PRELUDE, fourth.clone()).await;
    assert_eq!(
        not_replayed(&fourth),
        [(
            "review-verify-task-a-3-no-patch".to_string(),
            Answer::Checkpoint
        )],
        "{:#?}",
        fourth.answers.borrow()
    );
    assert_eq!(terminal(&fourth, &again), WorkflowV2Status::Accepted);
}

/// The same run, but C's fix never touched the file A was refused over:
/// nothing the refusal judged moved, so no verifier is asked and the refusal
/// holds A open -- the rule cannot loop and cannot mint a verdict.
#[tokio::test]
async fn a_no_patch_escalated_round_on_an_unmoved_tree_keeps_the_refusal() {
    let first = host(fixture(), edits(false));
    first.verdicts("TASK-A", vec![refuse(), refuse()]);
    run(&script(), OLD_PRELUDE, first.clone()).await;
    let second = next(first, edits(false));
    let after = run(&script(), NEW_PRELUDE, second.clone()).await;
    assert!(
        second
            .answers
            .borrow()
            .iter()
            .all(|(id, _)| !id.ends_with("-moved")),
        "{:#?}",
        second.answers.borrow()
    );
    let a = entry(&after, "unresolved", "TASK-A").expect("A stays open");
    assert!(
        a["reason"].as_str().unwrap().contains("refusal stands"),
        "{after}"
    );
    assert_eq!(terminal(&second, &after), WorkflowV2Status::NeedsReview);
}

/// Not only an escalated round: a regular round whose fix lands nothing
/// after a refusal, on a tree another task's landing changed in between (a
/// concurrent landing, simulated here by the host committing C's fix while
/// round 2 is prepared), is re-verified in that round -- and a refused
/// re-verification is the latest refusal, so the budget still ends it.
#[tokio::test]
async fn a_regular_no_patch_round_is_reverified_when_another_landing_moved_the_blocker() {
    let fixture = fixture();
    let repo: PathBuf = fixture.repo.clone();
    let run_id = fixture.run.clone();
    let concurrent = move |key: &str, round: u64, escalated: bool| {
        if (key, round, escalated) == ("TASK-A", 2, false) {
            std::fs::write(repo.join(C_TEST), C_FIXED).unwrap();
            git(&repo, &["add", C_TEST]);
            git(
                &repo,
                &[
                    "-c",
                    "user.name=archon-workflow",
                    "commit",
                    "-qm",
                    &format!(
                        "archon: wave 0 outputs (run {run_id}, stage review-remediate-task-c-1-99)"
                    ),
                ],
            );
            // Round 2 rewrites round 1's line: no patch.
            return edits_with(vec![("crates/a/src/lib.rs", "// TASK-A round 1\n")]);
        }
        edits(false)(key, round, escalated)
    };
    let host = host(fixture, concurrent);
    host.verdicts("TASK-A", vec![refuse(), Verdict::Accept]);
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let ids: Vec<String> = host
        .answers
        .borrow()
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    assert!(
        ids.contains(&"verification-wave-review-verify-task-a-2-3-moved".to_string()),
        "{ids:?}"
    );
    assert!(ids.iter().all(|id| !id.contains("-esc-")), "{ids:?}");
    let a = entry(&result, "resolved", "TASK-A").unwrap_or_else(|| panic!("A resolved: {result}"));
    assert!(a.get("escalatedTo").is_none());
    assert_eq!(terminal(&host, &result), WorkflowV2Status::Accepted);
}

/// A re-verification the host has no plan for -- a hand-made contract -- is
/// refused at dispatch, recorded nowhere and resolves nothing.
#[tokio::test]
async fn a_forged_reverification_is_refused_and_resolves_nothing() {
    const FORGED: &str = r#"export const meta = { name: 'forged', description: 'd', phases: [] }
const contract = { version: 1, stage: 'verify', taskId: 'TASK-A', round: 1, maxRounds: 2,
  sourceReduceCallIds: ['adversarial-review-reduce', 'coverage-audit-reduce'],
  reverify: { fixCallId: 'review-remediate-task-a-1-1', refusalCallId: 'none' } }
const check = await w.parallel('verification-wave-review-verify-task-a-1-9-moved',
  [{ item_id: 'x-check', canonical_task_ids: ['TASK-A'], task: 'accept', verification_requirements: ['accept'] }],
  { itemKind: 'focused_verification', task: 'accept', remediationContract: contract })
return check
"#;
    let host = host(fixture(), edits(false));
    let answer = run(FORGED, NEW_PRELUDE, host.clone()).await;
    assert_eq!(answer["status"], "failed", "{answer}");
    let answers = host.answers.borrow();
    assert!(
        matches!(&answers[0].1, Answer::Refused(why) if why.contains("no fix of its round")),
        "{answers:#?}"
    );
    assert!(
        host.store
            .load_call_record("verification-wave-review-verify-task-a-1-9-moved")
            .unwrap()
            .is_none()
    );
}
