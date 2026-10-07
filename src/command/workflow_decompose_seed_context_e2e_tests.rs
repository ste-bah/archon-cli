//! Issue 288 with #360: a run whose acceptance authors were shown every
//! completed entry in full resumes on this script through its phase seed. No
//! old call needs replaying: only the refuted entry is authored again, its
//! prompt names each carried entry by one record line, and the file a record
//! names holds that entry's exact bytes, under the run's own directory.
use super::super::*;
use archon_workflow::v2::script::author_context::{AUTHOR_CONTEXT_DIR, sha256_hex};

/// The earlier script with the old prior-entry section: every entry in full.
fn full_prior_script() -> String {
    let script = earlier_script();
    let marker = "function priorText(prior, id, order) {";
    assert_eq!(script.matches(marker).count(), 1);
    script.replace(
        marker,
        "function priorText(prior) {\n  if (prior.length === 0) return \"Previously completed entries: none.\";\n  return `Previously completed entries, one JSON line each:\\n- ${prior.map((entry) => JSON.stringify(entry)).join(\"\\n- \")}`;\n}\nfunction boundedPriorText(prior, id, order) {",
    )
}

#[tokio::test]
async fn a_run_authored_with_full_prior_entries_resumes_through_its_seed() {
    let run = Run::new();
    let error = run
        .run(&full_prior_script(), run.args())
        .await
        .expect_err("the refuted entry stalls and pauses");
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    let old = run.authors.tasks();
    assert!(
        old.iter()
            .any(|task| task.contains("\"command\":\"check AC-1\"")),
        "the old prompts carried whole entries"
    );
    run.authors.strong.store(true, Ordering::SeqCst);
    let args = run.upgrade(&[("new-script", "next-rev")]);
    archon_workflow::v2::script::dry_run_workflow_plan(FIXED_SCRIPT_SOURCE, Some(&args))
        .await
        .expect("the plan preview names context it does not write");
    let summary = run
        .run(FIXED_SCRIPT_SOURCE, args)
        .await
        .expect("the seeded run completes");
    assert_eq!(summary.status, WorkflowV2Status::Accepted, "{summary:?}");
    let new = run.authors.tasks()[old.len()..].to_vec();
    assert_eq!(
        acceptance_tasks(&new),
        ["AC-2"],
        "AC-1 and AC-3 are carried"
    );
    let prompt = &new[0];
    assert!(
        !prompt.contains("\"command\":\"check AC-1\""),
        "no entry inlined: {prompt}"
    );
    let dir = run.store.run_dir(&run.run_id).join(AUTHOR_CONTEXT_DIR);
    let dir_text = dir.to_string_lossy().replace('\\', "/");
    assert!(
        prompt.contains(&format!(
            "Read its exact JSON at {dir_text}/<its sha256>.json"
        )),
        "{prompt}"
    );
    for id in ["AC-1", "AC-3"] {
        let digest = prompt
            .split(&format!("\n- {id} sha256:"))
            .nth(1)
            .and_then(|rest| rest.get(..64))
            .unwrap_or_else(|| panic!("a record for {id}: {prompt}"));
        let bytes = std::fs::read(dir.join(format!("{digest}.json"))).expect("the named file");
        assert_eq!(
            sha256_hex(&bytes),
            digest,
            "{id}'s file holds the bytes its digest names"
        );
        let entry: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(entry["id"], id);
        assert_eq!(entry["check"]["command"], format!("check {id}"));
    }
}
