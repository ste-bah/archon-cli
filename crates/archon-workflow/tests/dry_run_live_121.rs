//! A dry run of the residual passes and the final gate against a COPY of a
//! live run's records and the live target repository (read-only). Ignored by
//! default; run with
//!   ARCHON_DRY_RUN_RUN=<copied run dir> ARCHON_DRY_RUN_REPO=<repository>
//!   cargo test -p archon-workflow --test dry_run_live_121 -- --ignored --nocapture
//! The copied run dir needs `events.jsonl`, `v2/results/`,
//! `v2/baseline-tests/` and `v2/generated-metadata.json`. The session is
//! every call answered since the second-to-last resume (a resume replays the
//! whole script, so that is the run as the next session will reach its
//! residual slots). It prints each pass's plan, whether each third-pass round
//! passes the host's dispatch check as the prelude would file it and can be
//! completed inside its tasks' declared files and granted files, and what
//! the final gate says once the third slot is asked. With
//! `ARCHON_DRY_RUN_SIMULATE=resolves` it first records, in the COPY, an
//! accepted verifier of the named second-pass round whose host run of the
//! commands the refused verifier's red tests failed in passes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::script::residual_paths::owners;
use archon_workflow::v2::script::residual_plan::{
    RESIDUAL_GAPS_MARKER, RESIDUAL_PASS_KEY, residual_plan_view, residual_refusal,
    residual_verdict, round_claim, second_pass_view, session_records, third_pass_plan,
    third_pass_view,
};
use archon_workflow::v2::verification::path_ownership::{
    DeclaredPathForm, declared_path_form, declared_paths_of,
};
use archon_workflow::*;
use serde_json::{Value, json};

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn slot(pass: u64) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert(RESIDUAL_GAPS_MARKER.into(), Value::Bool(true));
    options.extra.insert(RESIDUAL_PASS_KEY.into(), json!(pass));
    WorkflowV2HostCall {
        id: format!("residual-gaps-{pass}"),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options,
    }
}

