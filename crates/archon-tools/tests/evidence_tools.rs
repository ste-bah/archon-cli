use std::fs;
use std::os::unix::fs::PermissionsExt;

use archon_tools::docs::DocStatus;
use archon_tools::learning::LearningInspect;
use archon_tools::tool::{Tool, ToolContext};
use serde_json::json;
use serial_test::serial;

fn install_fake_archon(dir: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let bin = dir.path().join("archon-fake");
    let log = dir.path().join("argv.log");
    fs::write(
        &bin,
        r#"#!/usr/bin/env bash
printf '%s\n' "$*" >> "$ARCHON_EVIDENCE_TOOL_LOG"
echo "fake archon:$*"
"#,
    )
    .unwrap();
    let mut perms = fs::metadata(&bin).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&bin, perms).unwrap();
    (bin, log)
}

fn set_tool_env(bin: &std::path::Path, log: &std::path::Path) {
    unsafe {
        std::env::set_var("ARCHON_EVIDENCE_TOOL_BIN", bin);
        std::env::set_var("ARCHON_EVIDENCE_TOOL_LOG", log);
    }
}

#[tokio::test]
#[serial]
async fn test_doc_status_executes_cli_and_verifies_argv_source_of_truth() {
    let temp = tempfile::tempdir().unwrap();
    let (bin, log) = install_fake_archon(&temp);
    set_tool_env(&bin, &log);

    let result = DocStatus.execute(json!({}), &ToolContext::default()).await;

    assert!(!result.is_error);
    assert!(result.content.contains("fake archon:docs status"));
    let observed = fs::read_to_string(log).unwrap();
    assert_eq!(observed.trim(), "docs status");
}

#[tokio::test]
#[serial]
async fn test_learning_inspect_executes_behaviour_show_source_of_truth() {
    let temp = tempfile::tempdir().unwrap();
    let (bin, log) = install_fake_archon(&temp);
    set_tool_env(&bin, &log);

    let result = LearningInspect
        .execute(json!({ "id": "lev-123" }), &ToolContext::default())
        .await;

    assert!(!result.is_error);
    assert!(
        result
            .content
            .contains("fake archon:behaviour show lev-123")
    );
    let observed = fs::read_to_string(log).unwrap();
    assert_eq!(observed.trim(), "behaviour show lev-123");
}
