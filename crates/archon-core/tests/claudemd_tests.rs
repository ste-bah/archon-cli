use std::fs;

use archon_core::claudemd::{load_hierarchical_claude_md, load_hierarchical_claude_md_with_limit};

fn create_temp_dir() -> tempfile::TempDir {
    tempfile::TempDir::new().expect("failed to create temp dir")
}

#[test]
fn empty_dir_returns_empty() {
    let tmp = create_temp_dir();
    let result = load_hierarchical_claude_md(tmp.path());
    assert!(
        result.is_empty(),
        "expected empty string for dir with no CLAUDE.md"
    );
}

#[test]
fn project_claudemd_loaded() {
    let tmp = create_temp_dir();
    fs::write(tmp.path().join("CLAUDE.md"), "# Project rules").unwrap();
    let result = load_hierarchical_claude_md(tmp.path());
    assert!(
        result.contains("# Project rules"),
        "project CLAUDE.md content should appear in output"
    );
}

#[test]
fn dot_claude_preferred() {
    let tmp = create_temp_dir();
    // Both exist; .claude/CLAUDE.md should be preferred
    fs::write(tmp.path().join("CLAUDE.md"), "plain version").unwrap();
    fs::create_dir_all(tmp.path().join(".claude")).unwrap();
    fs::write(tmp.path().join(".claude/CLAUDE.md"), "dotclaude version").unwrap();

    let result = load_hierarchical_claude_md(tmp.path());
    assert!(
        result.contains("dotclaude version"),
        "should contain .claude/CLAUDE.md content"
    );
    assert!(
        !result.contains("plain version"),
        "plain CLAUDE.md should be skipped when .claude/CLAUDE.md exists"
    );
}

#[test]
fn hierarchy_order() {
    // parent/child structure: parent has CLAUDE.md, child has CLAUDE.md
    let tmp = create_temp_dir();
    let parent = tmp.path().join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();

    fs::write(parent.join("CLAUDE.md"), "PARENT_CONTENT").unwrap();
    fs::write(child.join("CLAUDE.md"), "CHILD_CONTENT").unwrap();

    let result = load_hierarchical_claude_md(&child);
    let parent_pos = result
        .find("PARENT_CONTENT")
        .expect("parent content missing");
    let child_pos = result.find("CHILD_CONTENT").expect("child content missing");
    assert!(
        parent_pos < child_pos,
        "parent content should appear before child content (global-to-local order)"
    );
}

#[test]
fn dedup_skips_same_file() {
    let tmp = create_temp_dir();
    fs::write(tmp.path().join("CLAUDE.md"), "UNIQUE_CONTENT").unwrap();

    let result = load_hierarchical_claude_md(tmp.path());
    let count = result.matches("UNIQUE_CONTENT").count();
    assert_eq!(
        count, 1,
        "content should appear exactly once even if paths resolve to same file"
    );
}

#[test]
fn section_headers_present() {
    let tmp = create_temp_dir();
    fs::write(tmp.path().join("CLAUDE.md"), "some content").unwrap();

    let result = load_hierarchical_claude_md(tmp.path());
    assert!(
        result.contains("# CLAUDE.md from"),
        "output should contain section headers"
    );
}

#[test]
fn truncation_removes_global_first() {
    let tmp = create_temp_dir();
    let parent = tmp.path().join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();

    // Parent gets a large block, child gets a small block
    let parent_content = "P".repeat(5000);
    let child_content = "CHILD_PRESERVED";
    fs::write(parent.join("CLAUDE.md"), &parent_content).unwrap();
    fs::write(child.join("CLAUDE.md"), child_content).unwrap();

    // Limit that can only fit child content + header
    let result = load_hierarchical_claude_md_with_limit(&child, 200);
    assert!(
        result.contains("CHILD_PRESERVED"),
        "child/project content should be preserved when truncating"
    );
    assert!(
        !result.contains(&parent_content),
        "parent content should be truncated when over limit"
    );
}

#[test]
fn non_utf8_skipped() {
    let tmp = create_temp_dir();
    // Write binary content
    fs::write(tmp.path().join("CLAUDE.md"), [0xFF, 0xFE, 0x00, 0x01]).unwrap();

    // Should not panic, should return empty or skip gracefully
    let result = load_hierarchical_claude_md(tmp.path());
    // Binary file should be skipped -- result may be empty or just a header
    // The key is that it doesn't panic
    assert!(
        !result.contains("\u{FFFD}"),
        "should not contain replacement characters from binary data"
    );
}
