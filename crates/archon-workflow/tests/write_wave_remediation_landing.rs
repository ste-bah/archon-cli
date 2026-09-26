//! Issue-108 end to end: a resume replays a recorded remediation landing that
//! the run's OWN later landing overwrote, exactly as the uninterrupted run
//! went on from it, and re-runs one whose file changed outside the run.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::rc::Rc;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, at_head, run};
use support::{Edits, Fixture, git};

const SCRIPT: &str = r#"export const meta = { name: 'landing', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['crates/a/src/lib.rs'] },
  { id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: ['crates/b/src/lib.rs'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
return await remediateFindings([
  { id: 'gate', canonical_task_ids: ['TASK-A'], severity: 'high', claim: 'fail closed' },
  { id: 'doc', canonical_task_ids: ['TASK-B'], severity: 'medium', claim: 'document it' },
], { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
"#;

fn fixture() -> Fixture {
    let mut f = Fixture::new();
    for (path, content) in [
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        ("crates/a/src/lib.rs", "// a\n"),
        ("crates/b/Cargo.toml", "[package]\nname = \"b\"\n"),
        ("crates/b/src/lib.rs", "// b\n"),
    ] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crates"]);
    let task = |id: &str, owns: &str| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: vec![owns.into()],
        ..Default::default()
    };
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-A", "crates/a/src/lib.rs"),
            task("TASK-B", "crates/b/src/lib.rs"),
        ],
    });
    f
}

/// Both of A's rounds rewrite the SAME file.
fn edits(key: &str, round: u64, _escalated: bool) -> Edits {
    let path = if key == "TASK-A" {
        "crates/a/src/lib.rs"
    } else {
        "crates/b/src/lib.rs"
    };
    let line: &'static str = Box::leak(format!("// {key} round {round}\n").into_boxed_str());
    Edits {
        files: vec![(path, line)],
        report: vec![path],
        via_adapter: false,
    }
}

fn host(f: Fixture) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    Rc::new(Host::new(f, store, Box::new(edits)))
}

/// Session 1: A's round 1 lands and is refused, round 2 overwrites the same
/// file and is accepted, then B's unit runs.
async fn first_session() -> Fixture {
    let first = host(fixture());
    first.verdicts("TASK-A", vec![Verdict::Refuse(vec![]), Verdict::Accept]);
    let result = run(SCRIPT, NEW_PRELUDE, first.clone()).await;
    assert_eq!(result["resolved"].as_array().unwrap().len(), 2, "{result}");
    assert_eq!(
        at_head(&first.f.repo, "crates/a/src/lib.rs"),
        "// TASK-A round 2"
    );
    let Ok(first) = Rc::try_unwrap(first) else {
        panic!("session 1 still referenced")
    };
    first.f
}

fn answers(host: &Host) -> Vec<(String, Answer)> {
    host.answers.borrow().clone()
}

#[tokio::test]
async fn a_landing_the_runs_next_round_overwrote_replays_on_resume() {
    let second = host(first_session().await);
    let result = run(SCRIPT, NEW_PRELUDE, second.clone()).await;
    let expected = [
        "review-remediate-task-a-1-1",
        "verification-wave-review-verify-task-a-1-2",
        "review-remediate-task-a-2-3",
        "verification-wave-review-verify-task-a-2-4",
        "review-remediate-task-b-1-5",
        "verification-wave-review-verify-task-b-1-6",
    ]
    .map(|id| (id.to_string(), Answer::Replayed));
    assert_eq!(answers(&second), expected, "{:#?}", answers(&second));
    assert_eq!(result["resolved"].as_array().unwrap().len(), 2, "{result}");
    assert_eq!(
        at_head(&second.f.repo, "crates/a/src/lib.rs"),
        "// TASK-A round 2"
    );
}

#[tokio::test]
async fn a_file_changed_outside_the_run_after_its_landings_reruns_the_fix() {
    let f = first_session().await;
    std::fs::write(f.repo.join("crates/a/src/lib.rs"), "// edited by hand\n").unwrap();
    git(&f.repo, &["commit", "-qam", "outside edit"]);
    let second = host(f);
    run(SCRIPT, NEW_PRELUDE, second.clone()).await;
    let answers = answers(&second);
    assert_eq!(
        answers[0],
        ("review-remediate-task-a-1-1".to_string(), Answer::Ran),
        "{answers:#?}"
    );
    // B's landing is untouched by the edit and still replays (by drift: A's
    // re-run was accepted in round 1, so B's ordinals moved).
    assert!(
        answers
            .iter()
            .any(|(id, answer)| id.starts_with("review-remediate-task-b-1-")
                && *answer == Answer::Replayed),
        "{answers:#?}"
    );
}
