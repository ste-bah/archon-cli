//! Whole-pipeline plan generation over the real 17-task PRD decomposition.
//!
//! # Why this exists
//!
//! Each piece of the decomposed-PRD path was made correct in isolation: the
//! task parser round-trips all declared fields, malformed files fail loudly,
//! `blocks` edges are read, adversarial review became per-task, attribution is
//! host-stamped. Nothing had ever driven the whole path over a real task set
//! and looked at the plan that comes out the other end.
//!
//! These tests generate the plan for
//! `tests/fixtures/prd-trading-data-lake-ahdm-001` — 17 task files copied from
//! a user's own PRD decomposition — and assert its shape against what those
//! files *declare*, never against a golden blob. A snapshot of the plan would
//! be a snapshot of whatever the pipeline does today, including its bugs.
//!
//! Nothing here runs a workflow, spawns an agent, or calls a provider. The
//! decomposed-PRD planning path is deterministic and LLM-free by construction
//! (`compile_harness_plan` takes the static `decomposed_prd_plan_calls()`
//! branch whenever a task universe was extracted), so the plan is fully
//! computable offline.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use archon_core::config::{GeneratedWorkflowConfig, LearningConfig};
use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;
use crate::command::workflow_live::workflow_live_generated_scaffold::{
    decomposed_prd_plan_calls, decomposed_prd_scaffold,
};
use crate::command::workflow_live::workflow_live_planner::WorkflowScriptPlan;
use archon_workflow::task_universe::{
    WorkflowV2TaskUniverse, extract_task_universe_for_generated_run,
};

#[path = "workflow_live_v2_prd_pipeline_prd_tests.rs"]
mod prd_conformance;
#[path = "workflow_live_v2_prd_pipeline_report_tests.rs"]
mod report;

pub(super) fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/prd-trading-data-lake-ahdm-001")
}

/// The task description a user would type. It must name an absolute path and
/// trip `requires_authoritative_task_universe`, or the planner takes the
/// provider-authored branch instead of the deterministic one.
pub(super) fn plan_task_text(root: &std::path::Path) -> String {
    format!(
        "Implement the decomposed PRD task files under {} for PRD-TRADING-DATA-LAKE-AHDM-001",
        root.display()
    )
}

pub(super) fn fixture_universe() -> WorkflowV2TaskUniverse {
    extract_task_universe_for_generated_run(&plan_task_text(&fixture_root()))
        .expect("the real 17-task fixture extracts without error")
        .expect("a decomposed-PRD task description yields a task universe")
}

/// The plan `plan_live` would build for this task, minus the two things that
/// need I/O: the store lookup for prior learning context (empty here) and the
/// TUI notifications. Everything that decides orchestration is here.
pub(super) fn generated_plan() -> WorkflowScriptPlan {
    let task = plan_task_text(&fixture_root());
    let universe = fixture_universe();
    let config = GeneratedWorkflowConfig::default();
    let harness = decomposed_prd_scaffold(&task, None, &universe, &[], &config)
        .expect("the deterministic scaffold renders");
    WorkflowScriptPlan::generated(
        &task,
        &harness,
        decomposed_prd_plan_calls(),
        Some(universe),
        config,
        &all_learning_enabled(),
    )
}

/// Every learning toggle on, including SONA's separate batch-recording consent.
/// The operator's own config decides what a real run derives; this fixes the
/// toggles so the test measures the derivation rather than the machine's
/// config file.
pub(super) fn all_learning_enabled() -> LearningConfig {
    let mut learning = LearningConfig::default();
    learning.sona.enabled = true;
    learning.sona.pipeline_recording = true;
    learning.reasoning_bank.enabled = true;
    learning.desc.enabled = true;
    learning
}

