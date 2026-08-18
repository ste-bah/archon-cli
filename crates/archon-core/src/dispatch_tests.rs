use super::*;
use archon_observability::{AgentActivityKind, InMemoryActivitySink};
use archon_tools::provider_env::{ProviderEnvPolicy, ProviderEnvSource};
use archon_tools::tool::{AgentMode, PermissionLevel};
use std::sync::Arc;

struct ReplaceTestTool(&'static str);

#[async_trait::async_trait]
impl Tool for ReplaceTestTool {
    fn name(&self) -> &str {
        "ReplaceTest"
    }

    fn description(&self) -> &str {
        self.0
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success(self.0)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[test]
fn replace_overwrites_existing_tool_registration() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReplaceTestTool("first")));
    registry.replace(Box::new(ReplaceTestTool("second")));

    let definition = registry
        .tool_definitions()
        .into_iter()
        .find(|tool| tool["name"] == "ReplaceTest")
        .expect("replacement tool should be registered");
    assert_eq!(definition["description"], "second");
}

#[test]
fn provider_env_overlay_preserves_registered_bash_policy() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::bash::BashTool {
        timeout_secs: 17,
        max_output_bytes: 23,
        safe_commands: vec!["echo safe".to_string()],
        risky_commands: vec!["echo risky".to_string()],
        dangerous_commands: vec!["echo dangerous".to_string()],
        provider_env: None,
        ..Default::default()
    }));
    let source = ProviderEnvSource::Policy(ProviderEnvPolicy::new(vec![
        "ARCHON_TEST_PROVIDER_KEY".to_string(),
    ]));

    assert!(registry.attach_provider_env_to_bash(source));
    let bash = registry.get("Bash").expect("Bash remains registered");
    assert_eq!(
        bash.permission_level(&serde_json::json!({"command": "echo safe value"})),
        PermissionLevel::Safe
    );
    assert_eq!(
        bash.permission_level(&serde_json::json!({"command": "echo risky value"})),
        PermissionLevel::Risky
    );
    assert_eq!(
        bash.permission_level(&serde_json::json!({"command": "echo dangerous value"})),
        PermissionLevel::Dangerous
    );
}

#[test]
fn default_registry_has_all_tools() {
    let working_dir = std::env::temp_dir();
    let registry = create_default_registry(working_dir, None);
    let names = registry.tool_names();

    // Core tools
    assert!(names.contains(&"Read"), "missing Read tool");
    assert!(names.contains(&"Write"), "missing Write tool");
    assert!(names.contains(&"Edit"), "missing Edit tool");
    assert!(names.contains(&"LargeEditBegin"), "missing LargeEditBegin");
    assert!(
        names.contains(&"LargeEditInsertAfter"),
        "missing LargeEditInsertAfter"
    );
    assert!(
        names.contains(&"LargeEditReplaceSection"),
        "missing LargeEditReplaceSection"
    );
    assert!(
        names.contains(&"LargeEditDeleteSection"),
        "missing LargeEditDeleteSection"
    );
    assert!(
        names.contains(&"LargeEditCommit"),
        "missing LargeEditCommit"
    );
    assert!(names.contains(&"LargeEditAbort"), "missing LargeEditAbort");
    assert!(names.contains(&"Glob"), "missing Glob tool");
    assert!(names.contains(&"Grep"), "missing Grep tool");
    assert!(names.contains(&"Bash"), "missing Bash tool");
    assert!(names.contains(&"Sleep"), "missing Sleep tool");
    assert!(names.contains(&"TodoWrite"), "missing TodoWrite tool");
    assert!(
        names.contains(&"AskUserQuestion"),
        "missing AskUserQuestion"
    );
    assert!(names.contains(&"EnterPlanMode"), "missing EnterPlanMode");
    assert!(names.contains(&"ExitPlanMode"), "missing ExitPlanMode");
    assert!(names.contains(&"WebFetch"), "missing WebFetch tool");
    assert!(names.contains(&"Config"), "missing Config tool");
    assert!(names.contains(&"Agent"), "missing Agent tool");
    assert!(names.contains(&"SendMessage"), "missing SendMessage tool");
    assert!(names.contains(&"NotebookEdit"), "missing NotebookEdit tool");
    assert!(names.contains(&"TaskCreate"), "missing TaskCreate tool");
    assert!(names.contains(&"TaskGet"), "missing TaskGet tool");
    assert!(names.contains(&"TaskUpdate"), "missing TaskUpdate tool");
    assert!(names.contains(&"TaskList"), "missing TaskList tool");
    assert!(names.contains(&"TaskStop"), "missing TaskStop tool");
    assert!(names.contains(&"TaskOutput"), "missing TaskOutput tool");
    assert!(
        names.contains(&"EnterWorktree"),
        "missing EnterWorktree tool"
    );
    assert!(names.contains(&"ExitWorktree"), "missing ExitWorktree tool");
    assert!(
        names.contains(&"ListMcpResources"),
        "missing ListMcpResources tool"
    );
    assert!(
        names.contains(&"ReadMcpResource"),
        "missing ReadMcpResource tool"
    );

    // TASK-CLI-500 Fix 3: previously missing tools now registered
    assert!(
        names.contains(&"CronCreate"),
        "missing CronCreate tool (Fix 3)"
    );
    assert!(names.contains(&"CronList"), "missing CronList tool (Fix 3)");
    assert!(
        names.contains(&"CronDelete"),
        "missing CronDelete tool (Fix 3)"
    );
    assert!(
        names.contains(&"TeamCreate"),
        "missing TeamCreate tool (Fix 3)"
    );
    assert!(
        names.contains(&"TeamDelete"),
        "missing TeamDelete tool (Fix 3)"
    );
    assert!(names.contains(&"lsp"), "missing LSP tool (Fix 3)");
    assert!(
        names.contains(&"RemoteTrigger"),
        "missing RemoteTrigger tool (Fix 3)"
    );

    // TASK-CLI-410: Code Cartographer
    assert!(
        names.contains(&"CartographerScan"),
        "missing CartographerScan tool (TASK-CLI-410)"
    );
    for name in archon_tools::gametheory::GAMETHEORY_TOOL_NAMES {
        assert!(names.contains(name), "missing {name} tool (Group 9)");
    }
    for name in archon_tools::docs::DOC_TOOL_NAMES {
        assert!(names.contains(name), "missing {name} tool (TSPEC §12)");
    }
    for name in archon_tools::learning::LEARNING_TOOL_NAMES {
        assert!(names.contains(name), "missing {name} tool (TSPEC §12)");
    }
    assert!(names.contains(&"ToolSearch"), "missing ToolSearch tool");
}

