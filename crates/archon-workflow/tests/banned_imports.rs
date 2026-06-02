#[test]
fn banned_import_guard_mentions_archon_workflow() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();
    let script = std::fs::read_to_string(root.join("scripts/check-banned-imports.sh")).unwrap();
    assert!(script.contains("crates/archon-workflow"));
    assert!(script.contains("archon_llm::providers"));
}
