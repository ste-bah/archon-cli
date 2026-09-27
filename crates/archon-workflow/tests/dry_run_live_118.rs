//! A dry run of the residual final gate against a COPY of a live run's
//! records and the live target repository (read-only). Ignored by default;
//! run with
//!   ARCHON_DRY_RUN_RUN=<copied run dir> ARCHON_DRY_RUN_REPO=<repository>
//!   cargo test -p archon-workflow --test dry_run_live_118 -- --ignored --nocapture
//! The copied run dir needs `events.jsonl`, `v2/results/`,
//! `v2/baseline-tests/` and `v2/generated-metadata.json`. It prints, for the
//! latest session's calls in the order it answered them: the plan each
//! residual slot answers, and what the final gate says.

use std::path::PathBuf;

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::script::residual_plan::{
    RESIDUAL_GAPS_MARKER, RESIDUAL_PASS_KEY, is_residual_slot, plan_from, residual_plan_view,
    residual_verdict, second_pass_view, with_residual_plan,
};
use archon_workflow::*;
use serde_json::Value;

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

#[test]
#[ignore = "needs a copied live run: ARCHON_DRY_RUN_RUN and ARCHON_DRY_RUN_REPO"]
fn dry_run_the_live_final_gate() {
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
    let resumed_at = lines
        .iter()
        .rposition(|line| line.contains("\"kind\":\"resumed\""))
        .unwrap_or(0);
    let mut order: Vec<String> = Vec::new();
    for event in lines
        .iter()
        .skip(resumed_at)
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
    println!("== session: {} calls answered", calls.len());
    for call in calls.iter().filter(|call| is_residual_slot(call)) {
        let pass = call
            .options
            .extra
            .get(RESIDUAL_GAPS_MARKER)
            .cloned()
            .unwrap_or_default();
        println!("== slot `{}` ({pass}) plans", call.id);
        for round in residual_plan_view(&store, Some(&universe), Some(&repo)) {
            println!(
                "  ROUND {} kind={} severity={} tasks={} granted={} attempted={}",
                round["key"],
                round["kind"],
                round["severity"],
                round["task_ids"],
                round["expansion_files"],
                round["attempted"]
            );
        }
    }
    if let Some(slot) = calls.iter().position(is_residual_slot) {
        let before: Vec<WorkflowV2CallRecord> = calls[..slot]
            .iter()
            .filter_map(|call| store.load_call_record(&call.id).ok().flatten())
            .filter(|record| record.invalidated_by.is_none())
            .collect();
        let refs: Vec<&WorkflowV2CallRecord> = before.iter().collect();
        let plan = plan_from(&refs, Some(&universe), Some(&repo));
        println!("== the gate's own plan (records before the slot)");
        for round in &plan.rounds {
            println!(
                "  ROUND {} kind={} tasks={:?}",
                round.key,
                round.kind.as_str(),
                round.tasks
            );
        }
        for (residual, why) in &plan.reported {
            println!("  REPORTED {}: {why}", residual.label());
        }
    }
    // Replay safety: the only existing records whose view the change can
    // move are the residual slots; print each one's plan keys.
    let records = store.load_call_records().unwrap();
    for record in &records {
        if let Some(view) =
            with_residual_plan(record, &record.result, &store, Some(&universe), Some(&repo))
        {
            let keys: Vec<&str> = view.data["residual_plan"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|round| round["key"].as_str())
                .collect();
            println!("existing record `{}` view plans {keys:?}", record.call.id);
        }
    }
    println!("== the second pass `residual-gaps-2` would plan now");
    for round in second_pass_view(&store, Some(&universe), Some(&repo)) {
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
                    .take(400)
                    .collect::<String>()
            );
        }
    }
    let mut second = WorkflowV2HostOptions::default();
    second
        .extra
        .insert(RESIDUAL_GAPS_MARKER.into(), Value::Bool(true));
    second
        .extra
        .insert(RESIDUAL_PASS_KEY.into(), serde_json::json!(2));
    let mut with_second = calls.clone();
    with_second.push(WorkflowV2HostCall {
        id: "residual-gaps-2".into(),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options: second,
    });
    let asked = residual_verdict(&with_second, &store, Some(&universe), Some(&repo));
    println!("== the final gate once the second slot is asked, before its rounds run");
    for clause in &asked.blocking {
        println!("  BLOCKS {clause}");
    }
    for note in &asked.notes {
        println!("  NOTE   {note}");
    }
    let verdict = residual_verdict(&calls, &store, Some(&universe), Some(&repo));
    println!("== the final gate over the session so far");
    for clause in &verdict.blocking {
        println!("  BLOCKS {clause}");
    }
    for note in &verdict.notes {
        println!("  NOTE   {note}");
    }
}
