// TASK-P0-B.5 (#183): ApplyPatch tool.
//
// Accepts a unified-diff patch string + an absolute target path, parses
// the patch from the unified-diff format spec, verifies every context
// and removed line against the original file, then writes the patched
// content back atomically via `std::fs::write`.
//
// The unified-diff parser is written from the published format spec
// (POSIX `diff -u` output shape); no external crate is used. See
// `parser.rs` for the accepted grammar and `applier.rs` for the
// apply semantics.
//
// Security: the tool rejects relative paths. The caller must pass an
// absolute filesystem path; every other path shape returns an error
// before any IO.
//
// TASK-P0-B.5 refactor: the original single-file module was split
// into `parser`, `applier`, and this `mod.rs` owning the public
// `ApplyPatchTool` so every file stays under the 500-line
// FileSizeGuard threshold. Types `Hunk`, `HunkLine`, and the
// helpers `parse_hunks` / `apply_hunks` are `pub(super)` — they
// remain private to the `apply_patch` module but are visible to the
// in-module `tests` submodule via `use super::*;` re-exports below.
// Public API is unchanged: `archon_tools::apply_patch::ApplyPatchTool`.

mod applier;
mod parser;

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// Re-export submodule internals at the `apply_patch` module scope so
// the existing `#[cfg(test)] mod tests { use super::*; }` block keeps
// working byte-for-byte without per-test path edits. These are
// `use`-imports (not `pub use`); the items remain module-private.
use applier::apply_hunks;
use parser::parse_hunks;
#[cfg(test)]
use parser::HunkLine;

// ---------------------------------------------------------------------------
// Tool impl
// ---------------------------------------------------------------------------

pub struct ApplyPatchTool;

