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

async fn fixed_author_task(
    set: &crate::command::workflow_task_set::republish::test_fixture::FrozenSet,
    notes: &[&str],
) -> String {
    let scope =
        crate::command::workflow_task_set::reauthor::AuthorScope::for_task_set_without_cache(
            set.project.path(),
            &set.tasks,
            &set.prd,
        )
        .unwrap();
    let run = Run::new();
    let mut args = run.args();
    args["projectRoot"] = serde_json::json!(set.project.path());
    args["repositoryRoot"] = serde_json::json!(&scope.repository_root);
    args["taskRoot"] = serde_json::json!(&set.tasks);
    args["prdPath"] = serde_json::json!(&set.prd);
    args["acceptanceCriteria"] = serde_json::json!({"AC-X-001":"criterion"});
    let entry = serde_json::to_string(&set.contract().acceptance[0]).unwrap();
    let notes = serde_json::to_string(notes).unwrap();
    let script = format!(
        r#"{FIXED_SCRIPT_SOURCE}
async function workflow(w) {{
  const id = "AC-X-001";
  const notes = {notes};
  const state = {{entries:new Map([[id,{entry}]]),order:[id]}};
  const prompt = authorPrompt(acceptanceAuthorPrompt(), 1, notes, []);
  await authorOne(w, prompt, 0, id, args.acceptanceCriteria[id], [], args.acceptanceCriteria, state);
  return {{}};
}}
"#
    );
    run.run(&script, args)
        .await
        .expect("the real script host dispatches the author");
    let tasks = run.authors.tasks();
    assert_eq!(tasks.len(), 1);
    tasks[0].clone()
}

#[tokio::test]
async fn read_guard_fixed_author_receives_named_content_and_stop_rule() {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set(&[(
        "AC-X-001",
        "inspect src/decision.data",
        false,
    )]);
    let path = set.project.path().join("src/decision.data");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "deciding content is supplied\n").unwrap();
    let task = fixed_author_task(&set, &[]).await;
    assert!(task.contains("Answer once you have read the supplied named_files content for the deciding file, or inspected that file at its named path yourself."), "{task}");
    assert!(task.contains("deciding content is supplied"), "{task}");
    assert!(task.contains("Files already identified"));
}

#[tokio::test]
async fn read_guard_fixed_author_prefetch_redacts_and_spends_the_existing_prompt_budget() {
    let set = crate::command::workflow_task_set::republish::test_fixture::frozen_set(&[(
        "AC-X-001",
        "inspect src/private.data",
        false,
    )]);
    let root = set.project.path().join("src");
    std::fs::create_dir_all(&root).unwrap();
    let secret = "sk-abcdefghijklmnopqrst0000";
    std::fs::write(
        root.join("private.data"),
        format!("credential = {secret}\n"),
    )
    .unwrap();
    std::fs::write(root.join("large.data"), "é".repeat(150_000)).unwrap();
    let task = fixed_author_task(&set, &["considered inputs: src/large.data"]).await;
    assert!(task.contains("***REDACTED***"), "{task}");
    assert!(!task.contains(secret));
    let (_, supplied) = task
        .split_once("Files already identified for this check are supplied here when available: ")
        .unwrap();
    let supplied: serde_json::Value = serde_json::from_str(supplied).unwrap();
    assert!(
        supplied
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"] == "src/large.data" && file["status"] == "not_supplied")
    );
    assert!(!task.contains(&"é".repeat(1000)));
    assert!(task.contains("Return exactly one complete reply with one entry and one check."));
    assert!(task.len() <= 262_144);
}