/// Replay the scheduler's own ready-item selection until every task is
/// complete, recording which tasks became eligible together.
///
/// This is not a reimplementation of the layering: it drives
/// `support::ready_items_from` and `support::item_is_completed`, the two
/// functions `run_dependency_waves` itself calls, over one item per task
/// carrying that task's reconciled `dependency_ids`. What a live run adds on
/// top is the reducer-authored inventory; what it cannot add is a different
/// answer to "which of these items may start now".
pub(super) fn wave_layering(universe: &WorkflowV2TaskUniverse) -> Vec<Vec<String>> {
    let contract = support::LifecycleContract {
        task_universe: universe,
        target_repository_root: None,
    };
    let mut remaining = universe
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
        .collect::<Vec<Value>>();
    let mut completed: BTreeSet<String> = BTreeSet::new();
    let mut waves: Vec<Vec<String>> = Vec::new();
    while !remaining.is_empty() {
        let ready = support::ready_items_from(&contract, &remaining, &completed);
        assert!(
            !ready.is_empty(),
            "scheduler deadlocked with {} task(s) never eligible: {:?}",
            remaining.len(),
            remaining
                .iter()
                .filter_map(|item| item.get("item_id").and_then(Value::as_str))
                .collect::<Vec<_>>()
        );
        let wave = ready
            .iter()
            .filter_map(|item| item.get("item_id").and_then(Value::as_str))
            .map(str::to_string)
            .collect::<Vec<String>>();
        completed.extend(wave.iter().cloned());
        remaining.retain(|item| !support::item_is_completed(&contract, item, &completed));
        waves.push(wave);
        assert!(
            waves.len() <= universe.tasks.len(),
            "layering produced more waves than tasks; the loop is not converging"
        );
    }
    waves
}

/// `canonical_task_id -> wave index`.
pub(super) fn wave_index_by_task(waves: &[Vec<String>]) -> BTreeMap<String, usize> {
    let mut index = BTreeMap::new();
    for (wave, ids) in waves.iter().enumerate() {
        for id in ids {
            index.insert(id.clone(), wave);
        }
    }
    index
}

#[test]
fn all_seventeen_tasks_reach_the_generated_plan_with_canonical_ids_and_no_duplicates() {
    let plan = generated_plan();
    let universe = plan
        .task_universe
        .as_ref()
        .expect("a decomposed-PRD plan carries its task universe");

    assert_eq!(universe.tasks.len(), 17, "the PRD decomposes into 17 tasks");
    let ids = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect::<Vec<_>>();
    let unique = ids.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        unique.len(),
        ids.len(),
        "duplicate canonical task ids: {ids:?}"
    );

    // Canonical form, and resolvable by every alias the universe advertises.
    for task in &universe.tasks {
        assert!(
            task.canonical_task_id.starts_with("TASK-TDL-"),
            "non-canonical id {}",
            task.canonical_task_id
        );
        for alias in std::iter::once(task.canonical_task_id.clone()).chain(task.aliases.clone()) {
            assert_eq!(
                universe
                    .resolve_canonical_task_id(&alias)
                    .expect("declared alias resolves"),
                task.canonical_task_id,
                "alias '{alias}' did not resolve to its own task"
            );
        }
        assert!(
            !task.source_path.is_empty(),
            "{} lost its source path",
            task.canonical_task_id
        );
    }

    // The plan itself, not just the universe.
    assert!(
        plan.generated_scaffold().is_some(),
        "a plan holding a task universe must produce a generated scaffold"
    );
    assert!(
        plan.harness_source.contains("max_dependency_waves: 51"),
        "the scaffold's wave budget must be derived from the 17-task universe"
    );
}

