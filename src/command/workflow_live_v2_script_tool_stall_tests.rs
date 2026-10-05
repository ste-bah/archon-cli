//! Issue 299: a script that repeats one tool call and keeps getting the same
//! answer pauses the run with the streak as evidence; it never fails, and a
//! script that only re-reads between other work runs to its end.
use super::*;

fn looping_script(path: &str, checkpoint_between: bool) -> String {
    let between = if checkpoint_between {
        r#"await w.checkpoint("between-reads", { note: "other work" });"#
    } else {
        ""
    };
    format!(
        r#"
async function workflow(w) {{
  const read = () => w.runTool("Read", {{ file_path: {path:?} }});
  for (let i = 0; i < 99; i += 1) await read();
  {between}
  for (let i = 0; i < 99; i += 1) await read();
  return {{ finished: true }};
}}
"#
    )
}

fn stall_events(store: &WorkflowStore, run_id: &str) -> Vec<archon_workflow::WorkflowEvent> {
    events(store, run_id)
        .into_iter()
        .filter(|event| event.detail["event"] == "script_tool_stall_pause")
        .collect()
}

#[tokio::test]
async fn a_repeating_tool_call_pauses_the_run_with_its_evidence() {
    let (temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let file = temp.path().join("same.txt");
    std::fs::write(&file, "unchanging").unwrap();

    let outcome = run_script(
        &store,
        &run_id,
        &looping_script(&file.to_string_lossy(), false),
    )
    .await;

    assert!(
        matches!(&outcome, Err(WorkflowError::ControlPaused(message)) if message.contains("Read")),
        "a stall pauses, never fails: {outcome:?}"
    );
    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, archon_workflow::RunStatus::Paused);
    let stalls = stall_events(&store, &run_id);
    assert_eq!(stalls.len(), 1, "one pause event with the evidence");
    let detail = &stalls[0].detail;
    assert_eq!(detail["cause"], "no_progress");
    assert_eq!(detail["tool"], "Read");
    assert_eq!(detail["refused_call_executed"], false);
    assert!(
        detail["identical_calls_in_a_row"].as_u64().unwrap_or(0) >= 98,
        "{detail}"
    );
}

/// Other work between the reads breaks the streak: the same 198 reads with
/// a checkpoint in the middle are two short runs, and the script finishes.
#[tokio::test]
async fn other_host_work_between_repeats_is_not_a_stall() {
    let (temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let file = temp.path().join("same.txt");
    std::fs::write(&file, "unchanging").unwrap();

    let outcome = run_script(
        &store,
        &run_id,
        &looping_script(&file.to_string_lossy(), true),
    )
    .await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(stall_events(&store, &run_id).is_empty());
}
