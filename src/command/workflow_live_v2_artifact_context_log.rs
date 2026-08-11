//! Record the project-artifact context each write call was dispatched with.
//!
//! When a branch is rejected for changing "files outside declared
//! target_files", the question is always which paths the host considered
//! writable — and the answer lived only in memory. By the time the failure is
//! read the worktree is deleted and the inputs are unreconstructible, which
//! turns diagnosis into guesswork and guesswork into a build-and-run per
//! theory. One append-only line per call makes it a fact instead.
//!
//! Best-effort by design: this is diagnostic evidence, never a gate. A failure
//! to write it must never fail the call it is describing.

use std::io::Write;

use archon_workflow::WorkflowV2ResultStore;
use archon_workflow::v2::project_artifacts::WorkflowV2ProjectArtifactContext;

const LOG_FILE: &str = "artifact-context.jsonl";

pub(super) fn record(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    repository_root: Option<&str>,
    context: &WorkflowV2ProjectArtifactContext,
) {
    let line = serde_json::json!({
        "call_id": call_id,
        "repository_root": repository_root,
        "project_root": context.project_root,
        "artifact_roots": context.artifact_roots,
        "artifact_paths": context.artifact_paths,
    });
    let Ok(mut text) = serde_json::to_string(&line) else {
        return;
    };
    text.push('\n');
    let path = store.root().join(LOG_FILE);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()));
}
