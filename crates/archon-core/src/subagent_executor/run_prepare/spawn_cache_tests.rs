//! Issue #171 Parts 5 + 6 spawn fixture.
//!
//! Five spawns of the same agent type against one executor must:
//!
//! - read the ARCHON.md hierarchy **once** (four cache hits after the cold
//!   load), and
//! - run the agent's `recall_queries` against the memory store **once**,
//!
//! while producing a system prompt byte-identical to the uncached composition.

use std::sync::Arc;

use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_memory::MemoryTrait;
use tempfile::TempDir;

use crate::agent::AgentConfig;
use crate::agents::definition::AgentMemoryScope;
use crate::agents::memory::tests::helpers::MockMemory;
use crate::agents::{AgentRegistry, CustomAgentDefinition};
use crate::dispatch::ToolRegistry;
use crate::subagent::SubagentManager;
use crate::subagent_executor::AgentSubagentExecutor;

const SPAWNS: usize = 5;

struct StubLlmProvider;

#[async_trait::async_trait]
impl LlmProvider for StubLlmProvider {
    fn name(&self) -> &str {
        "stub"
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
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }
    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!("stub provider is never driven by the prompt-assembly fixture")
    }
}

/// A working directory with its own ARCHON.md, plus the executor under test.
fn fixture(memory: Option<Arc<dyn MemoryTrait>>) -> (AgentSubagentExecutor, TempDir) {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("ARCHON.md"),
        "# fixture project rules\nnever reopen the transcript per message\n",
    )
    .unwrap();

    let executor = AgentSubagentExecutor::new(
        Arc::new(StubLlmProvider),
        ToolRegistry::new(),
        Arc::new(tokio::sync::Mutex::new(SubagentManager::new(16))),
        Arc::new(std::sync::RwLock::new(AgentRegistry::load_with_user_home(
            tmp.path(),
            None,
        ))),
        None,
        memory,
        tmp.path().to_path_buf(),
        "spawn-cache-fixture".into(),
        "claude-sonnet-4-6".into(),
        vec![],
        Arc::new(tokio::sync::Mutex::new("default".to_string())),
        Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "spawn-cache-fixture".into(),
            String::new(),
            String::new(),
        )),
    );
    (executor, tmp)
}

fn recalling_agent() -> CustomAgentDefinition {
    CustomAgentDefinition {
        agent_type: "fixture-reviewer".into(),
        system_prompt: "You review code.".into(),
        memory_scope: Some(AgentMemoryScope::Project),
        recall_queries: vec![
            "past review corrections".into(),
            "known pitfalls".into(),
            "team conventions".into(),
        ],
        ..Default::default()
    }
}

fn request() -> archon_tools::agent_tool::SubagentRequest {
    archon_tools::agent_tool::SubagentRequest {
        prompt: "review the diff".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: Some("fixture-reviewer".into()),
        run_in_background: false,
        cwd: None,
        isolation: None,
        provider_env: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn five_spawns_read_archon_md_once_and_recall_once() {
    let memory = Arc::new(MockMemory::new());
    memory.set_search_results(&["prefer held writers over reopen-per-message"]);
    let (executor, _tmp) = fixture(Some(memory.clone() as Arc<dyn MemoryTrait>));

    let def = recalling_agent();
    let request = request();

    let prompts: Vec<String> = (0..SPAWNS)
        .map(|_| executor.assemble_system_prompt(&request, Some(&def)))
        .collect();

    // Part 5: the hierarchy is read on the cold spawn only.
    let md = executor.archon_md_cache_stats();
    assert_eq!(md.misses, 1, "ARCHON.md hierarchy must be read once");
    assert_eq!(md.hits, SPAWNS - 1, "later spawns must be mtime cache hits");
    assert!(
        md.files_read >= 1,
        "the cold spawn must actually read files"
    );

    // Part 6: the recall queries run on the cold spawn only.
    let recall = executor.recall_cache_stats();
    assert_eq!(recall.misses, 1, "memory recall must be queried once");
    assert_eq!(recall.hits, SPAWNS - 1);
    assert_eq!(recall.queries_run, def.recall_queries.len());
    assert_eq!(
        memory.search_count(),
        def.recall_queries.len(),
        "{SPAWNS} spawns must issue {} store searches, not {}",
        def.recall_queries.len(),
        def.recall_queries.len() * SPAWNS
    );

    // Every spawn still gets the same prompt, and it still carries both blocks.
    for prompt in &prompts {
        assert_eq!(prompt, &prompts[0], "caching must not perturb the prompt");
        assert!(prompt.contains("<archon-md>"));
        assert!(prompt.contains("fixture project rules"));
        assert!(prompt.contains("<agent-memory>"));
        assert!(prompt.contains("prefer held writers over reopen-per-message"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cached_prompt_is_identical_to_the_uncached_composition() {
    let memory = Arc::new(MockMemory::new());
    memory.set_search_results(&["row one", "row two"]);
    let (executor, tmp) = fixture(Some(memory.clone() as Arc<dyn MemoryTrait>));

    let def = recalling_agent();
    let request = request();

    // Compose what the pre-#171 code produced: load the hierarchy and run the
    // recall queries directly, then wrap them the way `assemble_system_prompt`
    // does.
    let archon_md = crate::archonmd::load_hierarchical_archon_md(tmp.path());
    let memories = crate::agents::memory::load_agent_memory(
        &def.agent_type,
        &def.recall_queries,
        memory.as_ref(),
        def.memory_scope.as_ref(),
    );
    let expected = format!(
        "{}\n\n<archon-md>\n{archon_md}\n</archon-md>\n\n<agent-memory>\n{}\n</agent-memory>",
        def.system_prompt,
        memories.join("\n---\n"),
    );

    let cold = executor.assemble_system_prompt(&request, Some(&def));
    let warm = executor.assemble_system_prompt(&request, Some(&def));

    // `with_file_memory` appends the file-backed memory prompt after these two
    // blocks; it is untouched by #171, so compare the prefix the caches own.
    assert!(
        cold.starts_with(&expected),
        "cold spawn diverged from the uncached composition:\n{cold}"
    );
    assert_eq!(
        warm, cold,
        "warm spawn must reproduce the cold spawn exactly"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_edited_archon_md_is_picked_up_by_the_next_spawn() {
    let (executor, tmp) = fixture(None);
    let def = CustomAgentDefinition {
        agent_type: "fixture-plain".into(),
        system_prompt: "base".into(),
        ..Default::default()
    };
    let request = request();

    let before = executor.assemble_system_prompt(&request, Some(&def));
    assert!(before.contains("fixture project rules"));

    std::fs::write(
        tmp.path().join("ARCHON.md"),
        "# fixture project rules, revision two, materially longer than before\n",
    )
    .unwrap();

    let after = executor.assemble_system_prompt(&request, Some(&def));
    assert!(after.contains("revision two"));
    assert_eq!(executor.archon_md_cache_stats().misses, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agents_that_omit_archon_md_do_not_consult_the_cache() {
    let (executor, _tmp) = fixture(None);
    let def = CustomAgentDefinition {
        agent_type: "fixture-omit".into(),
        system_prompt: "base".into(),
        omit_claude_md: true,
        ..Default::default()
    };

    let prompt = executor.assemble_system_prompt(&request(), Some(&def));
    assert!(!prompt.contains("<archon-md>"));
    let md = executor.archon_md_cache_stats();
    assert_eq!(md.misses, 0);
    assert_eq!(md.hits, 0);
}
