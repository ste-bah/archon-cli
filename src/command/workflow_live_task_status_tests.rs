//! The dependency-cycle diagnostic's file paths, and what `status:` causes.

use super::*;

use std::collections::BTreeSet;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

/// A task file in the standard shape, with a chosen `status:`.
fn task_file(task_id: &str, status: &str, depends_on: &str) -> String {
    format!(
        "# {task_id}\n\n```yaml\ntask_id: {task_id}\ntitle: Fixture {task_id}\n\
         complexity: medium\nstatus: {status}\ndepends_on: {depends_on}\nblocks: []\n\
         required_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n"
    )
}

fn universe_of(
    files: &[(&str, String)],
) -> (tempfile::TempDir, WorkflowResult<WorkflowV2TaskUniverse>) {
    let temp = tempfile::tempdir().expect("tempdir");
    for (name, contents) in files {
        fs::write(temp.path().join(name), contents).expect("write task file");
    }
    let extracted = extract_task_universe_for_generated_run(&format!(
        "Implement the decomposed PRD at {}",
        temp.path().display()
    ))
    .map(|universe| universe.expect("a decomposed-PRD task set produces a universe"));
    (temp, extracted)
}

// ---------------------------------------------------------------------------
// The cycle diagnostic
// ---------------------------------------------------------------------------

/// Every other failure in the universe builder names the file the reader has to
/// open. A cycle named task ids only, which across seventeen files meant mapping
/// ids back to filenames by hand. The task that merely *led into* the cycle must
/// not be named: its file needs no edit.
#[test]
fn a_dependency_cycle_names_the_file_of_every_task_on_it() {
    let (_temp, result) = universe_of(&[
        (
            "TASK-TDL-001-entrypoint.md",
            task_file("TASK-TDL-001", "ready", "['TASK-TDL-010']"),
        ),
        (
            "TASK-TDL-010-first.md",
            task_file("TASK-TDL-010", "ready", "['TASK-TDL-020']"),
        ),
        (
            "TASK-TDL-020-second.md",
            task_file("TASK-TDL-020", "ready", "['TASK-TDL-010']"),
        ),
    ]);
    let message = result.expect_err("a cycle must fail").to_string();

    assert!(message.contains("dependency cycle"), "{message}");
    for on_the_cycle in ["TASK-TDL-010-first.md", "TASK-TDL-020-second.md"] {
        assert!(
            message.contains(on_the_cycle),
            "the file of every task on the cycle is named: {message}"
        );
    }
    assert!(
        !message.contains("TASK-TDL-001-entrypoint.md"),
        "the walk merely passed through TDL-001; naming its file sends the reader to a file \
         that needs no edit: {message}"
    );
}

// ---------------------------------------------------------------------------
// `status:` — what each value causes
// ---------------------------------------------------------------------------

/// The real corpus shape: fifteen of seventeen tasks declare `blocked`, meaning
/// "waiting on what I depend on". That must keep loading and keep running.
#[test]
fn blocked_behind_a_declared_dependency_still_loads_and_still_runs() {
    let (_temp, result) = universe_of(&[
        (
            "TASK-TDL-001-root.md",
            task_file("TASK-TDL-001", "ready", "[]"),
        ),
        (
            "TASK-TDL-010-downstream.md",
            task_file("TASK-TDL-010", "blocked", "['TASK-TDL-001']"),
        ),
    ]);
    let universe = result.expect("blocked-behind-a-dependency is the corpus shape");
    let contract = support::LifecycleContract {
        task_universe: &universe,
        target_repository_root: None,
    };

    let items = items_for(&universe);
    let ready = support::ready_items_from(&contract, &items, &BTreeSet::new());
    assert_eq!(
        ready_ids(&ready),
        vec!["TASK-TDL-001".to_string()],
        "the blocked task waits behind its dependency rather than being refused"
    );

    let completed = BTreeSet::from(["TASK-TDL-001".to_string()]);
    let ready = support::ready_items_from(&contract, &items, &completed);
    assert_eq!(ready_ids(&ready), vec!["TASK-TDL-010".to_string()]);
}

/// `blocked` with nothing to wait for is a claim the task set cannot discharge.
#[test]
fn blocked_with_no_declared_dependency_fails_closed_naming_the_file() {
    let (_temp, result) = universe_of(&[(
        "TASK-TDL-001-stuck.md",
        task_file("TASK-TDL-001", "blocked", "[]"),
    )]);
    let message = result.expect_err("nothing can unblock it").to_string();
    assert!(message.contains("TASK-TDL-001-stuck.md"), "{message}");
    assert!(message.contains("can ever unblock it"), "{message}");
}

