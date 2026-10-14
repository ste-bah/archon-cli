use super::*;

#[test]
fn agent_output_keeps_unknown_usage_null_and_reported_zero_zero() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let run = store
        .create_run(crate::WorkflowSpec {
            schema: crate::spec::WORKFLOW_SCHEMA.into(),
            name: "usage-persistence".into(),
            task: "usage-persistence".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let run_id = run.id.as_str();
    let output = StageRunOutput::markdown("result");
    record_agent_output(
        &store,
        run_id,
        "stage",
        "unknown",
        Some(&output),
        None,
        true,
        None,
    )
    .unwrap();

    let unknown: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            store
                .run_dir(run_id)
                .join("agent-outputs/stage/unknown.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(unknown["tokens_in"].is_null());
    assert!(unknown["tokens_out"].is_null());
    assert!(unknown["cost_usd"].is_null());
    assert!(!crate::events::contains_forbidden_field(&unknown));

    let mut zero = output;
    zero.tokens_in = Some(0);
    zero.tokens_out = Some(0);
    record_agent_output(
        &store,
        run_id,
        "stage",
        "zero",
        Some(&zero),
        None,
        true,
        None,
    )
    .unwrap();
    let reported_zero: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("agent-outputs/stage/zero.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(reported_zero["tokens_in"], 0);
    assert_eq!(reported_zero["tokens_out"], 0);
    assert!(!crate::events::contains_forbidden_field(&reported_zero));

    let mut mixed = StageRunOutput::markdown("mixed usage");
    mixed.tokens_in = Some(19);
    mixed.tokens_out = None;
    record_agent_output(
        &store,
        run_id,
        "stage",
        "mixed",
        Some(&mixed),
        None,
        true,
        None,
    )
    .unwrap();
    // Read the persisted public record independently of the serializer input:
    // this covers the complete sanitize + redaction + write path.
    let mixed_record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("agent-outputs/stage/mixed.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(mixed_record["tokens_in"], 19);
    assert!(mixed_record["tokens_out"].is_null());
    assert!(!crate::events::contains_forbidden_field(&mixed_record));

    let string_usage = crate::events::sanitize_value(serde_json::json!({
        "tokens_in": "credential-shaped value",
        "tokens_out": null
    }));
    assert!(string_usage.get("tokens_in").is_none());
    assert!(string_usage["tokens_out"].is_null());
    println!(
        "EVIDENCE #369 sanitize-redact BEFORE: mixed public output tokens_in=19/tokens_out=null and string tokens_in; AFTER: persisted tokens_in={}, tokens_out={}, string-key-present={}",
        mixed_record["tokens_in"],
        mixed_record["tokens_out"],
        string_usage.get("tokens_in").is_some()
    );

    let detail = crate::web_api::detail(&store, run_id).unwrap();
    let unknown_view = detail
        .agents
        .iter()
        .find(|agent| agent.item_id == "unknown")
        .expect("unknown usage record remains visible");
    assert_eq!(unknown_view.tokens_in, None);
    assert_eq!(unknown_view.tokens_out, None);
    let zero_view = detail
        .agents
        .iter()
        .find(|agent| agent.item_id == "zero")
        .expect("reported zero record remains visible");
    assert_eq!(zero_view.tokens_in, Some(0));
    assert_eq!(zero_view.tokens_out, Some(0));
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn authoritative_agent_results_have_private_directories_and_files() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let output = StageRunOutput::markdown("Authorization: Bearer opaque-credential-123");
    record_captured_agent_output(&store, "run", "stage", "item", &output).unwrap();
    let root = store.run_dir("run").join(AGENT_RESULTS_DIR);
    assert_eq!(mode(&root), 0o700);
    assert_eq!(mode(&root.join("stage")), 0o700);
    assert_eq!(mode(&root.join("stage/item.json")), 0o600);
    let raw = std::fs::read_to_string(root.join("stage/item.json")).unwrap();
    assert!(raw.contains("opaque-credential-123"));
    let public =
        std::fs::read_to_string(store.run_dir("run").join("agent-outputs/stage/item.json"))
            .unwrap();
    assert!(!public.contains("opaque-credential-123"));
}

#[cfg(unix)]
#[test]
fn existing_authoritative_directories_are_made_private_before_writing() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let root = store.run_dir("run").join(AGENT_RESULTS_DIR);
    let stage = root.join("stage");
    std::fs::create_dir_all(&stage).unwrap();
    for path in [&root, &stage] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    record_agent_output(
        &store,
        "run",
        "stage",
        "item",
        None,
        None,
        false,
        Some("secret=raw"),
    )
    .unwrap();
    assert_eq!(mode(&root), 0o700);
    assert_eq!(mode(&stage), 0o700);
    assert_eq!(mode(&stage.join("item.json")), 0o600);
}

#[cfg(unix)]
#[test]
fn authoritative_temporary_file_is_private_even_when_publication_fails() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let stage = store.run_dir("run").join(AGENT_RESULTS_DIR).join("stage");
    // A directory in the target slot forces rename to fail after writing.
    std::fs::create_dir_all(stage.join("item.json")).unwrap();
    assert!(
        record_agent_output(
            &store,
            "run",
            "stage",
            "item",
            None,
            None,
            false,
            Some("secret=raw")
        )
        .is_err()
    );
    let files: Vec<_> = std::fs::read_dir(&stage)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .collect();
    assert_eq!(
        files.len(),
        1,
        "the unpublished temporary file is inspectable"
    );
    assert_eq!(mode(&files[0]), 0o600);
}
