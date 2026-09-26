//! Issue-107 end to end: a remediation unit whose verifier refuses the fix
//! over a file another task owns gets ONE cross-owner round, through the real
//! prelude, the real write wave (Git, forbidden paths, task floors) and the
//! host's terminal rule -- and a resume from the deployed prelude replays
//! every call that already existed, so only the escalated round is new.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::collections::BTreeSet;
use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::v2::script::{
    AuthoredAcceptanceGateFact, AuthoredRunFacts, authored_call_facts,
    authored_run_terminal_status, writable_task_ids,
};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, OLD_PRELUDE, Verdict, at_head, run};
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

/// The file the verifier blames: TASK-C owns it, and TASK-C has no unit.
const C_TEST: &str = "crates/c/src/gate_tests.rs";

const SCRIPT: &str = r#"export const meta = { name: 'esc', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs', 'crates/a/src/r2.rs', 'crates/a/src/r3.rs'] },
  { id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: ['crates/b/src/lib.rs'] },
  { id: 'TASK-D', file: 'tasks/TASK-D.md', targetFiles: ['crates/d/src/lib.rs'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
return await remediateFindings(FINDINGS, { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
"#;

fn findings(cross: bool) -> Value {
    let mut list = vec![
        json!({"id": "gate", "canonical_task_ids": ["TASK-A"], "severity": "high", "claim": "the write path must fail closed"}),
        json!({"id": "doc", "canonical_task_ids": ["TASK-B"], "severity": "medium", "claim": "document the gate"}),
    ];
    if cross {
        list.push(json!({"id": "seam", "attributable_to_task": false, "canonical_task_ids": ["TASK-B", "TASK-D"],
            "severity": "medium", "claim": "the two tasks disagree at their seam"}));
    }
    Value::Array(list)
}

fn script() -> String {
    script_with(false)
}

fn script_with(cross: bool) -> String {
    SCRIPT.replace("FINDINGS", &findings(cross).to_string())
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
        ("crates/a/src/r3.rs", "// a r3\n"),
        ("crates/b/Cargo.toml", "[package]\nname = \"b\"\n"),
        ("crates/b/src/lib.rs", "// b\n"),
        ("crates/d/Cargo.toml", "[package]\nname = \"d\"\n"),
        ("crates/d/src/lib.rs", "// d\n"),
        (C_TEST, "// b tests\n"),
        ("crates/c/Cargo.toml", "[package]\nname = \"c\"\n"),
        ("crates/c/src/lib.rs", "// c\n"),
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
            // TASK-A forbids C's test file, which the escalated round must
            // lift, and B's file, which it must not.
            task(
                "TASK-A",
                &[
                    "crates/a/src/lib.rs",
                    "crates/a/src/r2.rs",
                    "crates/a/src/r3.rs",
                ],
                &[
                    "`crates/c/src/gate_tests.rs` (TASK-C scope)",
                    "`crates/b/src/lib.rs` (TASK-B scope)",
                ],
            ),
            task("TASK-B", &["crates/b/src/lib.rs"], &[]),
            task("TASK-C", &["crates/c/src/lib.rs", C_TEST], &[]),
            task("TASK-D", &["crates/d/src/lib.rs"], &[]),
        ],
    });
    f
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// Every fix lands something new, each round of A in its own file, and the
/// escalated round also changes C's test file.
fn edits(key: &str, round: u64, escalated: bool) -> Edits {
    let own = match (key, round) {
        ("TASK-A", 1) => "crates/a/src/lib.rs",
        ("TASK-A", 2) => "crates/a/src/r2.rs",
        ("TASK-A", _) => "crates/a/src/r3.rs",
        _ => "crates/b/src/lib.rs",
    };
    edits_with(
        vec![(own, leak(format!("// {key} round {round}\n")))],
        escalated,
    )
}

/// The resume scenario: A's round 2 lands nothing (it rewrites round 1's
/// line) and the escalated round changes only C's file, so no later write
/// touches a file an earlier fix recorded -- a recorded fix replays only on
/// the tree it left (dfa009787, Issue-102), which is a separate rule.
fn edits_resume(key: &str, _round: u64, escalated: bool) -> Edits {
    let (own, line) = match key {
        "TASK-A" => ("crates/a/src/lib.rs", "// TASK-A round 1\n"),
        "TASK-B" => ("crates/b/src/lib.rs", "// TASK-B round 1\n"),
        _ => ("crates/d/src/lib.rs", "// the seam, round 1\n"),
    };
    edits_with(vec![(own, line)], escalated)
}

fn edits_with(mut files: Vec<(&'static str, &'static str)>, escalated: bool) -> Edits {
    if escalated {
        files.push((C_TEST, "// c tests seed a registered fixture\n"));
    }
    Edits {
        report: files.iter().map(|(path, _)| *path).collect(),
        files,
        via_adapter: false,
    }
}

fn host(f: Fixture) -> Rc<Host> {
    host_with(f, edits)
}

fn host_with(f: Fixture, edits: fn(&str, u64, bool) -> Edits) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(f, store, Box::new(edits)))
}

