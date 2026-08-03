//! The remaining plan-shape assertions, plus the readable dump of what a run
//! *would* do.
//!
//! The dump exists because deciding whether a 17-task decomposition is right is
//! a judgement no assertion can make. It is written on every run of this test to
//! `target/workflow-plan-reports/prd-trading-data-lake-ahdm-001.md` so the
//! decomposition can be read and argued with before a real run is spent on it.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use super::super::workflow_live_v2_lifecycle_adversarial as adversarial;
use super::prd_conformance::render_waves;
use super::{
    all_learning_enabled, fixture_universe, generated_plan, plan_task_text, wave_index_by_task,
    wave_layering,
};
use crate::command::learning_workflow_hooks::derive_learning_hooks;
use archon_workflow::task_universe::WorkflowV2TaskUniverse;

/// Review branches for every task, which is what the plan produces once every
/// task has been accepted by its own verification.
fn review_items(universe: &WorkflowV2TaskUniverse) -> Vec<Value> {
    let all = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect::<BTreeSet<_>>();
    adversarial::per_task_review_items(universe, &all, &[])
}

#[test]
fn there_is_one_adversarial_review_branch_per_task_with_an_item_id_derived_from_the_task_id() {
    let universe = fixture_universe();
    let items = review_items(&universe);

    assert_eq!(
        items.len(),
        universe.tasks.len(),
        "the review stage must produce exactly one branch per task"
    );
    let mut seen = BTreeSet::new();
    for (item, task) in items.iter().zip(universe.tasks.iter()) {
        let item_id = item["item_id"].as_str().expect("branch carries an item_id");
        assert_eq!(
            item_id,
            adversarial::review_item_id(&task.canonical_task_id),
            "branch id is not a total function of the task id"
        );
        assert!(
            item_id.ends_with(&task.canonical_task_id),
            "{item_id} does not carry {} — attribution would stop being structural",
            task.canonical_task_id
        );
        assert!(
            seen.insert(item_id.to_string()),
            "duplicate branch {item_id}"
        );
        assert_eq!(
            item["task_id"],
            Value::String(task.canonical_task_id.clone())
        );
        assert_eq!(
            item["canonical_task_ids"],
            serde_json::json!([task.canonical_task_id])
        );
        assert_eq!(item["review_scope"], Value::String("single_task".into()));
        assert_eq!(
            item["adversarial_review_notes"],
            serde_json::json!(task.adversarial_review_notes),
            "{} lost its declared review notes on the way to its reviewer",
            task.canonical_task_id
        );
        assert!(
            !task.adversarial_review_notes.is_empty(),
            "{} declares no adversarial review notes, so its reviewer has no hypothesis to test",
            task.canonical_task_id
        );
    }

    // The plan must actually contain the stage that consumes these, as a
    // PARALLEL family rather than a terminal reduce.
    let plan = generated_plan();
    let review = plan
        .calls
        .iter()
        .find(|call| call.id == "adversarial-review")
        .expect("the plan declares an adversarial-review stage family");
    assert_eq!(
        review.method.as_str(),
        "parallel",
        "per-task review must fan out; a reduce has no per-item branch to stamp"
    );
    assert!(
        plan.calls
            .iter()
            .any(|call| call.id == "cross-cutting-review"),
        "the narrowed terminal reduce must still exist"
    );
}

#[test]
fn derived_learning_hooks_are_non_empty_and_every_one_routes_in_the_fold() {
    use crate::command::topology_fold::workflow_learning::plan_dispatch;

    let plan = generated_plan();
    assert!(
        !plan.learning_hooks.is_empty(),
        "a generated plan with every learning subsystem enabled must name at least one hook"
    );
    assert_eq!(
        plan.learning_hooks,
        derive_learning_hooks(
            &plan_task_text(&super::fixture_root()),
            plan.task_universe.as_ref(),
            &all_learning_enabled()
        ),
        "the plan's hooks must be the ones its own content derives"
    );

    // Routed by the real fold, not by a mirrored allowlist: a record carrying
    // exactly these hooks must leave `unrouted_hooks` empty and produce a call.
    let record = serde_json::from_value(serde_json::json!({
        "run_id": "wf-plan-inspection",
        "name": &plan.name,
        "stage_id": "implementation-wave-1",
        "phase": "implementation",
        "agent": null,
        "status": "accepted",
        "verification": "accepted",
        "durable": true,
        "artifact_refs": [],
        "telemetry": { "attempt": 1, "error_class": null, "artifact_count": 0 },
        "trace_ref": null,
        "hooks": &plan.learning_hooks,
        "ts": "2026-01-01T00:00:00Z",
    }))
    .expect("learning record deserializes");

    let dispatch = plan_dispatch(std::slice::from_ref(&record));
    assert!(
        dispatch.unrouted_hooks.is_empty(),
        "derived hook(s) the fold cannot route: {:?}",
        dispatch.unrouted_hooks
    );
    assert_eq!(dispatch.skipped_unhooked, 0);
    assert_eq!(
        dispatch.calls.len(),
        1,
        "a completed stage carrying routable hooks must dispatch exactly once"
    );
}

