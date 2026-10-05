//! Issue 299: a script that repeats one tool call and keeps getting the same
//! answer pauses the run with the streak as evidence; it never fails, and a
//! script that only re-reads between other work runs to its end.
use super::*;

/// Reads two files in turn, `per_half` times before and after an optional
/// checkpoint. After the first two reads every answer is one already seen.
fn looping_script(a: &str, b: &str, per_half: usize, checkpoint_between: bool) -> String {
    let between = if checkpoint_between {
        r#"await w.checkpoint("between-reads", { note: "other work" });"#
    } else {
        ""
    };
    format!(
        r#"
async function workflow(w) {{
  const files = [{a:?}, {b:?}];
  const read = (i) => w.runTool("Read", {{ file_path: files[i % 2] }});
  for (let i = 0; i < {per_half}; i += 1) await read(i);
  {between}
  for (let i = 0; i < {per_half}; i += 1) await read(i);
  return {{ finished: true }};
}}
"#
    )
}

fn two_files(temp: &tempfile::TempDir) -> (String, String) {
    let a = temp.path().join("a.txt");
    let b = temp.path().join("b.txt");
    std::fs::write(&a, "first file").unwrap();
    std::fs::write(&b, "second file").unwrap();
    (
        a.to_string_lossy().into_owned(),
        b.to_string_lossy().into_owned(),
    )
}

/// Reads per half: one more than the stall bound in total, but fewer than
/// it on each side of a checkpoint.
const PER_HALF: usize = 1_000;

fn stall_events(store: &WorkflowStore, run_id: &str) -> Vec<archon_workflow::WorkflowEvent> {
    events(store, run_id)
        .into_iter()
        .filter(|event| event.detail["event"] == "script_tool_stall_pause")
        .collect()
}

/// A loop reading two files in turn brings no new answer after its first
/// two reads: it pauses with the evidence, the call it refused unexecuted.
#[tokio::test]
async fn an_alternating_tool_loop_pauses_the_run_with_its_evidence() {
    let (temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let (a, b) = two_files(&temp);

    let outcome = run_script(&store, &run_id, &looping_script(&a, &b, PER_HALF, false)).await;

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
        detail["calls_without_a_new_answer"].as_u64().unwrap_or(0) >= 900,
        "{detail}"
    );
}

/// Other work between the reads starts a new window: the same loop with a
/// checkpoint in the middle stays under the bound on each side and finishes.
#[tokio::test]
async fn other_host_work_between_repeats_is_not_a_stall() {
    let (temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let (a, b) = two_files(&temp);

    let outcome = run_script(&store, &run_id, &looping_script(&a, &b, PER_HALF - 2, true)).await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(stall_events(&store, &run_id).is_empty());
}
