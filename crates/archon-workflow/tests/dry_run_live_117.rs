//! A dry run of the Issue-117 host rules against a COPY of a live run's
//! records and the live target repository (read-only). Ignored by default;
//! run with
//!   ARCHON_DRY_RUN_RUN=<copied run dir> ARCHON_DRY_RUN_REPO=<repository>
//!   cargo test -p archon-workflow --test dry_run_live_117 -- --ignored --nocapture
//! The copied run dir needs `events.jsonl`, `v2/results/` and
//! `v2/generated-metadata.json`. It answers, for a session that replays every
//! call the latest session answered: whether any existing call's view
//! changes (so whether any existing call id or prompt can move), what the
//! pre-acceptance slot plans, the exact new calls the prelude would issue
//! for it, and what the final gate would say if none of them resolved.

use std::path::PathBuf;

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::script::residual_plan::{
    RESIDUAL_GAPS_MARKER, done_checkpoint_id, is_residual_round, plan_from, residual_verdict,
    residuals_of, session_records, with_residual_plan,
};
use archon_workflow::*;
use serde_json::Value;

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn slug(text: &str) -> String {
    let lowered: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let parts: Vec<&str> = lowered.split('-').filter(|p| !p.is_empty()).collect();
    let joined = parts.join("-");
    let cut: String = joined.chars().take(40).collect();
    if cut.is_empty() { "step".into() } else { cut }
}

fn key_hash(text: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for unit in text.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// The prelude's `unitLabel`.
fn unit_label(prefix: &str, key: &str, suffix: &str) -> String {
    let plain = format!("{prefix}-{}-{suffix}", slug(key));
    if plain.len() <= 40 {
        return plain;
    }
    let tail = format!("-{}-{suffix}", key_hash(key));
    let room = 40usize.saturating_sub(prefix.len() + 1 + tail.len()).max(1);
    let head: String = slug(key).chars().take(room).collect();
    format!("{prefix}-{}{tail}", head.trim_end_matches('-'))
}

#[test]
#[ignore = "needs a copied live run: ARCHON_DRY_RUN_RUN and ARCHON_DRY_RUN_REPO"]
fn dry_run_the_live_records() {
    let (Some(run), Some(repo)) = (env("ARCHON_DRY_RUN_RUN"), env("ARCHON_DRY_RUN_REPO")) else {
        eprintln!("ARCHON_DRY_RUN_RUN / ARCHON_DRY_RUN_REPO unset; nothing to do");
        return;
    };
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(run.join("v2/generated-metadata.json")).unwrap())
            .unwrap();
    let universe: WorkflowV2TaskUniverse =
        serde_json::from_value(metadata["task_universe"].clone()).unwrap();
    // The latest session's calls, in the order it answered them.
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
    let records = store.load_call_records().unwrap();
    println!(
        "== session: {} calls answered, {} records on disk",
        calls.len(),
        records.len()
    );
    // Replay safety: no existing record's view carries a residual plan and
    // none is a residual round, so every answer the script reads, and with
    // it every later call id and prompt, is what the deployed host gave.
    let changed: Vec<&str> = records
        .iter()
        .filter(|record| {
            with_residual_plan(record, &record.result, &store, Some(&universe), Some(&repo))
                .is_some()
                || is_residual_round(&record.call)
                || record.call.options.extra.contains_key(RESIDUAL_GAPS_MARKER)
        })
        .map(|record| record.call.id.as_str())
        .collect();
    println!("existing records whose view or role the change moves: {changed:?}");
    println!("== in-scope residual gaps of accepted remediation verifiers this session");
    let session = session_records(&store);
    for record in &session {
        let accepted = archon_workflow::v2::script::residual_plan::accepted_verdict(record);
        if !accepted || is_residual_round(&record.call) {
            continue;
        }
        for residual in residuals_of(record, Some(&repo)) {
            println!(
                "  {} files={:?} unit={:?}",
                residual.label(),
                residual.files,
                residual.unit_tasks
            );
        }
    }
    let refs: Vec<&WorkflowV2CallRecord> = session.iter().collect();
    let plan = plan_from(&refs, Some(&universe), Some(&repo));
    println!("== the plan the pre-acceptance slot `residual-gaps-1` is answered");
    for round in &plan.rounds {
        println!(
            "ROUND {} kind={} tasks={:?} granted={:?}",
            round.key,
            round.kind.as_str(),
            round.tasks,
            round.files
        );
        for residual in &round.residuals {
            println!(
                "    carries {} files={:?}",
                residual.label(),
                residual.files
            );
        }
        let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
        let key = if tasks.len() > 1 {
            format!("cross:{}", tasks.join("+"))
        } else {
            tasks[0].to_string()
        };
        if round.kind.as_str() == "adjudication" {
            println!("    NEW verify  verification-wave-{}-adjudicate", round.key);
            println!("    NEW done    {}", done_checkpoint_id(&round.key));
            continue;
        }
        let suffix = format!("{}-1", round.key);
        println!(
            "    NEW fix     {}-<N>",
            slug(&unit_label("review-remediate", &key, &suffix))
        );
        println!(
            "    NEW verify  verification-wave-{}-<N+1>",
            slug(&unit_label("review-verify", &key, &suffix))
        );
        println!("    NEW done    {}", done_checkpoint_id(&round.key));
    }
    println!("== reported at the final gate (no round carries them)");
    for (residual, why) in &plan.reported {
        println!("  {} files={:?}: {why}", residual.label(), residual.files);
    }
    let mut ended = calls.clone();
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert(RESIDUAL_GAPS_MARKER.into(), Value::Bool(true));
    ended.push(WorkflowV2HostCall {
        id: "residual-gaps-1".into(),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options,
    });
    let verdict = residual_verdict(&ended, &store, Some(&universe), Some(&repo));
    println!("== the final gate if no planned round resolved");
    for clause in &verdict.blocking {
        println!("  BLOCKS {clause}");
    }
    for note in &verdict.notes {
        println!("  NOTE   {note}");
    }
}
