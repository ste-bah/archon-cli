use std::fs;

use archon_core::archonmd::{load_hierarchical_archon_md, load_hierarchical_archon_md_with_limit};

fn create_temp_dir() -> tempfile::TempDir {
    tempfile::TempDir::new().expect("failed to create temp dir")
}

#[test]
fn empty_dir_returns_empty() {
    let tmp = create_temp_dir();
    let result = load_hierarchical_archon_md(tmp.path());
    assert!(
        result.is_empty(),
        "expected empty string for dir with no ARCHON.md"
    );
}

#[test]
fn project_archonmd_loaded() {
    let tmp = create_temp_dir();
    fs::write(tmp.path().join("ARCHON.md"), "# Project rules").unwrap();
    let result = load_hierarchical_archon_md(tmp.path());
    assert!(
        result.contains("# Project rules"),
        "project ARCHON.md content should appear in output"
    );
}

#[test]
fn dot_archon_preferred() {
    let tmp = create_temp_dir();
    // Both exist; .archon/ARCHON.md should be preferred
    fs::write(tmp.path().join("ARCHON.md"), "plain version").unwrap();
    fs::create_dir_all(tmp.path().join(".archon")).unwrap();
    fs::write(tmp.path().join(".archon/ARCHON.md"), "dotarchon version").unwrap();

    let result = load_hierarchical_archon_md(tmp.path());
    assert!(
        result.contains("dotarchon version"),
        "should contain .archon/ARCHON.md content"
    );
    assert!(
        !result.contains("plain version"),
        "plain ARCHON.md should be skipped when .archon/ARCHON.md exists"
    );
}

#[test]
fn hierarchy_order() {
    // parent/child structure: parent has ARCHON.md, child has ARCHON.md
    let tmp = create_temp_dir();
    let parent = tmp.path().join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).unwrap();

    fs::write(parent.join("ARCHON.md"), "PARENT_CONTENT").unwrap();
    fs::write(child.join("ARCHON.md"), "CHILD_CONTENT").unwrap();

    let result = load_hierarchical_archon_md(&child);
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
    fs::write(tmp.path().join("ARCHON.md"), "UNIQUE_CONTENT").unwrap();

    let result = load_hierarchical_archon_md(tmp.path());
    let count = result.matches("UNIQUE_CONTENT").count();
    assert_eq!(
        count, 1,
        "content should appear exactly once even if paths resolve to same file"
    );
}

#[test]
fn section_headers_present() {
    let tmp = create_temp_dir();
    fs::write(tmp.path().join("ARCHON.md"), "some content").unwrap();

    let result = load_hierarchical_archon_md(tmp.path());
    assert!(
        result.contains("# ARCHON.md from"),
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
    fs::write(parent.join("ARCHON.md"), &parent_content).unwrap();
    fs::write(child.join("ARCHON.md"), child_content).unwrap();

    // Limit that can only fit child content + header
    let result = load_hierarchical_archon_md_with_limit(&child, 200);
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
    fs::write(tmp.path().join("ARCHON.md"), [0xFF, 0xFE, 0x00, 0x01]).unwrap();

    // Should not panic, should return empty or skip gracefully
    let result = load_hierarchical_archon_md(tmp.path());
    // Binary file should be skipped -- result may be empty or just a header
    // The key is that it doesn't panic
    assert!(
        !result.contains("\u{FFFD}"),
        "should not contain replacement characters from binary data"
    );
}

// --- Backward compatibility tests ---

#[test]
fn backward_compat_claude_md_fallback() {
    let tmp = create_temp_dir();
    // Only old-style CLAUDE.md exists, no ARCHON.md
    fs::write(tmp.path().join("CLAUDE.md"), "# Legacy rules").unwrap();

    let result = load_hierarchical_archon_md(tmp.path());
    assert!(
        result.contains("# Legacy rules"),
        "should load CLAUDE.md as fallback when ARCHON.md is absent"
    );
}

#[test]
fn backward_compat_dot_claude_fallback() {
    let tmp = create_temp_dir();
    // Only old-style .claude/CLAUDE.md exists
    fs::create_dir_all(tmp.path().join(".claude")).unwrap();
    fs::write(tmp.path().join(".claude/CLAUDE.md"), "# Legacy dot rules").unwrap();

    let result = load_hierarchical_archon_md(tmp.path());
    assert!(
        result.contains("# Legacy dot rules"),
        "should load .claude/CLAUDE.md as fallback when .archon/ARCHON.md is absent"
    );
}

#[test]
fn new_path_takes_precedence_over_old() {
    let tmp = create_temp_dir();
    // Both old and new exist -- new should win
    fs::create_dir_all(tmp.path().join(".archon")).unwrap();
    fs::write(tmp.path().join(".archon/ARCHON.md"), "NEW_CONTENT").unwrap();
    fs::create_dir_all(tmp.path().join(".claude")).unwrap();
    fs::write(tmp.path().join(".claude/CLAUDE.md"), "OLD_CONTENT").unwrap();

    let result = load_hierarchical_archon_md(tmp.path());
    assert!(
        result.contains("NEW_CONTENT"),
        ".archon/ARCHON.md should be loaded"
    );
    assert!(
        !result.contains("OLD_CONTENT"),
        ".claude/CLAUDE.md should NOT be loaded when .archon/ARCHON.md exists"
    );
}

#[test]
fn archon_md_root_takes_precedence_over_claude_md_root() {
    let tmp = create_temp_dir();
    fs::write(tmp.path().join("ARCHON.md"), "NEW_ROOT").unwrap();
    fs::write(tmp.path().join("CLAUDE.md"), "OLD_ROOT").unwrap();

    let result = load_hierarchical_archon_md(tmp.path());
    assert!(
        result.contains("NEW_ROOT"),
        "ARCHON.md should be loaded"
    );
    assert!(
        !result.contains("OLD_ROOT"),
        "CLAUDE.md should NOT be loaded when ARCHON.md exists"
    );
}