#[test]
fn the_dependency_graph_honours_both_depends_on_and_blocks() {
    let universe = fixture_universe();
    let by_id = universe
        .tasks
        .iter()
        .map(|task| (task.canonical_task_id.as_str(), task))
        .collect::<BTreeMap<_, _>>();

    let mut reversed = 0usize;
    for task in &universe.tasks {
        for blocked in &task.blocks_ids {
            let target = by_id.get(blocked.as_str()).unwrap_or_else(|| {
                panic!("{} blocks unknown task {blocked}", task.canonical_task_id)
            });
            assert!(
                target.dependency_ids.contains(&task.canonical_task_id),
                "{} declares it blocks {blocked}, but {blocked}'s reconciled dependencies are {:?}",
                task.canonical_task_id,
                target.dependency_ids
            );
            reversed += 1;
        }
        // Every reconciled dependency must be a real task, and nothing may
        // depend on itself.
        for dependency in &task.dependency_ids {
            assert!(
                by_id.contains_key(dependency.as_str()),
                "{} depends on unknown task {dependency}",
                task.canonical_task_id
            );
            assert_ne!(
                dependency, &task.canonical_task_id,
                "{} depends on itself",
                task.canonical_task_id
            );
        }
    }
    assert_eq!(
        reversed, 26,
        "the fixture declares 26 blocks edges; all of them must survive reconciliation"
    );

    // Reachability, which is what the graph is for: the audit task gates every
    // other task, and the last task gates nothing.
    assert_eq!(universe.downstream_task_closure("TASK-TDL-001").len(), 17);
    assert_eq!(universe.downstream_task_closure("TASK-TDL-140").len(), 1);
}

#[test]
fn wave_assignment_is_a_correct_topological_layering() {
    let universe = fixture_universe();
    let waves = wave_layering(&universe);
    let index = wave_index_by_task(&waves);

    assert_eq!(
        index.len(),
        universe.tasks.len(),
        "every task must be assigned exactly one wave"
    );
    for task in &universe.tasks {
        let own = index[&task.canonical_task_id];
        for dependency in &task.dependency_ids {
            assert!(
                index[dependency] < own,
                "{} is in wave {own} but its dependency {dependency} is in wave {}",
                task.canonical_task_id,
                index[dependency]
            );
        }
    }
    // Layering must be tight: a task whose dependencies are all satisfied
    // earlier has no reason to sit out a wave, and a loose layering silently
    // serialises work that could run together.
    for task in &universe.tasks {
        let own = index[&task.canonical_task_id];
        let deepest = task
            .dependency_ids
            .iter()
            .map(|dependency| index[dependency] + 1)
            .max()
            .unwrap_or(0);
        assert_eq!(
            own, deepest,
            "{} sits in wave {own} but its dependencies allow wave {deepest}",
            task.canonical_task_id
        );
    }
}

#[test]
fn required_tools_env_keys_and_deliverable_contracts_reach_the_per_task_inputs() {
    use archon_workflow::generated_contract::normalize_generated_item_value;

    let universe = fixture_universe();
    let mut saw_tools = 0usize;
    let mut saw_env = 0usize;
    let mut saw_contracts = 0usize;

    for task in &universe.tasks {
        // The barest thing a reducer can emit for a task: an id and nothing
        // else. Every declared capability below is stamped by the host from
        // the universe, never read back off the item.
        let raw = serde_json::json!({
            "item_id": format!("impl-{}", task.canonical_task_id),
            "canonical_task_ids": [task.canonical_task_id],
        });
        let item = normalize_generated_item_value(&raw, Some(&universe)).value;

        let stamped_tools = strings_at(&item, "required_tools");
        let stamped_env = strings_at(&item, "required_env_keys");
        assert_eq!(
            stamped_tools, task.required_tools,
            "{} declared required_tools {:?} but the item carries {stamped_tools:?}",
            task.canonical_task_id, task.required_tools
        );
        assert_eq!(
            stamped_env, task.required_env_keys,
            "{} declared required_env_keys {:?} but the item carries {stamped_env:?}",
            task.canonical_task_id, task.required_env_keys
        );

        let stamped_paths = item
            .get("deliverable_contracts")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.get("artifact_path").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let declared_paths = task
            .deliverable_contracts
            .iter()
            .map(|contract| contract.artifact_path.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            stamped_paths, declared_paths,
            "{} lost deliverable contract paths on the way to its item",
            task.canonical_task_id
        );

        saw_tools += usize::from(!stamped_tools.is_empty());
        saw_env += usize::from(!stamped_env.is_empty());
        saw_contracts += usize::from(!stamped_paths.is_empty());
    }

    // Equality with an empty declaration is satisfied vacuously, so assert the
    // fixture actually exercises each channel.
    assert!(saw_tools >= 4, "only {saw_tools} task(s) carried tools");
    assert!(saw_env >= 2, "only {saw_env} task(s) carried env keys");

    // 15 of 17, not 17: TASK-TDL-042 and TASK-TDL-090 declare
    // `deliverable_contracts: []`. That is the author's statement, not a parse
    // failure, and the pipeline is right to carry it through — but it means
    // those two tasks reach the deliverable gate with nothing to check, so
    // acceptance for them rests entirely on focused verification and review.
    // Pinned so that stops being a silent property of the task set.
    let ungated = universe
        .tasks
        .iter()
        .filter(|task| task.deliverable_contracts.is_empty())
        .map(|task| task.canonical_task_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ungated,
        ["TASK-TDL-042", "TASK-TDL-090"],
        "the set of tasks with no deliverable contract changed"
    );
    assert_eq!(saw_contracts, 17 - ungated.len());
}

