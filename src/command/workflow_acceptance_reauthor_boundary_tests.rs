//! #236: the re-author runs in the repository and is confined to it, so
//! every place its prompt tells it to read must be in the call's read roots.
//! These tests build the real re-author call. They take every path the
//! prompt names and check it with the same path guard the agent's `Read`
//! uses, under the context a workspace boundary gives the agent: its
//! working directory plus its read roots, and nothing inherited.

use std::sync::Mutex;

use archon_tools::tool::ToolContext;
use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::{
    WorkflowAgentCall, WorkflowAgentOutcome, WorkflowLlmClient,
};
use async_trait::async_trait;
use serde_json::Value;

use super::*;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set;

/// Records the call and returns an empty reply. The test needs only the call.
#[derive(Default)]
struct Capture(Mutex<Vec<WorkflowAgentCall>>);

#[async_trait]
impl WorkflowLlmClient for Capture {
    async fn run_agent(&self, call: WorkflowAgentCall) -> WorkflowResult<WorkflowAgentOutcome> {
        self.0.lock().unwrap().push(call);
        Ok(WorkflowAgentOutcome {
            content: "{}".into(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }

    async fn send_message(
        &self,
        _messages: Vec<Value>,
        _system: Vec<Value>,
        _tools: Vec<Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the author call is all this test makes")
    }
}

/// Every absolute path the prompt names that exists on disk under `root`.
/// Trailing sentence punctuation is not part of the path.
fn prompt_paths(prompt: &str, root: &Path) -> Vec<PathBuf> {
    let root = root.display().to_string();
    let mut paths = Vec::new();
    for token in prompt.split_whitespace() {
        let token = token.trim_matches(|c: char| "\"'`,;:()[]{}".contains(c));
        let token = token.trim_end_matches('.');
        if !token.starts_with(&root) {
            continue;
        }
        let path = PathBuf::from(token);
        if path.exists() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// The context a workspace boundary gives the agent for this call.
fn bounded_context(call: &WorkflowAgentCall) -> ToolContext {
    ToolContext {
        working_dir: call
            .cwd
            .clone()
            .expect("the re-author runs in the repository"),
        extra_dirs: call.read_roots.iter().map(PathBuf::from).collect(),
        ..ToolContext::default()
    }
}

async fn author_call(scope: &AuthorScope) -> WorkflowAgentCall {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let frozen = set.contract().acceptance[0].clone();
    let client = Capture::default();
    super::author::author_entry(&client, scope, &frozen, &["finding".into()], 1)
        .await
        .expect("the author call returns");
    let calls = client.0.into_inner().unwrap();
    assert_eq!(calls.len(), 1);
    calls.into_iter().next().unwrap()
}

#[tokio::test]
async fn every_path_the_reauthor_prompt_names_is_readable_under_the_boundary() {
    let root = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(root.path()).unwrap();
    let project = root.join("project");
    let repository = root.join("repository");
    let prd = project.join("prds").join("PRD-F.md");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::write(&prd, "requirements").unwrap();
    let scope = AuthorScope {
        prd_path: prd.clone(),
        project_root: project.clone(),
        repository_root: repository.clone(),
    };

    let call = author_call(&scope).await;
    let ctx = bounded_context(&call);
    let named = prompt_paths(&call.task, &root);
    // The prompt still names the three places, so the scan below checks them.
    for place in [&prd, &project, &repository] {
        assert!(
            named.contains(place),
            "the prompt no longer names {place:?}"
        );
    }
    for path in &named {
        archon_tools::path_guard_probe::probe_read_access(path, &ctx).unwrap_or_else(|refusal| {
            panic!(
                "the prompt sends the author to {path:?}, and the boundary refuses it: {refusal}"
            )
        });
    }
    // Only what the prompt names: the repository's parent directory, which
    // the prompt does not name, stays out.
    assert!(archon_tools::path_guard_probe::probe_read_access(&root, &ctx).is_err());
}

#[tokio::test]
async fn a_prd_outside_the_project_root_is_still_readable() {
    let root = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(root.path()).unwrap();
    let project = root.join("project");
    let repository = root.join("repository");
    let prd = root.join("elsewhere").join("PRD-F.md");
    for dir in [&project, &repository, &prd.parent().unwrap().to_path_buf()] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&prd, "requirements").unwrap();
    let scope = AuthorScope {
        prd_path: prd.clone(),
        project_root: project,
        repository_root: repository,
    };

    let call = author_call(&scope).await;
    assert!(
        archon_tools::path_guard_probe::probe_read_access(&prd, &bounded_context(&call)).is_ok()
    );
}

#[tokio::test]
async fn a_scope_inside_the_repository_needs_no_read_roots() {
    let root = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(root.path()).unwrap();
    let prd = root.join("prds").join("PRD-F.md");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::write(&prd, "requirements").unwrap();
    let scope = AuthorScope {
        prd_path: prd,
        project_root: root.clone(),
        repository_root: root,
    };
    assert!(scope.read_roots().expect("a valid scope").is_empty());
}

/// An empty field fails at the scope, by name, before any spawn. It no
/// longer turns into `""` and fails later as "not an absolute path".
#[tokio::test]
async fn an_empty_scope_field_fails_by_name_before_the_author_is_spawned() {
    let root = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(root.path()).unwrap();
    let prd = root.join("PRD-F.md");
    std::fs::write(&prd, "requirements").unwrap();
    let valid = || AuthorScope {
        prd_path: prd.clone(),
        project_root: root.clone(),
        repository_root: root.join("repository"),
    };
    for (field, scope) in [
        (
            "prd_path",
            AuthorScope {
                prd_path: PathBuf::new(),
                ..valid()
            },
        ),
        (
            "project_root",
            AuthorScope {
                project_root: PathBuf::new(),
                ..valid()
            },
        ),
        (
            "repository_root",
            AuthorScope {
                repository_root: PathBuf::new(),
                ..valid()
            },
        ),
    ] {
        let error = scope.read_roots().expect_err("an empty field");
        assert!(error.contains(field) && error.contains("empty"), "{error}");
    }
    // The empty PRD path a contract can carry, once joined to the project.
    let joined = AuthorScope {
        prd_path: root.join(""),
        ..valid()
    };
    let error = joined.read_roots().expect_err("a directory is not a PRD");
    assert!(
        error.contains("prd_path") && error.contains("directory"),
        "{error}"
    );

    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let frozen = set.contract().acceptance[0].clone();
    let client = Capture::default();
    let empty = AuthorScope {
        prd_path: PathBuf::new(),
        ..valid()
    };
    let error = super::author::author_entry(&client, &empty, &frozen, &[], 1)
        .await
        .expect_err("the call is refused");
    assert!(
        format!("{error:#}").contains("prd_path is empty"),
        "{error:#}"
    );
    assert!(
        client.0.lock().unwrap().is_empty(),
        "the author was spawned"
    );
}
