//! Through the production resume, stopped at the provider barrier: the
//! frozen chain, bodies and phase seed the script would have been given.

use super::*;

fn task_root(store: &WorkflowStore, run_id: &str) -> PathBuf {
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    PathBuf::from(state.identity.task_root_identity)
}

/// A body landing as an observe-mode run records it: published, under the
/// previous land-task-body logic, with `findings`.
fn land(
    store: &WorkflowStore,
    run_id: &str,
    (task, id, sequence, status): (&str, &str, u64, &str),
    findings: serde_json::Value,
) {
    let file = format!("{task}.md");
    let path = task_root(store, run_id).join(&file);
    std::fs::write(&path, format!("# {task}\n\nlanded by {id}\n")).unwrap();
    let receipt = receipt(
        store,
        run_id,
        "land-task-body",
        id,
        vec![entry(file.clone(), path, None)],
    );
    let version = crate::command::workflow_host_command_logic::versions()["land-task-body"];
    save_bound(
        store,
        run_id,
        ("land-task-body", id, Some(sequence), status),
        version - 1,
        findings,
        receipt,
        serde_json::json!([{"fileName": file, "taskId": task}]),
    );
}

async fn resume_to_barrier(
    project: &tempfile::TempDir,
    store: &WorkflowStore,
    run_id: &str,
) -> serde_json::Value {
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let resume = UpgradeBarrier {
        run_id: run_id.to_string(),
        store: store.clone(),
        expected: state.identity.clone(),
        builds: AtomicUsize::new(0),
    };
    let error = resume_fixed_decomposition_at_binary_revision(
        project.path(),
        run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &resume,
        crate::command::workflow_decompose_progress::DecompositionCliUiSink::shared(),
        None,
        None,
        "next-logic-binary",
    )
    .await
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("barrier observed"),
        "{error:#}"
    );
    assert_eq!(resume.builds.load(Ordering::SeqCst), 1);
    crate::command::workflow_decompose_seed::resume::capture::take()
        .expect("the resume built script arguments")
}

#[tokio::test]
async fn observe_mode_body_with_findings_reopens_while_a_clean_body_stays_frozen() {
    let (project, store, run_id, _) = chained_run(1, None).await;
    let findings = serde_json::json!([{
        "text": "body repair remains open", "subject": "land-task-body",
        "remediation_scope": "candidate_artifact"
    }]);
    land(
        &store,
        &run_id,
        ("TASK-X-010", "land-clean", 160, "accepted"),
        serde_json::json!([]),
    );
    land(
        &store,
        &run_id,
        ("TASK-X-011", "land-observed", 161, "needs_review"),
        findings,
    );
    let output = dry_run(&project, &run_id).await;
    assert!(
        output.contains("frozen_chain_override body:TASK-X-010.md=verify reason=judged attempt land-clean gate logic changed; re-run the phase under current logic"),
        "{output}"
    );
    assert!(
        output.contains("frozen_chain_override body:TASK-X-011.md=keep reason=judged attempt land-observed status=needs_review"),
        "{output}"
    );
    let script = resume_to_barrier(&project, &store, &run_id).await;
    let chain = &script["frozenChain"];
    assert_eq!(chain["acceptance"], true, "{chain}");
    assert_eq!(chain["skeleton"], true, "{chain}");
    assert_eq!(
        chain["bodies"],
        serde_json::json!(["TASK-X-010.md"]),
        "{chain}"
    );
    assert_eq!(chain["subjects"].as_array().unwrap().len(), 2, "{chain}");
}

#[tokio::test]
async fn two_same_binary_resumes_after_a_flip_hand_the_script_one_floor_seed() {
    let (project, store, run_id, _) = chained_run(1, None).await;
    let first = resume_to_barrier(&project, &store, &run_id).await;
    assert_eq!(first["frozenChain"]["acceptance"], true, "{first}");
    let seed = first["phaseSeed"].clone();
    assert!(seed.is_object(), "{first}");
    let seeds = store
        .run_dir(&run_id)
        .join(crate::command::workflow_decompose_seed::SEEDS_DIR);
    let recorded: Vec<_> = std::fs::read_dir(&seeds)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        recorded.len() == 1 && recorded[0].starts_with("resume-floors-"),
        "{recorded:?}"
    );
    // Work after the seed: a body author call and its landing, as a run on
    // this binary leaves them before it pauses again.
    let author = archon_workflow::WorkflowV2CallRecord::new(
        &run_id,
        archon_workflow::WorkflowV2HostCall {
            id: "body-TASK-X-011-author-12".into(),
            method: archon_workflow::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        1,
        "after-the-seed".into(),
        archon_workflow::WorkflowV2Result::accepted("a body attempt after the seed"),
        Vec::new(),
    );
    archon_workflow::WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"))
        .save_call_record(&author)
        .unwrap();
    land(
        &store,
        &run_id,
        ("TASK-X-011", "land-after-seed", 300, "needs_review"),
        serde_json::json!([{"text": "still open", "subject": "land-task-body", "remediation_scope": "candidate_artifact"}]),
    );
    let second = resume_to_barrier(&project, &store, &run_id).await;
    assert_eq!(
        second["phaseSeed"], seed,
        "a same-binary resume must keep the floors fixed at the first flip"
    );
    assert!(
        second["phaseSeed"]["history_attempts"]
            .get("body-TASK-X-011-author-12")
            .is_none()
    );
    assert!(
        second["phaseSeed"]["author_ordinals"]
            .get("body-TASK-X-011")
            .is_none_or(|ordinal| ordinal.as_u64() < Some(12))
    );
}

#[tokio::test]
async fn digest_only_repin_resume_hands_the_script_no_flip_and_no_seed() {
    let (project, store, run_id, _) = chained_run(0, None).await;
    let script = resume_to_barrier(&project, &store, &run_id).await;
    assert_eq!(script["frozenChain"]["acceptance"], false, "{script}");
    assert_eq!(script["frozenChain"]["skeleton"], false, "{script}");
    assert!(
        script
            .get("phaseSeed")
            .is_none_or(serde_json::Value::is_null)
    );
}
