//! Issue-213 C3, end to end: an agent spawned into a linked worktree of a
//! repository cannot write the repository's other checkout, the one its
//! parent works in, through its file tools. Driven through the real executor,
//! the real `Write` tool and a provider that asks for both writes; the
//! canonical checkout is read back afterwards.

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

fn git(dir: &Path, args: &[&str]) -> String {
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
fn checkout_and_worktree(root: &Path) -> (PathBuf, PathBuf) {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_agent_in_a_worktree_cannot_modify_the_canonical_checkout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(temp.path()).expect("real temp");
    let (canonical, worktree) = checkout_and_worktree(&root);
    let escaped = canonical.join("escaped.txt");
    let own = worktree.join("made-here.txt");
    let provider = Arc::new(WritesThenText {
        writes: vec![
            serde_json::json!({"file_path": escaped.display().to_string(), "content": "x\n"}),
            serde_json::json!({"file_path": own.display().to_string(), "content": "y\n"}),
        ],
        calls: AtomicU32::new(0),
    });
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(archon_tools::file_write::WriteTool));
    let session = "subagent-isolated-write-session";
    let executor = Arc::new(AgentSubagentExecutor::new(
        provider,
        tools,
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(4))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(&root))),
        None,
        None,
        canonical.clone(),
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
        max_turns: 6,
        timeout_secs: 60,
        subagent_type: None,
        run_in_background: false,
        cwd: Some(worktree.display().to_string()),
        isolation: None,
        write_roots: Vec::new(),
        provider_env: None,
    };
    let parent = ToolContext {
        working_dir: canonical.clone(),
        session_id: session.into(),
        ..ToolContext::default()
    };

    let outcome = executor
        .run_to_completion(
            uuid::Uuid::new_v4().to_string(),
            request,
            parent,
            tokio_util::sync::CancellationToken::new(),
        )
        .await;
    assert!(outcome.is_ok(), "{outcome:?}");

    // Read back: the canonical checkout is exactly as it was.
    assert!(!escaped.exists(), "the agent wrote the canonical checkout");
    assert_eq!(git(&canonical, &["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read_to_string(canonical.join("lib.txt")).expect("read back"),
        "canonical\n"
    );
    // And the agent was not simply stopped from writing: its own workspace
    // took the second write.
    assert_eq!(
        std::fs::read_to_string(&own).expect("worktree write"),
        "y\n"
    );
}