#[test]
fn test_all_8_gametheory_tools_registered() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let names = registry.tool_names();
    let registered: Vec<_> = archon_tools::gametheory::GAMETHEORY_TOOL_NAMES
        .iter()
        .filter(|name| names.contains(name))
        .copied()
        .collect();

    assert_eq!(
        registered,
        archon_tools::gametheory::GAMETHEORY_TOOL_NAMES,
        "all Group 9 gametheory tools must be discoverable from the runtime registry"
    );
}

#[test]
fn test_evidence_engine_tools_registered() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let names = registry.tool_names();

    for name in archon_tools::docs::DOC_TOOL_NAMES {
        assert!(
            names.contains(name),
            "Doc tool {name} must be discoverable from the runtime registry"
        );
    }
    for name in archon_tools::learning::LEARNING_TOOL_NAMES {
        assert!(
            names.contains(name),
            "Learning tool {name} must be discoverable from the runtime registry"
        );
    }
}

#[test]
fn tool_definitions_valid_json() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let defs = registry.tool_definitions();

    for def in &defs {
        assert!(def["name"].is_string(), "tool def missing name");
        assert!(
            def["description"].is_string(),
            "tool def missing description"
        );
        assert!(def["input_schema"].is_object(), "tool def missing schema");
    }
}

#[test]
fn docs_do_not_reference_unknown_tools() {
    let documented = documented_tool_names();
    let registry = create_default_registry(std::env::temp_dir(), None);
    let mut registered: std::collections::HashSet<String> = registry
        .tool_names()
        .into_iter()
        .map(str::to_string)
        .collect();

    // Session-wired tools need runtime dependencies (the memory graph, the
    // resolved agent catalog); they are still real tools and are registered by
    // src/session.rs.
    registered.insert("memory_store".to_string());
    registered.insert("memory_recall".to_string());
    registered.insert("AgentCatalog".to_string());

    // LEANN tools are conditional because they require an available index
    // at startup. The docs explicitly mark them as conditional.
    let conditional = ["LeannSearch", "LeannFindSimilar"];

    let unknown: Vec<_> = documented
        .into_iter()
        .filter(|name| !registered.contains(name) && !conditional.contains(&name.as_str()))
        .collect();

    assert!(
        unknown.is_empty(),
        "docs/reference/tools.md references unknown tools: {unknown:?}"
    );
}

