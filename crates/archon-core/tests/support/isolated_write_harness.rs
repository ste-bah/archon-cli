//! Shared by the Issue-213 C3 end-to-end tests: a provider that asks for a
//! fixed list of `Write`s, one per turn, and the real executor that runs it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::dispatch::ToolRegistry;
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use archon_tools::subagent_executor::SubagentExecutor;
use archon_tools::subagent_request::SubagentRequest;
use archon_tools::tool::ToolContext;

/// One `Write` per turn, in order, then a final text turn.
struct WritesThenText {
    writes: Vec<serde_json::Value>,
    calls: AtomicU32,
}

#[async_trait::async_trait]
impl LlmProvider for WritesThenText {
    fn name(&self) -> &str {
        "mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) as usize;
        let mut events = vec![StreamEvent::MessageStart {
            id: format!("msg-{call}"),
            model: "mock".into(),
            usage: Usage::default(),
        }];
        if let Some(input) = self.writes.get(call) {
            events.extend([
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::ToolUse,
                    tool_use_id: Some(format!("tool-{call}")),
                    tool_name: Some("Write".into()),
                },
                StreamEvent::InputJsonDelta {
                    index: 0,
                    partial_json: input.to_string(),
                },
                StreamEvent::ContentBlockStop { index: 0 },
            ]);
        } else {
            events.extend([
                StreamEvent::ContentBlockStart {
                    index: 0,
                    block_type: ContentBlockType::Text,
                    tool_use_id: None,
                    tool_name: None,
                },
                StreamEvent::TextDelta {
                    index: 0,
                    text: "done".into(),
                },
                StreamEvent::ContentBlockStop { index: 0 },
            ]);
        }
        events.push(StreamEvent::MessageStop);
        let (tx, rx) = tokio::sync::mpsc::channel(events.len() + 1);
        for event in events {
            let _ = tx.send(event).await;
        }
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "{args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A canonical checkout with one commit, and a linked worktree of it.
pub fn checkout_and_worktree(root: &Path) -> (PathBuf, PathBuf) {
    let canonical = root.join("canonical");
    std::fs::create_dir_all(&canonical).expect("canonical");
    git(&canonical, &["init", "-q"]);
    git(&canonical, &["config", "user.email", "t@example.invalid"]);
    git(&canonical, &["config", "user.name", "t"]);
    std::fs::write(canonical.join("lib.txt"), "canonical\n").expect("seed");
    git(&canonical, &["add", "."]);
    git(&canonical, &["commit", "-qm", "base"]);
    let worktree = root.join("branch-worktree");
    git(
        &canonical,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "branch",
            worktree.to_str().unwrap(),
        ],
    );
    (canonical, worktree)
}

/// Run one child of `parent` through the real executor (whose own directory
/// is `executor_dir`), asking for `writes` of `(path, content)` in order.
pub async fn run_child(
    executor_dir: &Path,
    parent: ToolContext,
    cwd: Option<&Path>,
    isolation: Option<&str>,
    writes: &[(&Path, &str)],
) {
    let provider = Arc::new(WritesThenText {
        writes: writes
            .iter()
            .map(|(path, content)| {
                serde_json::json!({"file_path": path.display().to_string(), "content": content})
            })
            .collect(),
        calls: AtomicU32::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(archon_tools::file_write::WriteTool));
    let session = "subagent-isolated-write-session";
    let executor = Arc::new(AgentSubagentExecutor::new(
        provider,
        tools,
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(4))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(executor_dir))),
        None,
        None,
        executor_dir.to_path_buf(),
        session.into(),
        "mock-model".into(),
        vec![],
        Arc::new(tokio::sync::Mutex::new("bypassPermissions".to_string())),
        Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            session.into(),
            String::new(),
            String::new(),
        )),
    ));
    let request = SubagentRequest {
        prompt: "write the files".into(),
        model: None,
        allowed_tools: vec!["Write".into()],
        max_turns: 8,
        timeout_secs: 60,
        subagent_type: None,
        run_in_background: false,
        cwd: cwd.map(|dir| dir.display().to_string()),
        isolation: isolation.map(str::to_string),
        write_roots: Vec::new(),
        provider_env: None,
    };
    let outcome = executor
        .run_to_completion(
            uuid::Uuid::new_v4().to_string(),
            request,
            ToolContext {
                session_id: session.into(),
                ..parent
            },
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
    assert!(outcome.is_ok(), "{outcome:?}");
}

/// A workflow parent context: a run store is in scope.
pub fn workflow_parent(dir: &Path) -> ToolContext {
    ToolContext {
        working_dir: dir.to_path_buf(),
        run_store: Some(archon_tools::workflow_read_guard::RunStoreScope::default()),
        ..ToolContext::default()
    }
}

/// The canonical checkout reads back exactly as it was committed.
pub fn assert_unchanged(canonical: &Path) {
    assert_eq!(git(canonical, &["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read_to_string(canonical.join("lib.txt")).expect("read back"),
        "canonical\n"
    );
}

pub fn real_temp() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = archon_shell::paths::canonicalize(temp.path()).expect("real temp");
    (temp, root)
}
