//! TASK-P1-6 (#191) — apply_patch round-trip verification.
//!
//! Applies a multi-hunk unified-diff patch to a temp-copied file tree,
//! then applies the REVERSE patch, and asserts the final file SHA256
//! matches the original SHA256 byte-for-byte.
//!
//! Multi-hunk shape exercised here: `@@ -1,3 +1,3 @@ ... @@ -15,3 +15,3 @@ ... @@ -30,3 +30,3 @@`
//! — three separate `@@ -<...>,<...> +<...>,<...> @@` hunks in one patch.

use archon_tools::tool::{Tool, ToolContext};
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

#[tokio::test]
async fn apply_patch_forward_then_reverse_byte_identical() {
    // Source file — 50-line deterministic Rust content, ASCII only for
    // stable byte counts.
    let original: String = (0..50)
        .map(|i| format!("// line {i:02} — original content\n"))
        .collect();

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("src.rs");
    std::fs::write(&path, original.as_bytes()).unwrap();

    let original_hash = hash(original.as_bytes());

    // Multi-hunk forward patch — 3 hunks changing 3 separate lines.
    // Each hunk has 1 context line, 1 remove, 1 add.
    let forward = r#"--- a/src.rs
+++ b/src.rs
@@ -1,3 +1,3 @@
 // line 00 — original content
-// line 01 — original content
+// line 01 — MODIFIED content
 // line 02 — original content
@@ -15,3 +15,3 @@
 // line 14 — original content
-// line 15 — original content
+// line 15 — MODIFIED content
 // line 16 — original content
@@ -30,3 +30,3 @@
 // line 29 — original content
-// line 30 — original content
+// line 30 — MODIFIED content
 // line 31 — original content
"#;

    // Reverse patch — swap +/- and hunk numbering stays same.
    let reverse = r#"--- a/src.rs
+++ b/src.rs
@@ -1,3 +1,3 @@
 // line 00 — original content
-// line 01 — MODIFIED content
+// line 01 — original content
 // line 02 — original content
@@ -15,3 +15,3 @@
 // line 14 — original content
-// line 15 — MODIFIED content
+// line 15 — original content
 // line 16 — original content
@@ -30,3 +30,3 @@
 // line 29 — original content
-// line 30 — MODIFIED content
+// line 30 — original content
 // line 31 — original content
"#;

    let tool = archon_tools::apply_patch::ApplyPatchTool;
    let ctx = ToolContext {
        working_dir: tmp.path().to_path_buf(),
        session_id: "roundtrip-test".to_string(),
        ..Default::default()
    };

    // Forward apply.
    let fwd_result = tool
        .execute(
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "patch": forward,
            }),
            &ctx,
        )
        .await;
    assert!(
        !fwd_result.is_error,
        "forward apply failed: {}",
        fwd_result.content
    );

    // Post-forward hash — MUST differ from original.
    let after_fwd = std::fs::read(&path).unwrap();
    let fwd_hash = hash(&after_fwd);
    assert_ne!(
        fwd_hash, original_hash,
        "forward patch was a no-op (hashes match); diff ineffective"
    );

    // Reverse apply.
    let rev_result = tool
        .execute(
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "patch": reverse,
            }),
            &ctx,
        )
        .await;
    assert!(
        !rev_result.is_error,
        "reverse apply failed: {}",
        rev_result.content
    );

    // Post-reverse hash — MUST equal original.
    let after_rev = std::fs::read(&path).unwrap();
    let rev_hash = hash(&after_rev);
    assert_eq!(
        rev_hash, original_hash,
        "round-trip failed: after forward+reverse, file hash differs from original. \
         Original: {original_hash}\nAfter: {rev_hash}\nBytes: {after_rev:?}"
    );

    // Belt-and-braces: byte-for-byte compare.
    assert_eq!(
        after_rev.as_slice(),
        original.as_bytes(),
        "round-trip produced non-identical bytes"
    );
}
