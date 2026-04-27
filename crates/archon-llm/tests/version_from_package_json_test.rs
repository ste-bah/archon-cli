use std::fs;
use std::path::PathBuf;

use archon_llm::identity::version_from_package_json_at;

fn temp_claude_path() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("archon-test-{}", uuid::Uuid::new_v4()));
    let bin_dir = dir.join("lib").join("foo").join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let claude_exe = bin_dir.join("claude.exe");
    fs::write(&claude_exe, "").unwrap();
    (dir, claude_exe)
}

#[test]
fn returns_none_when_no_package_json() {
    let (dir, claude_exe) = temp_claude_path();
    // No package.json exists -> returns None
    let result = version_from_package_json_at(&claude_exe);
    assert!(result.is_none());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn extracts_version_from_valid_package_json() {
    let (dir, claude_exe) = temp_claude_path();
    // package.json goes two levels up from bin/claude.exe
    let pkg_dir = claude_exe.parent().unwrap().parent().unwrap();
    let pkg_json = r#"{"version":"2.1.119","name":"@anthropic-ai/claude-code"}"#;
    fs::write(pkg_dir.join("package.json"), pkg_json).unwrap();
    let result = version_from_package_json_at(&claude_exe);
    assert_eq!(result, Some("2.1.119".to_string()));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn returns_none_on_malformed_json() {
    let (dir, claude_exe) = temp_claude_path();
    let pkg_dir = claude_exe.parent().unwrap().parent().unwrap();
    fs::write(pkg_dir.join("package.json"), "not valid json").unwrap();
    let result = version_from_package_json_at(&claude_exe);
    assert!(result.is_none());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn returns_none_when_version_field_missing() {
    let (dir, claude_exe) = temp_claude_path();
    let pkg_dir = claude_exe.parent().unwrap().parent().unwrap();
    let pkg_json = r#"{"name":"@anthropic-ai/claude-code","other":"data"}"#;
    fs::write(pkg_dir.join("package.json"), pkg_json).unwrap();
    let result = version_from_package_json_at(&claude_exe);
    assert!(result.is_none());
    fs::remove_dir_all(&dir).unwrap();
}
