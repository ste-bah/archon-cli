//! A replay of a recorded acceptance round's regression search, routing and
//! blocked classification against a COPY of its run, through the stage's own
//! scratch observer (Batch J). Ignored by default; run with
//!
//!   ARCHON_ATTRIBUTION_RUN=<copied run dir>
//!   ARCHON_ATTRIBUTION_ATTEMPT=<a round record, e.g. v2/acceptance/round-01/attempt-06.json>
//!   ARCHON_ATTRIBUTION_REPO=<a clone of the target at the round's commit>
//!   ARCHON_ATTRIBUTION_SCRATCH=<a scratch parent for the observations>
//!   [ARCHON_ATTRIBUTION_MINUTES=<search time budget; the stage's default otherwise>]
//!   cargo test --bin archon attribution_replay -- --ignored --nocapture
//!
//! The scratch policy is the project's own `[workflow.acceptance_execution]`
//! with its repository and scratch parent pointed at the clone and the
//! given parent, so nothing runs in, or registers worktrees with, the live
//! checkout. It prints each failed check's outcome as JSON.

use std::path::PathBuf;

use super::*;
use archon_workflow::v2::acceptance_routing::{mark_blocked, route_failures};

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

#[tokio::test]
#[ignore = "needs a copied live run: ARCHON_ATTRIBUTION_RUN, _ATTEMPT, _REPO, _SCRATCH"]
async fn attribution_replay_of_a_recorded_round() {
    let (Some(run_dir), Some(attempt), Some(repo), Some(scratch)) = (
        env("ARCHON_ATTRIBUTION_RUN"),
        env("ARCHON_ATTRIBUTION_ATTEMPT"),
        env("ARCHON_ATTRIBUTION_REPO"),
        env("ARCHON_ATTRIBUTION_SCRATCH"),
    ) else {
        eprintln!("ARCHON_ATTRIBUTION_* unset; nothing to do");
        return;
    };
    let mut record: AcceptanceRoundRecordV1 =
        serde_json::from_slice(&std::fs::read(run_dir.join(&attempt)).unwrap()).unwrap();
    let execution = record.execution.clone().expect("the round ran somewhere");
    let (project, task_root) = (
        PathBuf::from(&execution.project),
        PathBuf::from(&execution.task_root),
    );
    let mut binding = crate::command::acceptance_scratch_policy::capture(&project, &task_root)
        .unwrap()
        .expect("the project's scratch policy");
    binding.policy.repository = repo.canonicalize().unwrap();
    binding.policy.scratch_parent = scratch.canonicalize().unwrap();
    // The run's persistent build cache, as the stage keys it (Batch J2).
    let binding = binding.with_run_build_cache(&record.run_id);
    let context = exec::StageContext {
        project,
        task_root,
        repository: binding.policy.repository.clone(),
        binding: Some(binding),
        launch: None,
        launch_lineage: archon_workflow::task_set_lineage::LaunchLineage::Predates,
        run_id: record.run_id.clone(),
    };
    let (contract, digest, _) = exec::load_contract(&context).unwrap();
    let all: Vec<&AcceptanceCriterion> = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .collect();
    for check in &mut record.checks {
        check.regressed_by = None;
        check.regression_search = None;
        check.routing = None;
        check.blocked = None;
    }
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("v2/generated-metadata.json")).unwrap())
            .unwrap();
    let universe: WorkflowV2TaskUniverse =
        serde_json::from_value(metadata["task_universe"].clone()).unwrap();
    let evidence = scratch.join("replay-evidence");
    let started = std::time::Instant::now();
    let mut budget = archon_workflow::v2::acceptance_regression::SearchBudget::default();
    if let Some(minutes) = std::env::var("ARCHON_ATTRIBUTION_MINUTES")
        .ok()
        .and_then(|m| m.parse::<u64>().ok())
    {
        budget.time = std::time::Duration::from_secs(minutes * 60);
    }
    println!("budget: {budget:?}");
    regression::attribute(
        &context,
        &all,
        &digest,
        &run_dir,
        &evidence,
        &mut record,
        budget,
    )
    .await;
    route_failures(Some(&universe), &context.repository, &all, &mut record);
    mark_blocked(&mut record);
    for check in record.failing_checks() {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "check_id": check.check_id,
                "owning_tasks": check.owning_tasks,
                "regressed_by": check.regressed_by,
                "regression_search": check.regression_search,
                "routing": check.routing,
                "blocked": check.blocked,
            }))
            .unwrap()
        );
    }
    // Each probe leaves one evidence directory, written as it finishes.
    let mut probes: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(&evidence)
        .map(|entries| {
            (entries.flatten())
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let written = std::fs::metadata(entry.path().join("scratch-observation.json"))
                        .and_then(|m| m.modified())
                        .ok()?;
                    name.starts_with("regression-").then_some((written, name))
                })
                .collect()
        })
        .unwrap_or_default();
    probes.sort();
    let mut previous = std::time::SystemTime::now() - started.elapsed();
    for (written, name) in &probes {
        let took = written.duration_since(previous).unwrap_or_default();
        println!("probe {name}: {}s", took.as_secs());
        previous = *written;
    }
    println!(
        "remediable: {}; {} probe(s); search took {:?}",
        record.has_remediable_failures(),
        probes.len(),
        started.elapsed()
    );
}
