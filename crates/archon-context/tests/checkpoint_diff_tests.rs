use archon_context::checkpoint_diff::{CheckpointEntry, diff_snapshots};

// ---------------------------------------------------------------------------
// diff_identical_files
// ---------------------------------------------------------------------------

#[test]
fn diff_identical_files() {
    let content = "line one\nline two\nline three\n";
    let result = diff_snapshots(content, content, "same.txt");
    // Identical files produce no hunk body (only possible header, but no +/- lines)
    let has_changes = result.lines().any(|l| {
        (l.starts_with('+') && !l.starts_with("+++"))
            || (l.starts_with('-') && !l.starts_with("---"))
    });
    assert!(
        !has_changes,
        "identical files should produce no change lines, got:\n{result}"
    );
}

// ---------------------------------------------------------------------------
// diff_added_lines
// ---------------------------------------------------------------------------

#[test]
fn diff_added_lines() {
    let before = "";
    let after = "alpha\nbeta\n";
    let result = diff_snapshots(before, after, "added.txt");
    assert!(
        result.contains("+alpha"),
        "should show added lines with '+' prefix, got:\n{result}"
    );
    assert!(
        result.contains("+beta"),
        "should show added lines with '+' prefix, got:\n{result}"
    );
}

// ---------------------------------------------------------------------------
// diff_removed_lines
// ---------------------------------------------------------------------------

#[test]
fn diff_removed_lines() {
    let before = "alpha\nbeta\n";
    let after = "";
    let result = diff_snapshots(before, after, "removed.txt");
    assert!(
        result.contains("-alpha"),
        "should show removed lines with '-' prefix, got:\n{result}"
    );
    assert!(
        result.contains("-beta"),
        "should show removed lines with '-' prefix, got:\n{result}"
    );
}

// ---------------------------------------------------------------------------
// diff_modified_lines
// ---------------------------------------------------------------------------

#[test]
fn diff_modified_lines() {
    let before = "hello world\n";
    let after = "hello rust\n";
    let result = diff_snapshots(before, after, "mod.txt");
    assert!(
        result.contains("-hello world"),
        "should show removed original line, got:\n{result}"
    );
    assert!(
        result.contains("+hello rust"),
        "should show added new line, got:\n{result}"
    );
}

// ---------------------------------------------------------------------------
// diff_format_unified
// ---------------------------------------------------------------------------

#[test]
fn diff_format_unified() {
    let before = "a\n";
    let after = "b\n";
    let result = diff_snapshots(before, after, "unified.txt");
    assert!(
        result.contains("--- a/unified.txt"),
        "should contain '---' header, got:\n{result}"
    );
    assert!(
        result.contains("+++ b/unified.txt"),
        "should contain '+++' header, got:\n{result}"
    );
    assert!(
        result.contains("@@"),
        "should contain '@@' hunk header, got:\n{result}"
    );
}

// ---------------------------------------------------------------------------
// checkpoint_entry_fields
// ---------------------------------------------------------------------------

#[test]
fn checkpoint_entry_fields() {
    let entry = CheckpointEntry {
        checkpoint_id: "cp-001".to_string(),
        file_path: "/src/main.rs".to_string(),
        timestamp: "2026-04-03T12:00:00Z".to_string(),
        size_bytes: 1024,
    };
    assert_eq!(entry.checkpoint_id, "cp-001");
    assert_eq!(entry.file_path, "/src/main.rs");
    assert_eq!(entry.timestamp, "2026-04-03T12:00:00Z");
    assert_eq!(entry.size_bytes, 1024);
}

// ---------------------------------------------------------------------------
// diff_empty_inputs
// ---------------------------------------------------------------------------

#[test]
fn diff_empty_inputs() {
    let result = diff_snapshots("", "", "empty.txt");
    let has_changes = result.lines().any(|l| {
        (l.starts_with('+') && !l.starts_with("+++"))
            || (l.starts_with('-') && !l.starts_with("---"))
    });
    assert!(
        !has_changes,
        "both empty should produce no change lines, got:\n{result}"
    );
}
