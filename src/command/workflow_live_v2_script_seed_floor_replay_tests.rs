//! Issue 360 with Issue 337: a run seeded after an upgrade replays none of
//! the history before its seed, by any covered-replay path. A pause taken
//! before the seed (a `w.pause` or a host-taken crash pause) credits nothing,
//! and a host pause taken after it never replays the attempts still standing
//! from before it: the seeded run's gate judges again. A run without a seed
//! still replays what its pauses cover.

use super::*;

/// The gate answers, then the run takes the pause `args` asks for. A run
/// with `crashFirst` crashes before it reaches the gate.
const GATE_THEN_PAUSE: &str = r#"
async function workflow(w) {
  if (args.crashFirst) throw new Error("the run crashed before its gate");
  const gate = await w.hostCommand("task-set-lint", { stdin: null });
  if (args.pause) await w.pause("pause-gate-1", { subject: "gate", reason: "no_progress" });
  if (args.crash) throw new Error("the run crashed after its gate");
  return JSON.stringify({ gate: gate.stdout });
}
"#;

async fn paused(store: &WorkflowStore, run_id: &str, counters: &Counters, args: serde_json::Value) {
    let (runner, _ui) = fixed_runner(store, run_id, counters, args);
    let error = runner.run(GATE_THEN_PAUSE).await.unwrap_err();
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    resume(store, run_id);
}

/// The seed a resume derives now: the generation is the pause floor, each
/// recorded attempt the history floor.
fn seed_now(store: &WorkflowStore, run_id: &str) -> serde_json::Value {
    let records = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .load_call_records()
        .unwrap();
    let history: serde_json::Map<String, serde_json::Value> = records
        .iter()
        .map(|record| (record.call.id.clone(), record.attempt.into()))
        .collect();
    serde_json::json!({
        "pause_generation_floor": store.load_state(run_id).unwrap().generation,
        "history_attempts": history,
    })
}

/// After the first pause: a run (seeded when `seed` is set) crashes before
/// its gate, so its host pause covers the old gate attempt; the next run
/// then asks the gate.
async fn crash_before_gate_then_finish(
    store: &WorkflowStore,
    run_id: &str,
    counters: &Counters,
    seed: Option<serde_json::Value>,
) -> serde_json::Value {
    let with_seed = |mut args: serde_json::Value| {
        if let Some(seed) = &seed {
            args["phaseSeed"] = seed.clone();
        }
        args
    };
    paused(
        store,
        run_id,
        counters,
        with_seed(serde_json::json!({"crashFirst": true})),
    )
    .await;
    let (runner, _ui) = fixed_runner(store, run_id, counters, with_seed(serde_json::json!({})));
    let summary = runner.run(GATE_THEN_PAUSE).await.expect("the run ends");
    script_result(&summary)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seeded_run_never_replays_a_gate_a_pre_seed_w_pause_covers() {
    let (_temp, store, run_id, counters) = setup();
    paused(
        &store,
        &run_id,
        &counters,
        serde_json::json!({"pause": true}),
    )
    .await;
    assert_eq!(counters.ran(), ["task-set-lint"]);
    let seed = seed_now(&store, &run_id);

    let result = crash_before_gate_then_finish(&store, &run_id, &counters, Some(seed)).await;

    assert_eq!(
        counters.ran(),
        ["task-set-lint", "task-set-lint"],
        "the seeded run's gate judges again"
    );
    assert_eq!(result["gate"], "refusal 2", "{result}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_without_a_seed_still_replays_what_its_pauses_cover() {
    let (_temp, store, run_id, counters) = setup();
    paused(
        &store,
        &run_id,
        &counters,
        serde_json::json!({"pause": true}),
    )
    .await;

    let result = crash_before_gate_then_finish(&store, &run_id, &counters, None).await;

    assert_eq!(counters.ran(), ["task-set-lint"], "the refusal is replayed");
    assert_eq!(result["gate"], "refusal 1", "{result}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seeded_run_never_replays_a_gate_a_pre_seed_crash_pause_covers() {
    let (_temp, store, run_id, counters) = setup();
    paused(
        &store,
        &run_id,
        &counters,
        serde_json::json!({"crash": true}),
    )
    .await;
    assert_eq!(counters.ran(), ["task-set-lint"]);
    let seed = seed_now(&store, &run_id);

    let result = crash_before_gate_then_finish(&store, &run_id, &counters, Some(seed)).await;

    assert_eq!(
        counters.ran(),
        ["task-set-lint", "task-set-lint"],
        "the seeded run's gate judges again"
    );
    assert_eq!(result["gate"], "refusal 2", "{result}");
}