/// Rewrite the checked-in report. It is committed rather than left in `target/`
/// so the decomposition can be read and argued with without running anything,
/// and so a change to the plan shows up as a reviewable diff instead of as a
/// number nobody looked at. Nothing machine-specific is rendered, so running
/// this on another checkout must leave the file byte-identical.
#[test]
fn the_plan_report_is_written_for_review() {
    let report = render_report();
    // `tests/plan-reports/`, not `tests/artifacts/`: the repository gitignores
    // every `artifacts/` directory, so a report written there would look
    // committed locally and be invisible to everyone else.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/plan-reports/prd-trading-data-lake-ahdm-001.md");
    fs::create_dir_all(path.parent().expect("artifact directory"))
        .expect("create report directory");
    fs::write(&path, &report).expect("write plan report");
    println!("plan report written to {}", path.display());

    // The report is the deliverable, so its completeness is the assertion.
    for expected in [
        "## Stage graph",
        "## Wave layering",
        "## Per-task adversarial review branches",
        "## Per-task inputs",
        "TASK-TDL-001",
        "TASK-TDL-140",
    ] {
        assert!(report.contains(expected), "report is missing {expected}");
    }
    assert!(
        !report.contains(env!("CARGO_MANIFEST_DIR")),
        "the report leaked this checkout's absolute path; it would churn on every machine"
    );
}

fn render_report() -> String {
    let plan = generated_plan();
    let universe = plan
        .task_universe
        .clone()
        .expect("a decomposed-PRD plan carries its task universe");
    let waves = wave_layering(&universe);
    let index = wave_index_by_task(&waves);
    let items = review_items(&universe);
    let mut out = String::new();

    let _ = writeln!(
        out,
        "# Generated plan — PRD-TRADING-DATA-LAKE-AHDM-001\n\n\
         What a decomposed-PRD run would do, computed without running it. Generated by\n\
         `workflow_live_v2_prd_pipeline_report_tests::the_plan_report_is_written_for_review`;\n\
         do not hand-edit; run that test to refresh it.\n\n\
         - tasks: {}\n- stage families: {}\n- dependency waves: {}\n\
         - learning hooks: {:?}\n- target repository root: {}\n\n\
         Absolute paths are deliberately omitted: the workflow name and the repository root are\n\
         both derived from the task text, which carries this checkout's location.\n",
        universe.tasks.len(),
        plan.calls.len(),
        waves.len(),
        plan.learning_hooks,
        if plan.target_repository_root.is_some() {
            "inferred from the task text"
        } else {
            "NOT inferred — write coordination would have no root"
        }
    );

    let _ = writeln!(
        out,
        "\n## Stage graph\n\nThe static family list the lifecycle schedules from. \
         Families with a dynamic prefix are instantiated once per wave or round.\n\n\
         | # | family | host method | write mode |\n|---:|---|---|---|"
    );
    for (n, call) in plan.calls.iter().enumerate() {
        let _ = writeln!(
            out,
            "| {} | `{}` | {} | {} |",
            n + 1,
            call.id,
            call.method.as_str(),
            match call.write_mode {
                Some(mode) => format!("{mode:?}"),
                None => "-".to_string(),
            }
        );
    }

    let _ = writeln!(
        out,
        "\n## Wave layering\n\n`{}`\n\n| wave | task | title | complexity | status | depends on |\n\
         |---:|---|---|---|---|---|",
        render_waves(&waves)
    );
    for (wave, ids) in waves.iter().enumerate() {
        for id in ids {
            let task = universe
                .tasks
                .iter()
                .find(|task| &task.canonical_task_id == id)
                .expect("wave names a task in the universe");
            let _ = writeln!(
                out,
                "| {wave} | `{id}` | {} | {} | {} | {} |",
                task.title.as_deref().unwrap_or("-"),
                task.complexity.as_deref().unwrap_or("-"),
                task.status.as_deref().unwrap_or("-"),
                if task.dependency_ids.is_empty() {
                    "-".to_string()
                } else {
                    task.dependency_ids.join(", ")
                }
            );
        }
    }

    let _ = writeln!(
        out,
        "\n## Per-task adversarial review branches\n\nOne PARALLEL branch per task, run as soon as \
         that task's own verification accepts it. `item_id` is host-stamped from the task id, so \
         a finding that names no task is still attributed.\n\n\
         | branch `item_id` | task | declared review notes | acceptance criteria |\n|---|---|---:|---:|"
    );
    for item in &items {
        let task_id = item["task_id"].as_str().unwrap_or("-");
        let _ = writeln!(
            out,
            "| `{}` | `{task_id}` | {} | {} |",
            item["item_id"].as_str().unwrap_or("-"),
            item["adversarial_review_notes"]
                .as_array()
                .map_or(0, Vec::len),
            item["acceptance_criteria"].as_array().map_or(0, Vec::len)
        );
    }

    let _ = writeln!(
        out,
        "\n## Per-task inputs\n\nStamped onto every implementation item by the host from the task \
         universe. An agent cannot grant itself a tool, an env key or a contract that its task \
         file did not declare.\n\n\
         | wave | task | required tools | required env keys | deliverable contracts |\n|---:|---|---|---|---|"
    );
    let mut ordered = universe.tasks.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|task| {
        (
            index[&task.canonical_task_id],
            task.canonical_task_id.clone(),
        )
    });
    for task in ordered {
        let _ = writeln!(
            out,
            "| {} | `{}` | {} | {} | {} |",
            index[&task.canonical_task_id],
            task.canonical_task_id,
            join_or_dash(&task.required_tools),
            join_or_dash(&task.required_env_keys),
            join_or_dash(
                &task
                    .deliverable_contracts
                    .iter()
                    .map(|contract| format!("{} → `{}`", contract.kind, contract.artifact_path))
                    .collect::<Vec<_>>()
            )
        );
    }
    out
}

fn join_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join("<br>")
    }
}
