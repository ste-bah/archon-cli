//! Exact deliverable paths an artifact-only item is entitled to write.
//!
//! A task whose only output is a project artifact (a report, an audit, a
//! generated dataset) carries no repository `target_files`. The write-ownership
//! check then sees a changed file against an empty ownership list and rejects
//! the branch — "declares no target ownership" — so the agent is refused the
//! very deliverable it was dispatched to produce. Four consecutive runs died
//! there, on one markdown file.
//!
//! The earlier attempt admitted the *directory* of each declared contract as an
//! artifact root and then tried to keep source paths out with a repository
//! ownership guard. That was the wrong shape twice over: a directory is wider
//! than the deliverable, and the guard needed to answer "does the repo own
//! this?" — a question whose inputs (repository root, git availability inside a
//! transient worktree) vary per branch and cannot be reconstructed after the
//! fact.
//!
//! This admits the exact declared path instead, under two conditions that
//! together need no repository at all:
//!
//! 1. **The item is artifact-only** — no repository `target_files` AND concrete
//!    `artifact_requirements`, the same test the inventory gate applies in
//!    `generated_lifecycle_outcomes`. Both halves matter. An earlier version of
//!    this file checked only the absence of `target_files`, but that is a state
//!    a CODE item can legitimately be in: contract validation permits "no
//!    target_files + has a deliverable contract". Such an item inherited its
//!    task's source-file contracts as writable artifacts, reclassifying
//!    `crates/.../thing.rs` as a document and skipping write-ownership.
//! 2. **The path is host-parsed** — it appears as a `deliverable_contracts`
//!    entry for one of the item's canonical task ids in the authoritative task
//!    universe. Contracts are read from the task files by the host, never
//!    authored by an agent, so an agent cannot widen its own write rights by
//!    claiming a path.
//!
//! One exact file, declared by the task itself, for an item that can write
//! nothing else. Source paths declared by code tasks are untouched, because
//! those items carry `target_files`.

use serde_json::Value;

use crate::task_universe::WorkflowV2TaskUniverse;