fn documented_tool_names() -> Vec<String> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/tools.md");
    let markdown = std::fs::read_to_string(path).expect("tool docs exist");

    markdown
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with("| `") {
                return None;
            }
            let cell = trimmed.split('|').nth(1)?;
            let start = cell.find('`')? + 1;
            let rest = &cell[start..];
            let end = rest.find('`')?;
            Some(rest[..end].to_string())
        })
        .filter(|name| name != "Tool")
        .collect()
}

#[tokio::test]
async fn dispatch_unknown_tool_returns_error() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let ctx = ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    };

    let result = registry
        .dispatch("NonexistentTool", serde_json::json!({}), &ctx)
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("Unknown tool"));
}

#[tokio::test]
async fn dispatch_success_emits_started_and_completed_activity() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ActivityTestTool::success("ActivityEcho")));
    let sink = Arc::new(InMemoryActivitySink::new());
    let ctx = activity_ctx(sink.clone());

    let result = registry
        .dispatch("ActivityEcho", serde_json::json!({}), &ctx)
        .await;

    assert!(!result.is_error);
    let events = sink.events();
    let kinds: Vec<_> = events.iter().map(|event| event.kind).collect();
    assert_eq!(
        kinds,
        vec![
            AgentActivityKind::ToolStarted,
            AgentActivityKind::ToolCompleted
        ]
    );
    assert_eq!(events[0].message, "ActivityEcho");
    assert!(events[1].message.starts_with("ActivityEcho elapsed="));
    assert!(
        events
            .iter()
            .all(|event| event.session_id == "activity-test")
    );
}

#[tokio::test]
async fn dispatch_tool_error_emits_started_and_failed_activity() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ActivityTestTool::failure("ActivityFail")));
    let sink = Arc::new(InMemoryActivitySink::new());
    let ctx = activity_ctx(sink.clone());

    let result = registry
        .dispatch("ActivityFail", serde_json::json!({}), &ctx)
        .await;

    assert!(result.is_error);
    let kinds: Vec<_> = sink.events().iter().map(|event| event.kind).collect();
    assert_eq!(
        kinds,
        vec![
            AgentActivityKind::ToolStarted,
            AgentActivityKind::ToolFailed
        ]
    );
}

#[tokio::test]
async fn dispatch_unknown_tool_emits_failed_activity() {
    let registry = ToolRegistry::new();
    let sink = Arc::new(InMemoryActivitySink::new());
    let ctx = activity_ctx(sink.clone());

    let result = registry
        .dispatch("MissingActivityTool", serde_json::json!({}), &ctx)
        .await;

    assert!(result.is_error);
    let events = sink.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, AgentActivityKind::ToolFailed);
    assert_eq!(events[0].message, "MissingActivityTool");
}

#[test]
fn clone_filtered_with_subset() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let filtered = registry.clone_filtered(&["Read", "Grep"]);
    let names = filtered.tool_names();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"Read"));
    assert!(names.contains(&"Grep"));
}

#[test]
fn clone_filtered_empty_list_returns_empty() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let filtered = registry.clone_filtered(&[]);
    assert!(filtered.tool_names().is_empty());
}

#[test]
fn clone_filtered_nonexistent_tool_ignored() {
    let registry = create_default_registry(std::env::temp_dir(), None);
    let filtered = registry.clone_filtered(&["Read", "FakeTool"]);
    let names = filtered.tool_names();
    assert_eq!(names.len(), 1);
    assert!(names.contains(&"Read"));
}

fn activity_ctx(sink: Arc<InMemoryActivitySink>) -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "activity-test".to_string(),
        activity_sink: Some(sink),
        ..Default::default()
    }
}

struct ActivityTestTool {
    name: &'static str,
    succeeds: bool,
}

impl ActivityTestTool {
    fn success(name: &'static str) -> Self {
        Self {
            name,
            succeeds: true,
        }
    }

    fn failure(name: &'static str) -> Self {
        Self {
            name,
            succeeds: false,
        }
    }
}

#[async_trait::async_trait]
impl Tool for ActivityTestTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "activity test tool"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        if self.succeeds {
            ToolResult::success("ok")
        } else {
            ToolResult::error("failed")
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> archon_tools::tool::PermissionLevel {
        archon_tools::tool::PermissionLevel::Safe
    }
}