fn declared(
    universe: &WorkflowV2TaskUniverse,
    tasks: &BTreeSet<String>,
    repo: &Path,
) -> Vec<String> {
    universe
        .tasks
        .iter()
        .filter(|task| tasks.contains(&task.canonical_task_id))
        .flat_map(declared_paths_of)
        .filter_map(|entry| match declared_path_form(&entry, repo) {
            DeclaredPathForm::Repo(path) => Some(path),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Record, in the copy, an accepted verifier of the second-pass round `key`
/// (tasks `tasks`) whose host runs of `commands` passed.
fn simulate_resolves(
    run: &Path,
    store: &WorkflowV2ResultStore,
    key: &str,
    tasks: &[String],
    commands: &[String],
) {
    let id = "verification-wave-review-verify-dry-run-121-resolves";
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        json!({"version": 1, "stage": "verify", "taskId": format!("cross:{}", tasks.join("+")),
            "taskIds": tasks, "round": 1, "maxRounds": 1, "contest": key,
            "residual": {"key": key, "files": [], "pass": 2}}),
    );
    let call = WorkflowV2HostCall {
        id: id.into(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options,
    };
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "simulated: the regression is fixed".into(),
        ..WorkflowV2Result::default()
    };
    let record = WorkflowV2CallRecord::new(
        run.display().to_string(),
        call,
        1,
        "h".into(),
        result,
        vec![],
    );
    store.save_call_record(&record).unwrap();
    store.note_session_call(id);
    let dir = run.join("v2/baseline-tests").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    let runs: Vec<Value> = commands
        .iter()
        .map(|command| {
            json!({"command": command, "base_commit": "sim", "exit_code": 0, "timed_out": false,
            "duration_ms": 1, "failing_tests": [], "cached": false})
        })
        .collect();
    std::fs::write(
        dir.join(format!("{id}-0.json")),
        json!({"schema_version": 1, "stage_id": id, "branch_id": format!("{id}-0"), "base_commit": "sim",
            "canonical_task_ids": tasks, "commands": runs, "obligations": [], "routed": [], "ignored": [],
            "inherited": [], "pre_existing": []})
        .to_string(),
    )
    .unwrap();
}

#[test]
#[ignore = "needs a copied live run: ARCHON_DRY_RUN_RUN and ARCHON_DRY_RUN_REPO"]
fn dry_run_the_live_third_pass() {
    let (Some(run), Some(repo)) = (env("ARCHON_DRY_RUN_RUN"), env("ARCHON_DRY_RUN_REPO")) else {
        eprintln!("ARCHON_DRY_RUN_RUN / ARCHON_DRY_RUN_REPO unset; nothing to do");
        return;
    };
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(run.join("v2/generated-metadata.json")).unwrap())
            .unwrap();
    let universe: WorkflowV2TaskUniverse =
        serde_json::from_value(metadata["task_universe"].clone()).unwrap();
    let events = std::fs::read_to_string(run.join("events.jsonl")).unwrap();
    let lines: Vec<&str> = events.lines().collect();
    let resumes: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("\"kind\":\"resumed\""))
        .map(|(at, _)| at)
        .collect();
    let from = resumes.iter().rev().nth(1).copied().unwrap_or(0);
    let mut order: Vec<String> = Vec::new();
    for event in lines
        .iter()
        .skip(from)
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        if let Some(id) = event["detail"]["call_id"].as_str()
            && !order.iter().any(|seen| seen == id)
        {
            order.push(id.to_string());
        }
    }
    let store = WorkflowV2ResultStore::new(run.join("v2"));
    let mut calls: Vec<WorkflowV2HostCall> = Vec::new();
    for id in &order {
        if let Some(record) = store.load_call_record(id).unwrap() {
            store.note_session_call(id);
            calls.push(record.call.clone());
        }
    }
    println!(
        "== session since resume line {from}: {} calls answered",
        calls.len()
    );
    let show = |label: &str, rounds: Vec<Value>| {
        println!("== {label}");
        for round in rounds {
            println!(
                "  ROUND {} kind={} severity={} tasks={} granted={} dispatchable={} attempted={}",
                round["key"],
                round["kind"],
                round["severity"],
                round["task_ids"],
                round["expansion_files"],
                round["dispatchable"],
                round["attempted"]
            );
            for finding in round["findings"].as_array().into_iter().flatten() {
                println!(
                    "    carries {} {} by {}: {}",
                    finding["id"],
                    finding["severity"],
                    finding["recorded_by"],
                    finding["description"]
                        .as_str()
                        .unwrap_or_default()
                        .chars()
                        .take(300)
                        .collect::<String>()
                );
            }
        }
    };
    show(
        "pass 1 `residual-gaps-1` plans",
        residual_plan_view(&store, Some(&universe), Some(&repo)),
    );
    show(
        "pass 2 `residual-gaps-2` plans",
        second_pass_view(&store, Some(&universe), Some(&repo)),
    );
    if std::env::var("ARCHON_DRY_RUN_SIMULATE").as_deref() == Ok("resolves") {
        let key = std::env::var("ARCHON_DRY_RUN_ROUND")
            .expect("ARCHON_DRY_RUN_ROUND: the second-pass round key");
        let tasks: Vec<String> = std::env::var("ARCHON_DRY_RUN_TASKS")
            .unwrap()
            .split(',')
            .map(str::to_string)
            .collect();
        let commands: Vec<String> = std::env::var("ARCHON_DRY_RUN_COMMANDS")
            .unwrap()
            .split(';')
            .map(str::to_string)
            .collect();
        simulate_resolves(&run, &store, &key, &tasks, &commands);
        println!(
            "== SIMULATED: an accepted verifier of {key} whose host runs of {commands:?} passed"
        );
    }
    show(
        "pass 3 `residual-gaps-3` would plan now",
        third_pass_view(&store, Some(&universe), Some(&repo)).unwrap(),
    );
    let records = session_records(&store);
    let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
    let third = third_pass_plan(&refs, &store, Some(&universe), Some(&repo));
    for (residual, why) in &third.reported {
        println!("  REPORTED {}: {why}", residual.label());
    }
    // Implementability: each round as the prelude files it, through the
    // host's dispatch check, and every file its gaps name writable by it.
    for round in &third.rounds {
        let files: Vec<&str> = round.files.iter().map(String::as_str).collect();
        let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
        let mut targets = declared(&universe, &round.tasks, &repo);
        targets.extend(round.files.iter().cloned());
        let mut contract = json!({"version": 1, "stage": "remediate", "round": 1, "maxRounds": 1,
            "sourceReduceCallIds": ["adversarial-review-reduce", "coverage-audit-reduce"], "contest": round.key,
            "residual": {"key": round.key, "files": files, "pass": 3}});
        if tasks.len() > 1 {
            contract["taskId"] = json!(format!("cross:{}", tasks.join("+")));
            contract["taskIds"] = json!(tasks);
        } else {
            contract["taskId"] = json!(tasks[0]);
        }
        let adjudication = round.kind.as_str() == "adjudication";
        if adjudication {
            contract["stage"] = json!("verify");
        }
        let mut options = WorkflowV2HostOptions::default();
        options.extra.insert("remediationContract".into(), contract);
        let finding = json!([{"id": round.key, "claim": round_claim(round)}]).to_string();
        options.task = Some(format!(
            "Findings (verbatim):\n{}",
            json!([{"claim": finding}])
        ));
        let mut item = json!({"canonical_task_ids": tasks, "target_files": targets});
        if !adjudication {
            item["residual_expansion_paths"] = json!(files);
        }
        let execution = WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: format!("dry-run-{}", round.key),
                method: if adjudication {
                    WorkflowV2HostMethod::Parallel
                } else {
                    WorkflowV2HostMethod::Fanout
                },
                write_mode: (!adjudication).then_some(WorkflowV2WriteMode::Worktree),
                options,
            },
            input: json!({"source_data": [item]}),
            depends_on: vec![],
        };
        println!(
            "  DISPATCH {} ({}): {}",
            round.key,
            round.kind.as_str(),
            residual_refusal(&execution, &store, Some(&universe), Some(&repo))
                .unwrap_or_else(|| "answered".into())
        );
        for residual in &round.residuals {
            for file in &residual.files {
                let by = owners(&universe, file, &repo);
                let writable = round.files.contains(file) || !by.is_disjoint(&round.tasks);
                println!(
                    "    {} names {file}: declared by {by:?}; writable by the round: {writable}",
                    residual.id
                );
            }
        }
    }
    let mut asked = calls.clone();
    asked.extend([slot(3)]);
    let verdict = residual_verdict(&asked, &store, Some(&universe), Some(&repo));
    println!("== the final gate once the third slot is asked, before its rounds run");
    for clause in &verdict.blocking {
        println!("  BLOCKS {clause}");
    }
    for note in &verdict.notes {
        println!("  NOTE   {note}");
    }
}