/// Exact deliverable paths this item may write, or empty when it is not
/// artifact-only.
pub(crate) fn contract_artifact_paths_for_item(
    universe: &WorkflowV2TaskUniverse,
    item: &Value,
) -> Vec<String> {
    if !is_artifact_only(item) {
        return Vec::new();
    }
    let task_ids = canonical_task_ids(item);
    if task_ids.is_empty() {
        return Vec::new();
    }
    let mut paths: Vec<String> = Vec::new();
    for task in &universe.tasks {
        if !task_ids.iter().any(|id| id == &task.canonical_task_id) {
            continue;
        }
        for contract in &task.deliverable_contracts {
            let Some(path) = admissible_path(&contract.artifact_path) else {
                continue;
            };
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// The inventory gate's test, applied here so one definition governs both:
/// no repository `target_files` AND concrete `artifact_requirements`.
///
/// Absence of `target_files` alone is not enough. It is a state a code item can
/// hold, and admitting it hands that item's source-file deliverable contracts
/// out as writable artifacts.
fn is_artifact_only(item: &Value) -> bool {
    !declares_repository_targets(item) && declares_artifact_requirements(item)
}

/// Declared deliverables this item's tasks wrote as DIRECTORIES, normalised
/// without the trailing separator.
///
/// Read from the universe, where the separator the task author typed still
/// exists. Every downstream form loses it — `admissible_path` rebuilds with
/// `segments.join("/")` and `Path::join(..).display()` drops it again — so the
/// intent has to travel as its own value rather than as a character on a
/// string. A live task declares `.../coverage/history/` and was failed three
/// times for "is a directory, not the declared file", once even after a fix
/// that read the separator off a string it no longer had.
pub(crate) fn declared_directory_paths_for_item(
    universe: &WorkflowV2TaskUniverse,
    item: &Value,
) -> Vec<String> {
    let task_ids = canonical_task_ids(item);
    if task_ids.is_empty() {
        return Vec::new();
    }
    let mut paths: Vec<String> = Vec::new();
    for task in &universe.tasks {
        if !task_ids.iter().any(|id| id == &task.canonical_task_id) {
            continue;
        }
        for contract in &task.deliverable_contracts {
            let raw = contract.artifact_path.trim();
            if !raw.ends_with('/') && !raw.ends_with('\\') {
                continue;
            }
            if let Some(path) = admissible_path(raw)
                && !paths.contains(&path)
            {
                paths.push(path);
            }
        }
    }
    paths
}

fn declares_repository_targets(item: &Value) -> bool {
    match item.get("target_files") {
        None | Some(Value::Null) => false,
        Some(Value::Array(targets)) => targets
            .iter()
            .any(|target| target.as_str().is_some_and(|text| !text.trim().is_empty())),
        Some(_) => true,
    }
}

/// Concrete requirements — a non-empty array or object. An empty list declares
/// no work and must not qualify.
fn declares_artifact_requirements(item: &Value) -> bool {
    match item.get("artifact_requirements") {
        Some(Value::Array(requirements)) => !requirements.is_empty(),
        Some(Value::Object(requirements)) => !requirements.is_empty(),
        Some(Value::String(text)) => !text.trim().is_empty(),
        _ => false,
    }
}

fn canonical_task_ids(item: &Value) -> Vec<String> {
    item.get("canonical_task_ids")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// A declared path usable as an exact match, or `None` when it cannot name one
/// file: templated, glob, absolute, traversing, or empty.
fn admissible_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.contains("${")
        || trimmed.contains('*')
        || trimmed.contains('<')
        || std::path::Path::new(trimmed).is_absolute()
    {
        return None;
    }
    let segments: Vec<&str> = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.is_empty() || segments.contains(&"..") {
        return None;
    }
    Some(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_universe::{
        WorkflowV2DeliverableContract, WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask,
    };

    pub(super) fn universe_with(task_id: &str, paths: &[&str]) -> WorkflowV2TaskUniverse {
        WorkflowV2TaskUniverse {
            schema_version: "workflow-v2-task-universe-v1".to_string(),
            source_roots: Vec::new(),
            tasks: vec![WorkflowV2TaskUniverseTask {
                canonical_task_id: task_id.to_string(),
                deliverable_contracts: paths
                    .iter()
                    .map(|path| WorkflowV2DeliverableContract {
                        kind: "report".to_string(),
                        artifact_path: (*path).to_string(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
        }
    }

    fn artifact_only_item(task_id: &str) -> Value {
        serde_json::json!({
            "item_id": "impl-item",
            "canonical_task_ids": [task_id],
            "target_files": [],
            "artifact_requirements": ["a written gap audit"],
        })
    }

    /// The reachable hole: contract validation permits an item with no
    /// `target_files` that carries a deliverable contract, which is what a CODE
    /// task's items look like. Without concrete `artifact_requirements` it is
    /// not artifact-only, and must NOT be handed its task's source paths as
    /// writable artifacts — that would reclassify source as a document and skip
    /// write-ownership.
    #[test]
    fn a_code_item_without_target_files_is_not_artifact_only() {
        let universe = universe_with(
            "TASK-TDL-040",
            &["crates/archon-trading/src/data_lake/tradingview_mcp.rs"],
        );
        let item = serde_json::json!({
            "item_id": "impl-tdl-040",
            "canonical_task_ids": ["TASK-TDL-040"],
        });
        assert!(
            contract_artifact_paths_for_item(&universe, &item).is_empty(),
            "absent target_files alone must not grant source paths"
        );
        let empty_requirements = serde_json::json!({
            "item_id": "impl-tdl-040",
            "canonical_task_ids": ["TASK-TDL-040"],
            "target_files": [],
            "artifact_requirements": [],
        });
        assert!(
            contract_artifact_paths_for_item(&universe, &empty_requirements).is_empty(),
            "an empty requirements list declares no work of either kind"
        );
    }

    /// The live case: TASK-TDL-001 declares one report and no repo targets, so
    /// it is entitled to write exactly that file.
    #[test]
    fn an_artifact_only_item_may_write_its_declared_deliverable() {
        let universe = universe_with("TASK-TDL-001", &["docs/trading/data-lake-gap-audit.md"]);
        assert_eq!(
            contract_artifact_paths_for_item(&universe, &artifact_only_item("TASK-TDL-001")),
            vec!["docs/trading/data-lake-gap-audit.md".to_string()]
        );
    }

    /// An item WITH repository targets is ordinary repository work and gains
    /// nothing here — code tasks keep declared-target enforcement in full.
    #[test]
    fn an_item_with_repository_targets_gains_nothing() {
        let universe = universe_with("TASK-TDL-010", &["crates/thing/src/lib.rs"]);
        let item = serde_json::json!({
            "canonical_task_ids": ["TASK-TDL-010"],
            "target_files": ["crates/thing/src/lib.rs"],
        });
        assert!(contract_artifact_paths_for_item(&universe, &item).is_empty());
    }

    /// An agent cannot widen its own rights: a path it invents is not in the
    /// host-parsed universe, so it is never admitted.
    #[test]
    fn a_path_the_universe_does_not_declare_is_refused() {
        let universe = universe_with("TASK-TDL-001", &["docs/trading/data-lake-gap-audit.md"]);
        let item = serde_json::json!({
            "canonical_task_ids": ["TASK-TDL-001"],
            "target_files": [],
            "artifact_requirements": [{ "path": "crates/thing/src/lib.rs" }],
        });
        assert_eq!(
            contract_artifact_paths_for_item(&universe, &item),
            vec!["docs/trading/data-lake-gap-audit.md".to_string()],
            "only the host-parsed contract path is admitted"
        );
    }

    /// An item speaking for a different task gets that task's contracts only.
    #[test]
    fn contracts_are_matched_by_canonical_task_id() {
        let universe = universe_with("TASK-TDL-001", &["docs/trading/audit.md"]);
        assert!(
            contract_artifact_paths_for_item(&universe, &artifact_only_item("TASK-TDL-999"))
                .is_empty()
        );
    }

    /// Templated, glob, absolute, traversing and empty paths name no single
    /// file and are refused.
    #[test]
    fn unusable_shapes_are_refused() {
        let universe = universe_with(
            "TASK-TDL-001",
            &[
                "${PROJECT_ROOT}/out.json",
                "reports/*.md",
                "/etc/passwd",
                "../outside.md",
                "   ",
            ],
        );
        assert!(
            contract_artifact_paths_for_item(&universe, &artifact_only_item("TASK-TDL-001"))
                .is_empty()
        );
    }

    /// End-to-end against the REAL derivation, not a hand-built context: the
    /// context comes from `project_artifact_context_from_v2_root` on a store
    /// root shaped like a live run, and the deliverable is reported by absolute
    /// path exactly as the agent reports it. It must leave `files_changed`, or
    /// write-ownership rejects the branch with "declares no target ownership".
    #[test]
    fn the_declared_deliverable_is_reclassified_with_live_wiring() {
        use crate::v2::project_artifacts::{
            normalize_project_artifact_files, project_artifact_context_from_v2_root,
        };
        use crate::v2::result::{WorkflowV2FileRecord, WorkflowV2Result};

        let project = tempfile::tempdir().expect("project");
        let project_root = project.path().canonicalize().expect("canon");
        let v2_root = project_root.join(".archon/workflows/wf-live/v2");
        std::fs::create_dir_all(&v2_root).expect("mkdir v2");
        std::fs::create_dir_all(project_root.join("docs/trading")).expect("mkdir docs");
        let deliverable = project_root.join("docs/trading/data-lake-gap-audit.md");
        std::fs::write(&deliverable, "# audit\n").expect("write");

        let mut context = project_artifact_context_from_v2_root(&v2_root);
        context.add_contract_artifact_paths(
            &universe_with("TASK-TDL-001", &["docs/trading/data-lake-gap-audit.md"]),
            &artifact_only_item("TASK-TDL-001"),
        );

        let mut result = WorkflowV2Result::accepted("wrote the deliverable");
        result.files_changed = vec![WorkflowV2FileRecord::new(deliverable.display().to_string())];
        normalize_project_artifact_files("impl-tdl-001", &mut result, &context)
            .expect("classification");

        assert!(
            result.files_changed.is_empty(),
            "deliverable must leave files_changed: {:?}",
            result.files_changed
        );
        assert_eq!(result.artifacts.len(), 1, "it becomes a project artifact");
    }

    /// Control: a code path the item did not declare stays a repository change,
    /// so ownership enforcement still applies to everything else it touches.
    #[test]
    fn an_undeclared_path_remains_a_repository_change() {
        use crate::v2::project_artifacts::{
            normalize_project_artifact_files, project_artifact_context_from_v2_root,
        };
        use crate::v2::result::{WorkflowV2FileRecord, WorkflowV2Result};

        let project = tempfile::tempdir().expect("project");
        let project_root = project.path().canonicalize().expect("canon");
        let v2_root = project_root.join(".archon/workflows/wf-live/v2");
        std::fs::create_dir_all(&v2_root).expect("mkdir v2");
        std::fs::create_dir_all(project_root.join("crates/thing/src")).expect("mkdir");
        let code = project_root.join("crates/thing/src/lib.rs");
        std::fs::write(&code, "// code\n").expect("write");

        let mut context = project_artifact_context_from_v2_root(&v2_root);
        context.add_contract_artifact_paths(
            &universe_with("TASK-TDL-001", &["docs/trading/data-lake-gap-audit.md"]),
            &artifact_only_item("TASK-TDL-001"),
        );

        let mut result = WorkflowV2Result::accepted("touched code");
        result.files_changed = vec![WorkflowV2FileRecord::new(code.display().to_string())];
        normalize_project_artifact_files("impl-tdl-001", &mut result, &context)
            .expect("classification");

        assert_eq!(
            result.files_changed.len(),
            1,
            "an undeclared path must stay a repository change"
        );
    }
}