/// The host's terminal status for the run as it ended.
fn terminal(host: &Host, remediation: &Value) -> WorkflowV2Status {
    terminal_for(host, remediation, false)
}

fn terminal_for(host: &Host, remediation: &Value, cross: bool) -> WorkflowV2Status {
    let calls = host.calls.borrow().clone();
    let facts = authored_call_facts(&calls, |id| host.store.load_call_record(id)).unwrap();
    let accounting = json!({"accepted": [], "blocked": [], "adversarial_findings": findings(cross),
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

fn ids(host: &Host) -> Vec<String> {
    host.answers
        .borrow()
        .iter()
        .map(|(id, _)| id.clone())
        .collect()
}

fn refuse(paths: &[&'static str]) -> Verdict {
    Verdict::Refuse(paths.to_vec())
}

#[tokio::test]
async fn a_blocker_in_another_tasks_file_is_fixed_by_one_widened_round_and_the_run_can_go_green() {
    let host = host(fixture());
    host.verdicts(
        "TASK-A",
        vec![refuse(&[C_TEST]), refuse(&[C_TEST]), Verdict::Accept],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    assert_eq!(
        ids(&host),
        [
            "review-remediate-task-a-1-1",
            "verification-wave-review-verify-task-a-1-2",
            "review-remediate-task-a-2-3",
            "verification-wave-review-verify-task-a-2-4",
            "review-remediate-task-a-esc-5",
            "verification-wave-review-verify-task-a-esc-6",
            "review-remediate-task-b-1-7",
            "verification-wave-review-verify-task-b-1-8",
        ]
    );
    // The widened write landed in C's file, which TASK-A alone forbids.
    assert_eq!(
        at_head(&host.f.repo, C_TEST),
        "// c tests seed a registered fixture"
    );
    assert_eq!(
        at_head(&host.f.repo, "crates/a/src/r3.rs"),
        "// TASK-A round 3"
    );
    let prompts = host.prompts.borrow();
    let (_, escalated) = prompts
        .iter()
        .find(|(id, _)| id == "review-remediate-task-a-esc-5")
        .expect("the escalated fix was dispatched");
    assert!(
        escalated.contains("ESCALATED cross-owner round"),
        "{escalated}"
    );
    assert!(
        escalated.contains("PRIOR VERIFIER'S JUDGMENT"),
        "{escalated}"
    );
    assert!(
        escalated.contains("the write path must fail closed"),
        "{escalated}"
    );
    // Lifted only inside the involved tasks' declared paths: B stays frozen.
    assert!(
        escalated.contains("declared targets take precedence): crates/b/src/lib.rs."),
        "{escalated}"
    );
    // The owner contributes its blocker file, never its whole scope.
    let declared = escalated
        .split("\"_declared_targets\":[")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .expect("the tool-guard target stamp");
    assert!(declared.contains(C_TEST), "{declared}");
    assert!(!declared.contains("crates/c/src/lib.rs"), "{declared}");
    assert!(
        host.answers
            .borrow()
            .iter()
            .all(|(_, answer)| !matches!(answer, Answer::Refused(_))),
        "the host's own plan is never refused"
    );
    let (_, round_one) = &prompts[0];
    assert!(
        round_one.contains(C_TEST),
        "round 1 was told C's file is forbidden"
    );
    drop(prompts);
    let verify = host.calls.borrow()[5].clone();
    let contract = &verify.options.extra["remediationContract"];
    assert_eq!(contract["round"], 3);
    assert_eq!(contract["escalation"]["ownerTaskIds"], json!(["TASK-C"]));
    let record = host.store.load_call_record(&verify.id).unwrap().unwrap();
    assert_eq!(
        record.dispatched_items[0].canonical_task_ids,
        ["TASK-A", "TASK-C"],
        "the escalated verifier judges both tasks"
    );
    let a = result["resolved"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["taskId"] == "TASK-A");
    assert_eq!(
        a.expect("A resolved")["escalatedTo"],
        json!(["TASK-C"]),
        "{result}"
    );
    assert_eq!(terminal(&host, &result), WorkflowV2Status::Accepted);
}

#[tokio::test]
async fn a_blocker_no_other_task_owns_changes_nothing() {
    let host = host(fixture());
    // Its own file, and a file no task declares: neither buys a round.
    let own_and_unowned = refuse(&["crates/a/src/lib.rs", "crates/z/src/lib.rs"]);
    host.verdicts("TASK-A", vec![own_and_unowned.clone(), own_and_unowned]);
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    assert!(
        ids(&host).iter().all(|id| !id.contains("-esc-")),
        "{:?}",
        ids(&host)
    );
    assert_eq!(ids(&host).len(), 6, "{:?}", ids(&host));
    assert_eq!(result["unresolved"][0]["outcome"], "unverified");
    assert!(result["unresolved"][0].get("escalatedTo").is_none());
    assert_eq!(terminal(&host, &result), WorkflowV2Status::NeedsReview);
}

#[tokio::test]
async fn a_second_refusal_after_escalation_ends_unverified_with_no_further_round() {
    let host = host(fixture());
    host.verdicts(
        "TASK-A",
        vec![refuse(&[C_TEST]), refuse(&[C_TEST]), refuse(&[C_TEST])],
    );
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    let a_calls: Vec<String> = ids(&host)
        .into_iter()
        .filter(|id| id.contains("task-a"))
        .collect();
    assert_eq!(a_calls.len(), 6, "{a_calls:?}");
    assert!(a_calls[5].contains("-esc-"), "{a_calls:?}");
    let a = &result["unresolved"][0];
    assert_eq!(
        (a["taskId"].as_str(), a["outcome"].as_str()),
        (Some("TASK-A"), Some("unverified"))
    );
    assert_eq!(a["escalatedTo"], json!(["TASK-C"]));
    assert_eq!(terminal(&host, &result), WorkflowV2Status::NeedsReview);
}

/// Session 1 is the deployed prelude (dfa009787): A's round 1 is refused
/// over C's file, round 2 lands nothing, A ends open and B's unit runs after
/// it. Session 2 is this prelude over the same run directory: every call
/// session 1 made keeps its label and input identity and replays -- A's
/// rounds under their own ids (round 1's verdict as history), B's unit and
/// the cross-task unit under ordinals SHIFTED by the two new calls, by
/// drift -- and the round-1 verdict
/// replayed as history still carries the plan, so the escalated round is the
/// only work dispatched.
#[tokio::test]
async fn a_resume_from_the_deployed_prelude_replays_every_existing_call() {
    let first = host_with(fixture(), edits_resume);
    first.verdicts("TASK-A", vec![refuse(&[C_TEST])]);
    let before = run(&script_with(true), OLD_PRELUDE, first.clone()).await;
    assert_eq!(before["unresolved"][0]["taskId"], "TASK-A", "{before}");
    let recorded = [
        "review-remediate-task-a-1-1",
        "verification-wave-review-verify-task-a-1-2",
        "review-remediate-task-a-2-3",
        "review-verify-task-a-2-no-patch",
        "review-remediate-task-b-1-5",
        "verification-wave-review-verify-task-b-1-6",
        "review-remediate-cross-task-b-task-d-1-7",
        "verification-wave-review-verify-cross-task-b-task-d-1-8",
    ];
    assert_eq!(ids(&first), recorded);
    let Ok(first) = Rc::try_unwrap(first) else {
        panic!("session 1 still referenced")
    };
    let second = host_with(first.f, edits_resume);
    second.verdicts("TASK-A", vec![Verdict::Accept]);
    let after = run(&script_with(true), NEW_PRELUDE, second.clone()).await;
    let answers = second.answers.borrow().clone();
    let expected = [
        ("review-remediate-task-a-1-1", Answer::Replayed),
        (
            "verification-wave-review-verify-task-a-1-2",
            Answer::Replayed,
        ),
        ("review-remediate-task-a-2-3", Answer::Replayed),
        ("review-verify-task-a-2-no-patch", Answer::Checkpoint),
        ("review-remediate-task-a-esc-5", Answer::Ran),
        ("verification-wave-review-verify-task-a-esc-6", Answer::Ran),
        ("review-remediate-task-b-1-7", Answer::Replayed),
        (
            "verification-wave-review-verify-task-b-1-8",
            Answer::Replayed,
        ),
        ("review-remediate-cross-task-b-task-d-1-9", Answer::Replayed),
        (
            "verification-wave-review-verify-cross-task-b-task-d-1-10",
            Answer::Replayed,
        ),
    ];
    assert_eq!(
        answers,
        expected.map(|(id, answer)| (id.to_string(), answer)),
        "{answers:#?}"
    );
    assert_eq!(
        at_head(&second.f.repo, C_TEST),
        "// c tests seed a registered fixture"
    );
    let a = after["resolved"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["taskId"] == "TASK-A");
    assert_eq!(
        a.expect("A resolved")["escalatedTo"],
        json!(["TASK-C"]),
        "{after}"
    );
    assert_eq!(
        terminal_for(&second, &after, true),
        WorkflowV2Status::Accepted
    );
}

/// An escalated round that lands nothing at the review stage leaves the
/// refusal that bought it standing: the unit is open and the run holds.
#[tokio::test]
async fn an_escalated_round_that_lands_nothing_keeps_the_unit_blocking() {
    fn nothing_escalated(key: &str, round: u64, escalated: bool) -> Edits {
        if escalated {
            // Rewrites round 2's line: no patch lands.
            return edits_with(vec![("crates/a/src/r2.rs", "// TASK-A round 2\n")], false);
        }
        edits(key, round, false)
    }
    let host = host_with(fixture(), nothing_escalated);
    host.verdicts("TASK-A", vec![refuse(&[C_TEST]), refuse(&[C_TEST])]);
    let result = run(&script(), NEW_PRELUDE, host.clone()).await;
    assert!(
        ids(&host).contains(&"review-verify-task-a-3-no-patch".to_string()),
        "{:?}",
        ids(&host)
    );
    let a = &result["unresolved"][0];
    assert_eq!(
        (a["taskId"].as_str(), a["outcome"].as_str()),
        (Some("TASK-A"), Some("unverified"))
    );
    assert!(
        a["reason"].as_str().unwrap().contains("refusal stands"),
        "{result}"
    );
    assert_eq!(
        at_head(&host.f.repo, C_TEST),
        "// b tests",
        "C's file untouched"
    );
    assert_eq!(terminal(&host, &result), WorkflowV2Status::NeedsReview);
}

/// The live shape: the deployed prelude recorded two refused rounds, each
/// landing, round 2 over a file round 1's manifest covers. This prelude's
/// resume replays both fixes (the round-2 landing is the run's own, Issue-108)
/// and round 2's refusal as the history the escalation is bought with, so
/// the escalated round is the only work dispatched.
#[tokio::test]
async fn a_refused_last_round_that_buys_the_escalation_replays_on_resume() {
    let first = host(fixture());
    first.verdicts("TASK-A", vec![refuse(&[C_TEST]), refuse(&[C_TEST])]);
    run(&script(), OLD_PRELUDE, first.clone()).await;
    let Ok(first) = Rc::try_unwrap(first) else {
        panic!("session 1 still referenced")
    };
    let second = host(first.f);
    second.verdicts("TASK-A", vec![Verdict::Accept]);
    let after = run(&script(), NEW_PRELUDE, second.clone()).await;
    let ran: Vec<String> = second
        .answers
        .borrow()
        .iter()
        .filter(|(_, answer)| *answer != Answer::Replayed)
        .map(|(id, _)| id.clone())
        .collect();
    assert_eq!(
        ran,
        [
            "review-remediate-task-a-esc-5",
            "verification-wave-review-verify-task-a-esc-6"
        ],
        "{:#?}",
        second.answers.borrow()
    );
    assert_eq!(terminal(&second, &after), WorkflowV2Status::Accepted);
}
