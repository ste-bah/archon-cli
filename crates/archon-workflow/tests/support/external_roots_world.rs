//! Issue-226's world: the acceptance-grant world, plus directories outside
//! both the project and the repository, one of which the run's policy file
//! may allowlist (`observer_snapshot.native_execution.external_data_roots`,
//! as a launch records `[workflow.acceptance_execution]
//! external_data_roots`). Every check reads the files back.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::rc::Rc;

use archon_workflow::WorkflowV2ResultStore;
use serde_json::Value;

use super::harness::{EditsFn, Host, Verdict};
use super::support::{Edits, Fixture};
use super::world::{fixture, with_project_inputs};

/// The run policy key, as an operator writes it.
pub const KEY: &str = "workflow.acceptance_execution.external_data_roots";

/// Host directories outside the fixture's project and repository.
pub struct Outside {
    _temp: tempfile::TempDir,
    /// What the run's policy may list, canonical.
    pub allowed: PathBuf,
    /// What it never lists, canonical.
    pub elsewhere: PathBuf,
}

pub fn outside() -> Outside {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let allowed = base.join("allowed");
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    Outside {
        _temp: temp,
        allowed,
        elsewhere,
    }
}

/// A run whose policy file lists `allowed` and whose TASK-A declares
/// `artifacts`; `scratch` is the acceptance scratch parent.
pub fn run_with(scratch: &Path, allowed: &[&Path], artifacts: &[PathBuf]) -> Fixture {
    let mut f = fixture();
    with_project_inputs(&f, scratch, &[]);
    let path = f.store.run_dir(&f.run).join("v2/generated-metadata.json");
    let mut metadata: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    if !allowed.is_empty() {
        metadata["observer_snapshot"]["native_execution"]["external_data_roots"] =
            serde_json::json!(allowed);
    }
    std::fs::write(&path, serde_json::to_vec(&metadata).unwrap()).unwrap();
    f.universe.as_mut().unwrap().tasks[0].artifact_requirements = (artifacts.iter())
        .map(|path| path.display().to_string())
        .collect();
    f
}

/// Write `content` at `path`, its directories first.
pub fn put(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

pub fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// TASK-A's fix: its code, and -- on its first attempt only -- `content` at
/// its branch's copy of external file `target`.
pub fn fix_writing(target: &Path, content: &'static str) -> EditsFn {
    let copy = leak(format!("@copy:{}", target.display()));
    let first = std::cell::Cell::new(true);
    Box::new(move |_key, _round, _escalated| {
        let mut files = vec![("crates/a/src/lib.rs", "// a, fixed\n")];
        if first.replace(false) {
            files.push((copy, content));
        }
        Edits {
            files,
            report: vec!["crates/a/src/lib.rs"],
            via_adapter: false,
        }
    })
}

/// TASK-A's fix of its code alone.
pub fn fix_code() -> EditsFn {
    Box::new(|_key, _round, _escalated| Edits {
        files: vec![("crates/a/src/lib.rs", "// a, fixed\n")],
        report: vec!["crates/a/src/lib.rs"],
        via_adapter: false,
    })
}

/// A session over `f` answering `round` first, with TASK-A's verdicts.
pub fn session_with(f: Fixture, round: Value, edits: EditsFn, verdicts: Vec<Verdict>) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, edits));
    host.verdicts("TASK-A", verdicts);
    host.acceptance.borrow_mut().push_back(round);
    host
}

/// A review session over `f`, with TASK-A's verdicts.
pub fn session_review(f: Fixture, edits: EditsFn, verdicts: Vec<Verdict>) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, edits));
    host.verdicts("TASK-A", verdicts);
    host
}

/// Every line of the run's project-input landing log.
pub fn landings(f: &Fixture) -> Vec<Value> {
    let log = f
        .store
        .run_dir(&f.run)
        .join("write-coordination/project-inputs.jsonl");
    (read(&log).unwrap_or_default().lines())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The landing log's lines for `path`, as `(outcome, reason)`.
pub fn landed(f: &Fixture, path: &Path) -> Vec<(String, String)> {
    let path = path.display().to_string();
    (landings(f).into_iter())
        .filter(|line| line["path"] == path.as_str())
        .map(|line| {
            let text = |key: &str| line[key].as_str().unwrap_or_default().to_string();
            (text("outcome"), text("reason"))
        })
        .collect()
}

/// Every file the run kept of what its landings replaced, with its bytes.
pub fn kept_copies(f: &Fixture) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![
        f.store
            .run_dir(&f.run)
            .join("write-coordination/project-inputs-replaced"),
    ];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(text) = read(&path) {
                out.push((path, text));
            }
        }
    }
    out
}

pub fn failure_naming(paths: &[&Path]) -> String {
    let named: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    format!("AssertionError: {} disagree\n", named.join(" and "))
}
