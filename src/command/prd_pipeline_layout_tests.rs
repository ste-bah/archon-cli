//! The workflow PRD pipeline's output location, pinned against the engine's
//! discovery.
//!
//! `/workflow-prd-spec` writes its task files to
//! `archon_core::skills::workflow_prd_spec::workflow_task_dir(name)` — that is,
//! `tasks/PRD-<NAME>/`. The workflow engine finds task files by walking a named
//! directory with a single non-recursive read. Those two facts live in
//! different crates and, until this file existed, agreed only because two
//! documents said so.
//!
//! Every test here builds the directory from the skill's own constant rather
//! than from a literal, so changing where the skill writes without changing
//! what the engine walks fails here rather than at run time, where the symptom
//! is "found no parseable TASK-*.md files" against a directory full of them.

use std::fs;
use std::path::{Path, PathBuf};

use archon_core::skills::workflow_prd_spec::{TASK_ROOT, workflow_task_dir};

use crate::command::workflow_live::workflow_live_task_universe::{
    task_graph_from_root, task_requirement_claims_from_root,
};

const PRD_NAME: &str = "TRADING-DATA-LAKE-AHDM-001";

/// A minimal task file in the shape `workflow-prdtospec.md` teaches: fenced
/// yaml first block, all ten required keys, id matching the filename.
fn task_file(task_id: &str, depends_on: &str, implements: &str, anchor: &str) -> String {
    format!(
        "# {task_id} — Fixture\n\
         \n\
         ```yaml\n\
         task_id: {task_id}\n\
         prd: PRD-{PRD_NAME}\n\
         domain: TDL\n\
         title: Fixture\n\
         workstream: W1\n\
         complexity: small\n\
         status: pending\n\
         depends_on: {depends_on}\n\
         blocks: []\n\
         source_sections: ['6']\n\
         implements: {implements}\n\
         required_env_keys: []\n\
         required_tools: []\n\
         deliverable_contracts: []\n\
         ```\n\
         \n\
         ## Files Expected to Change\n\
         \n\
         - `{anchor}`\n\
         \n\
         ## Acceptance Criteria\n\
         \n\
         - The fixture parses.\n\
         \n\
         ## Focused Tests\n\
         \n\
         - `cargo test -p archon-core --lib skills`\n"
    )
}

/// Lay a task set down at exactly the path the skill tells the model to use.
fn write_task_set(root: &Path) -> PathBuf {
    let dir = root.join(workflow_task_dir(PRD_NAME));
    fs::create_dir_all(&dir).expect("create task directory");
    fs::write(
        dir.join("TASK-TDL-010-registry-schema.md"),
        task_file("TASK-TDL-010", "[]", "[REQ-DL-010]", "crates/a/src/lib.rs"),
    )
    .expect("write TASK-TDL-010");
    fs::write(
        dir.join("TASK-TDL-020-coverage-matrix.md"),
        task_file(
            "TASK-TDL-020",
            "['TASK-TDL-010']",
            "[REQ-DL-020, REQ-DL-021]",
            "crates/b/src/lib.rs",
        ),
    )
    .expect("write TASK-TDL-020");
    dir
}

#[test]
fn skill_task_dir_sits_directly_under_the_shared_tasks_root() {
    let dir = workflow_task_dir(PRD_NAME);
    assert_eq!(dir, format!("{TASK_ROOT}/PRD-{PRD_NAME}"));
    // One level under `tasks/`. Discovery is non-recursive, so a deeper
    // directory would not be walked at all.
    assert_eq!(dir.matches('/').count(), 1);
}

#[test]
fn a_task_set_written_where_the_skill_says_is_discovered_by_the_engine() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = write_task_set(temp.path());

    let graph = task_graph_from_root(&dir).expect("task set at tasks/PRD-<NAME>/ is discovered");

    let mut ids: Vec<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["TASK-TDL-010", "TASK-TDL-020"]);

    let downstream = graph
        .nodes
        .iter()
        .find(|node| node.id == "TASK-TDL-020")
        .expect("TASK-TDL-020 present");
    assert_eq!(downstream.depends_on, vec!["TASK-TDL-010".to_string()]);
}

#[test]
fn requirement_claims_are_read_from_the_same_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = write_task_set(temp.path());

    let claims = task_requirement_claims_from_root(&dir).expect("claims read");
    let mut claimed: Vec<String> = claims
        .iter()
        .flat_map(|claim| claim.implements.iter().cloned())
        .collect();
    claimed.sort();
    assert_eq!(claimed, vec!["REQ-DL-010", "REQ-DL-020", "REQ-DL-021"]);
}

#[test]
fn a_task_one_directory_deeper_is_not_found_at_all() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = write_task_set(temp.path());
    let nested = dir.join("phase1");
    fs::create_dir_all(&nested).expect("create nested dir");
    fs::write(
        nested.join("TASK-TDL-030-nested.md"),
        task_file("TASK-TDL-030", "[]", "[REQ-DL-030]", "crates/c/src/lib.rs"),
    )
    .expect("write nested task");

    let graph = task_graph_from_root(&dir).expect("flat tasks still parse");
    assert!(
        !graph.nodes.iter().any(|node| node.id == "TASK-TDL-030"),
        "a task one level deeper must not be discovered — discovery is a \
         single non-recursive read, so nesting loses the task silently"
    );
}

#[test]
fn an_empty_task_directory_is_refused_naming_the_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let dir = temp.path().join(workflow_task_dir(PRD_NAME));
    fs::create_dir_all(&dir).expect("create task directory");

    let error = task_graph_from_root(&dir).expect_err("an empty directory is refused");
    let rendered = error.to_string();
    assert!(
        rendered.contains("no TASK-*.md files found"),
        "unexpected error: {rendered}"
    );
    assert!(
        rendered.contains(&dir.display().to_string()),
        "the error must name the directory it walked: {rendered}"
    );
}