#[async_trait::async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "ApplyPatch"
    }

    fn description(&self) -> &str {
        "Apply a unified-diff patch to an absolute file path. \
         Parses the patch, verifies every context and removed line \
         against the current file content, then atomically replaces the \
         file with the patched result. Returns an error with a \
         diagnostic if any hunk's context does not match."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path of the file to patch."
                },
                "patch": {
                    "type": "string",
                    "description": "Unified-diff text. MUST start with '--- a/...' and '+++ b/...' headers followed by one or more '@@ -old_start,old_len +new_start,new_len @@' hunks. Context lines start with ' ', removed lines with '-', added lines with '+'."
                }
            },
            "required": ["path", "patch"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let path_str = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("path is required and must be a string"),
        };
        let patch_str = match input.get("patch").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::error("patch is required and must be a string"),
        };

        let path = Path::new(path_str);
        if !path.is_absolute() {
            return ToolResult::error(format!(
                "path must be absolute, got relative path: {path_str}"
            ));
        }

        let original = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => return ToolResult::error(format!("Failed to read file {path_str}: {e}")),
        };

        let hunks = match parse_hunks(patch_str) {
            Ok(h) => h,
            Err(e) => return ToolResult::error(format!("Failed to parse patch: {e}")),
        };

        let patched = match apply_hunks(&original, &hunks) {
            Ok(s) => s,
            Err(e) => return ToolResult::error(format!("Failed to apply patch: {e}")),
        };

        if let Err(e) = fs::write(path, &patched) {
            return ToolResult::error(format!("Failed to write file {path_str}: {e}"));
        }

        ToolResult::success(format!(
            "Applied {} hunk{} to {path_str}",
            hunks.len(),
            if hunks.len() == 1 { "" } else { "s" }
        ))
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        // Matches EditTool (file_edit.rs): filesystem writes are Risky
        // but not Dangerous — the caller owns the path and diff.
        PermissionLevel::Risky
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test".into(),
            ..Default::default()
        }
    }

    // ----- parse_hunks -----

    #[test]
    fn parse_hunks_empty_patch_errors() {
        let err = parse_hunks("").unwrap_err();
        assert!(err.contains("empty"), "unexpected error: {err}");
    }

    #[test]
    fn parse_hunks_missing_header_errors() {
        // Has hunk but no `---`/`+++` file headers.
        let patch = "@@ -1,1 +1,1 @@\n-foo\n+bar\n";
        let err = parse_hunks(patch).unwrap_err();
        assert!(
            err.contains("file headers"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_single_hunk_headers() {
        let patch = "\
--- a/x.txt
+++ b/x.txt
@@ -1,2 +1,2 @@
 keep
-old
+new
";
        let hunks = parse_hunks(patch).expect("parses");
        assert_eq!(hunks.len(), 1);
        let h = &hunks[0];
        assert_eq!(h.old_start, 1);
        assert_eq!(h.old_len, 2);
        assert_eq!(h.new_start, 1);
        assert_eq!(h.new_len, 2);
        assert_eq!(h.lines.len(), 3);
        assert_eq!(h.lines[0], HunkLine::Context("keep".into()));
        assert_eq!(h.lines[1], HunkLine::Remove("old".into()));
        assert_eq!(h.lines[2], HunkLine::Add("new".into()));
    }

    #[test]
    fn parse_hunk_header_without_length_defaults_to_one() {
        let patch = "\
--- a/x.txt
+++ b/x.txt
@@ -3 +3 @@
-foo
+bar
";
        let hunks = parse_hunks(patch).expect("parses");
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_start, 3);
        assert_eq!(hunks[0].old_len, 1);
        assert_eq!(hunks[0].new_start, 3);
        assert_eq!(hunks[0].new_len, 1);
    }

    #[test]
    fn parse_hunks_ignores_backslash_metadata() {
        let patch = "\
--- a/x.txt
+++ b/x.txt
@@ -1,1 +1,1 @@
-old
\\ No newline at end of file
+new
";
        let hunks = parse_hunks(patch).expect("parses");
        assert_eq!(hunks.len(), 1);
        // Metadata line skipped — body has exactly Remove + Add.
        assert_eq!(hunks[0].lines.len(), 2);
    }

    // ----- apply_hunks -----

    #[test]
    fn apply_single_hunk_replacement() {
        let original = "keep\nold\n";
        let patch = "\
--- a/x.txt
+++ b/x.txt
@@ -1,2 +1,2 @@
 keep
-old
+new
";
        let hunks = parse_hunks(patch).unwrap();
        let out = apply_hunks(original, &hunks).unwrap();
        assert_eq!(out, "keep\nnew\n");
    }

    #[test]
    fn apply_multi_hunk_patch() {
        // Two hunks: change line 2 and line 5.
        let original = "a\nb\nc\nd\ne\n";
        let patch = "\
--- a/x.txt
+++ b/x.txt
@@ -1,3 +1,3 @@
 a
-b
+B
 c
@@ -4,2 +4,2 @@
 d
-e
+E
";
        let hunks = parse_hunks(patch).unwrap();
        assert_eq!(hunks.len(), 2);
        let out = apply_hunks(original, &hunks).unwrap();
        assert_eq!(out, "a\nB\nc\nd\nE\n");
    }

    #[test]
    fn apply_hunks_context_mismatch_errors() {
        let original = "keep\nactual\n";
        let patch = "\
--- a/x.txt
+++ b/x.txt
@@ -1,2 +1,2 @@
 keep
-expected
+new
";
        let hunks = parse_hunks(patch).unwrap();
        let err = apply_hunks(original, &hunks).unwrap_err();
        assert!(err.contains("mismatch"), "unexpected error: {err}");
    }

    #[test]
    fn apply_hunks_preserves_no_trailing_newline() {
        let original = "keep\nold"; // no trailing newline
        let patch = "\
--- a/x.txt
+++ b/x.txt
@@ -1,2 +1,2 @@
 keep
-old
+new
";
        let hunks = parse_hunks(patch).unwrap();
        let out = apply_hunks(original, &hunks).unwrap();
        assert_eq!(out, "keep\nnew");
    }

    // ----- execute (end-to-end) -----

    #[tokio::test]
    async fn execute_happy_path_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("x.txt");
        fs::write(&file_path, "keep\nold\n").unwrap();

        let patch = "\
--- a/x.txt
+++ b/x.txt
@@ -1,2 +1,2 @@
 keep
-old
+new
";

        let tool = ApplyPatchTool;
        let result = tool
            .execute(
                json!({ "path": file_path.to_str().unwrap(), "patch": patch }),
                &ctx(),
            )
            .await;
        assert!(!result.is_error, "execute failed: {}", result.content);
        assert!(result.content.contains("Applied 1 hunk"));
        let on_disk = fs::read_to_string(&file_path).unwrap();
        assert_eq!(on_disk, "keep\nnew\n");
    }

    #[tokio::test]
    async fn execute_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("nope.txt");
        let patch = "\
--- a/nope.txt
+++ b/nope.txt
@@ -1,1 +1,1 @@
-a
+b
";
        let tool = ApplyPatchTool;
        let result = tool
            .execute(
                json!({ "path": file_path.to_str().unwrap(), "patch": patch }),
                &ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("Failed to read file"),
            "unexpected content: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn execute_relative_path_errors() {
        let tool = ApplyPatchTool;
        let result = tool
            .execute(
                json!({ "path": "relative/x.txt", "patch": "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n" }),
                &ctx(),
            )
            .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("absolute"),
            "unexpected content: {}",
            result.content
        );
    }

    #[test]
    fn permission_level_is_risky() {
        let tool = ApplyPatchTool;
        assert_eq!(
            tool.permission_level(&json!({})),
            PermissionLevel::Risky
        );
    }
}
