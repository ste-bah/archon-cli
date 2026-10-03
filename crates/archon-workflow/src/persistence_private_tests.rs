use super::*;

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