/// A `shared_append_target_files:` declared in a task file must arrive on the
/// fan-out item payload, because that payload is the only place the write
/// coordinator looks — `resolve_shared_append_targets` reads the item and never
/// the task file. Until this was wired the declaration was inert: an author
/// could name a shared registry in a task, the coordinator would keep
/// serialising the writers they had said were coordinated, and nothing reported
/// that the declaration had gone nowhere.
///
/// The second half is the more important one. A task that declared nothing must
/// come back with the key absent, not present-and-empty and not inherited from
/// the task beside it. A path becoming concurrently written because a key
/// appeared by default is the failure this whole mechanism exists to prevent.
#[test]
fn a_declared_shared_append_path_reaches_the_payload_the_coordinator_reads() {
    use archon_workflow::generated_contract::normalize_generated_item_value;
    use archon_workflow::write_coordinator::{
        SHARED_APPEND_TARGETS_KEY, resolve_shared_append_targets,
    };

    const SHARED: &str = ".archon/trading-lab/data/registry.json";
    let dir = tempfile::tempdir().expect("tempdir");
    for entry in std::fs::read_dir(fixture_root()).expect("read fixture") {
        let path = entry.expect("fixture entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("TASK-") || !name.ends_with(".md") {
            continue;
        }
        let mut raw = std::fs::read_to_string(&path).expect("read fixture task");
        if name.starts_with("TASK-TDL-010") {
            let declared = format!("shared_append_target_files: ['{SHARED}']\nrequired_tools: []");
            assert!(raw.contains("required_tools: []"), "{name} changed shape");
            raw = raw.replacen("required_tools: []", &declared, 1);
        }
        std::fs::write(dir.path().join(name), raw).expect("write task");
    }

    let universe = extract_task_universe_for_generated_run(&plan_task_text(dir.path()))
        .expect("the mutated fixture extracts without error")
        .expect("a decomposed-PRD task description yields a task universe");
    let declaring = universe
        .tasks
        .iter()
        .find(|task| task.canonical_task_id == "TASK-TDL-010")
        .expect("the mutated task is in the universe");
    assert_eq!(
        declaring.shared_append_target_files,
        [SHARED],
        "the parser did not carry the declaration onto the task record"
    );

    let item = item_for(&universe, "TASK-TDL-010");
    assert_eq!(strings_at(&item, SHARED_APPEND_TARGETS_KEY), [SHARED]);
    assert_eq!(
        resolve_shared_append_targets(&item).expect("the payload is a readable declaration"),
        [SHARED],
        "the coordinator's own reader does not see the declared path"
    );

    let quiet = item_for(&universe, "TASK-TDL-020");
    assert!(
        quiet.get(SHARED_APPEND_TARGETS_KEY).is_none(),
        "a task that declared nothing must not carry the key at all: {quiet}"
    );
    assert!(
        resolve_shared_append_targets(&quiet)
            .expect("an absent key is not an error")
            .is_empty(),
        "an undeclared target must stay exclusive"
    );

    fn item_for(universe: &WorkflowV2TaskUniverse, task_id: &str) -> Value {
        normalize_generated_item_value(
            &serde_json::json!({
                "item_id": format!("impl-{task_id}"),
                "canonical_task_ids": [task_id],
            }),
            Some(universe),
        )
        .value
    }
}

fn strings_at(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