/// A status nobody can classify is refused rather than defaulted. Defaulting to
/// runnable runs work the author may have cancelled; defaulting to complete
/// skips work nobody proved. Both hide the typo.
#[test]
fn an_unrecognised_status_fails_closed_naming_the_task_and_its_file() {
    let (_temp, result) = universe_of(&[(
        "TASK-TDL-001-typo.md",
        task_file("TASK-TDL-001", "blocekd", "[]"),
    )]);
    let message = result
        .expect_err("an unclassifiable status is refused")
        .to_string();
    assert!(message.contains("TASK-TDL-001"), "{message}");
    assert!(message.contains("TASK-TDL-001-typo.md"), "{message}");
    assert!(message.contains("blocekd"), "{message}");
}

/// `in_review` is not completion. `TASK-TDL-040` in the real corpus is
/// `in_review`, and a run that has not itself completed it must still do it.
#[test]
fn in_review_is_still_scheduled() {
    let (_temp, result) = universe_of(&[(
        "TASK-TDL-040-under-review.md",
        task_file("TASK-TDL-040", "in_review", "[]"),
    )]);
    let universe = result.expect("in_review loads");
    let contract = support::LifecycleContract {
        task_universe: &universe,
        target_repository_root: None,
    };
    let items = items_for(&universe);
    assert_eq!(
        ready_ids(&support::ready_items_from(
            &contract,
            &items,
            &BTreeSet::new()
        )),
        vec!["TASK-TDL-040".to_string()]
    );
}

/// The resume case. A task the file declares `done` is not re-scheduled, and it
/// satisfies its dependents instead of deadlocking them — the `completed` set is
/// empty here, which is exactly what a fresh process resuming a finished task
/// set sees.
#[test]
fn a_done_task_is_not_rescheduled_and_still_unblocks_its_dependents() {
    let (_temp, result) = universe_of(&[
        (
            "TASK-TDL-001-already-done.md",
            task_file("TASK-TDL-001", "done", "[]"),
        ),
        (
            "TASK-TDL-010-downstream.md",
            task_file("TASK-TDL-010", "blocked", "['TASK-TDL-001']"),
        ),
    ]);
    let universe = result.expect("a done task loads");
    let contract = support::LifecycleContract {
        task_universe: &universe,
        target_repository_root: None,
    };
    let items = items_for(&universe);

    assert_eq!(
        ready_ids(&support::ready_items_from(
            &contract,
            &items,
            &BTreeSet::new()
        )),
        vec!["TASK-TDL-010".to_string()],
        "the done task is not re-scheduled, and its dependent is not stranded"
    );
    assert!(
        support::item_is_completed(&contract, &items[0], &BTreeSet::new()),
        "and it reads as complete without this run having completed it"
    );
}

/// Removing work from the schedule on the strength of a line in a markdown file
/// is only honest if the reader can see which file said it.
#[test]
fn declared_statuses_are_surfaced_with_the_file_that_declared_them() {
    let (_temp, result) = universe_of(&[
        (
            "TASK-TDL-001-already-done.md",
            task_file("TASK-TDL-001", "done", "[]"),
        ),
        (
            "TASK-TDL-010-downstream.md",
            task_file("TASK-TDL-010", "blocked", "['TASK-TDL-001']"),
        ),
    ]);
    let universe = result.expect("universe");
    let contract = support::LifecycleContract {
        task_universe: &universe,
        target_repository_root: None,
    };
    let notice = contract
        .declared_status_notice()
        .expect("declared statuses are recorded");
    let rendered = notice.to_string();
    assert!(
        rendered.contains("declared_complete_not_scheduled"),
        "{rendered}"
    );
    assert!(
        rendered.contains("TASK-TDL-001-already-done.md"),
        "{rendered}"
    );
    assert!(
        rendered.contains("declared_blocked_behind_dependencies"),
        "{rendered}"
    );
    assert!(
        rendered.contains("TASK-TDL-010-downstream.md"),
        "{rendered}"
    );
}

/// Nothing is recorded when nothing was declared.
#[test]
fn no_status_notice_when_no_task_declares_one() {
    let (_temp, result) = universe_of(&[(
        "TASK-TDL-001-plain.md",
        task_file("TASK-TDL-001", "ready", "[]"),
    )]);
    let universe = result.expect("universe");
    let contract = support::LifecycleContract {
        task_universe: &universe,
        target_repository_root: None,
    };
    assert!(contract.declared_status_notice().is_none());
}

/// One scheduler item per task, carrying that task's reconciled dependencies —
/// the same shape `run_dependency_waves` selects over.
fn items_for(universe: &WorkflowV2TaskUniverse) -> Vec<serde_json::Value> {
    universe
        .tasks
        .iter()
        .map(|task| {
            serde_json::json!({
                "item_id": task.canonical_task_id,
                "canonical_task_ids": [task.canonical_task_id],
                "dependency_ids": task.dependency_ids,
                "work_type": "implementation",
            })
        })
        .collect()
}

fn ready_ids(items: &[serde_json::Value]) -> Vec<String> {
    let mut ids: Vec<String> = items
        .iter()
        .filter_map(|item| item.get("item_id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect();
    ids.sort();
    ids
}
