//! Agent-callable game-theory evidence tools.
//!
//! The tools live in `archon-tools`, but the real pipeline lives above this
//! crate in the dependency graph. To avoid a cycle, callers install a
//! [`GameTheoryExecutor`] at process startup.

use std::sync::{Arc, OnceLock, RwLock};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

pub const GAMETHEORY_TOOL_NAMES: &[&str] = &[
    "GameTheoryRun",
    "GameTheoryStatus",
    "GameTheoryListAgents",
    "GameTheorySpecimens",
    "GameTheoryInspect",
    "GameTheoryReplay",
    "GameTheoryClassify",
    "GameTheoryCallSpecialist",
];

#[derive(Debug, Clone)]
pub struct GameTheoryRunRequest {
    pub situation: String,
    pub budget_usd: Option<f64>,
    pub max_concurrent: Option<usize>,
    pub style: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GameTheoryStatusRequest {
    pub run_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GameTheoryListAgentsRequest {
    pub tier: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct GameTheorySpecimensRequest {
    pub filter: Option<String>,
    pub ingest: bool,
}

#[derive(Debug, Clone)]
pub struct GameTheoryInspectRequest {
    pub artifact_id: String,
}

#[derive(Debug, Clone)]
pub struct GameTheoryReplayRequest {
    pub run_id: String,
    pub reclassify: bool,
    pub rerun_specialist: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GameTheoryClassifyRequest {
    pub situation: String,
}

#[derive(Debug, Clone)]
pub struct GameTheoryCallSpecialistRequest {
    pub run_id: String,
    pub agent_key: String,
}

#[async_trait]
pub trait GameTheoryExecutor: Send + Sync {
    async fn run(&self, request: GameTheoryRunRequest) -> anyhow::Result<String>;
    async fn status(&self, request: GameTheoryStatusRequest) -> anyhow::Result<String>;
    async fn list_agents(&self, request: GameTheoryListAgentsRequest) -> anyhow::Result<String>;
    async fn specimens(&self, request: GameTheorySpecimensRequest) -> anyhow::Result<String>;
    async fn inspect(&self, request: GameTheoryInspectRequest) -> anyhow::Result<String>;
    async fn replay(&self, request: GameTheoryReplayRequest) -> anyhow::Result<String>;
    async fn classify(&self, request: GameTheoryClassifyRequest) -> anyhow::Result<String>;
    async fn call_specialist(
        &self,
        request: GameTheoryCallSpecialistRequest,
    ) -> anyhow::Result<String>;
}

static GAMETHEORY_EXECUTOR: OnceLock<RwLock<Option<Arc<dyn GameTheoryExecutor>>>> = OnceLock::new();

pub fn install_gametheory_executor(exec: Arc<dyn GameTheoryExecutor>) {
    let slot = GAMETHEORY_EXECUTOR.get_or_init(|| RwLock::new(None));
    if let Ok(mut guard) = slot.write() {
        *guard = Some(exec);
    }
}

pub fn get_gametheory_executor() -> Option<Arc<dyn GameTheoryExecutor>> {
    GAMETHEORY_EXECUTOR
        .get_or_init(|| RwLock::new(None))
        .read()
        .ok()
        .and_then(|guard| guard.clone())
}

macro_rules! define_gametheory_tool {
    ($struct_name:ident, $tool_name:literal, $desc:literal, $schema_fn:ident, $exec_fn:ident, $perm:expr) => {
        pub struct $struct_name;

        #[async_trait]
        impl Tool for $struct_name {
            fn name(&self) -> &str {
                $tool_name
            }

            fn description(&self) -> &str {
                $desc
            }

            fn input_schema(&self) -> Value {
                $schema_fn()
            }

            async fn execute(&self, input: Value, _ctx: &ToolContext) -> ToolResult {
                $exec_fn(input).await
            }

            fn permission_level(&self, _input: &Value) -> PermissionLevel {
                $perm
            }
        }
    };
}

define_gametheory_tool!(
    GameTheoryRun,
    "GameTheoryRun",
    "Run the game-theory evidence pipeline and persist the resulting Cozo artifacts.",
    run_schema,
    execute_run,
    PermissionLevel::Risky
);
define_gametheory_tool!(
    GameTheoryStatus,
    "GameTheoryStatus",
    "Read persisted game-theory run status from Cozo.",
    status_schema,
    execute_status,
    PermissionLevel::Safe
);
define_gametheory_tool!(
    GameTheoryListAgents,
    "GameTheoryListAgents",
    "List registered game-theory specialists, optionally filtered by tier.",
    list_agents_schema,
    execute_list_agents,
    PermissionLevel::Safe
);
define_gametheory_tool!(
    GameTheorySpecimens,
    "GameTheorySpecimens",
    "Load or query the game-theory specimen library.",
    specimens_schema,
    execute_specimens,
    PermissionLevel::Safe
);
define_gametheory_tool!(
    GameTheoryInspect,
    "GameTheoryInspect",
    "Inspect a persisted game-theory artifact by artifact id.",
    inspect_schema,
    execute_inspect,
    PermissionLevel::Safe
);
define_gametheory_tool!(
    GameTheoryReplay,
    "GameTheoryReplay",
    "Replay routing, reclassify a stored run, or rerun one specialist.",
    replay_schema,
    execute_replay,
    PermissionLevel::Risky
);
define_gametheory_tool!(
    GameTheoryClassify,
    "GameTheoryClassify",
    "Classify a game-theory situation and persist its Tier 1 fingerprint.",
    classify_schema,
    execute_classify,
    PermissionLevel::Risky
);
define_gametheory_tool!(
    GameTheoryCallSpecialist,
    "GameTheoryCallSpecialist",
    "Rerun one game-theory specialist against a stored Tier 1 fingerprint.",
    call_specialist_schema,
    execute_call_specialist,
    PermissionLevel::Risky
);

async fn execute_run(input: Value) -> ToolResult {
    let req = match parse_run(input) {
        Ok(req) => req,
        Err(e) => return ToolResult::error(e),
    };
    call_executor(|exec| async move { exec.run(req).await }).await
}

async fn execute_status(input: Value) -> ToolResult {
    let req = GameTheoryStatusRequest {
        run_id: opt_string(&input, "run_id"),
    };
    call_executor(|exec| async move { exec.status(req).await }).await
}

async fn execute_list_agents(input: Value) -> ToolResult {
    let tier = match opt_u8(&input, "tier") {
        Ok(tier) => tier,
        Err(e) => return ToolResult::error(e),
    };
    let req = GameTheoryListAgentsRequest { tier };
    call_executor(|exec| async move { exec.list_agents(req).await }).await
}

async fn execute_specimens(input: Value) -> ToolResult {
    let req = GameTheorySpecimensRequest {
        filter: opt_string(&input, "filter"),
        ingest: input
            .get("ingest")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    call_executor(|exec| async move { exec.specimens(req).await }).await
}

async fn execute_inspect(input: Value) -> ToolResult {
    let req = match required_string(&input, "artifact_id") {
        Ok(artifact_id) => GameTheoryInspectRequest { artifact_id },
        Err(e) => return ToolResult::error(e),
    };
    call_executor(|exec| async move { exec.inspect(req).await }).await
}

async fn execute_replay(input: Value) -> ToolResult {
    let req = match parse_replay(input) {
        Ok(req) => req,
        Err(e) => return ToolResult::error(e),
    };
    call_executor(|exec| async move { exec.replay(req).await }).await
}

async fn execute_classify(input: Value) -> ToolResult {
    let req = match required_string(&input, "situation") {
        Ok(situation) => GameTheoryClassifyRequest { situation },
        Err(e) => return ToolResult::error(e),
    };
    call_executor(|exec| async move { exec.classify(req).await }).await
}

async fn execute_call_specialist(input: Value) -> ToolResult {
    let req = match (
        required_string(&input, "run_id"),
        required_string(&input, "agent_key"),
    ) {
        (Ok(run_id), Ok(agent_key)) => GameTheoryCallSpecialistRequest { run_id, agent_key },
        (Err(e), _) | (_, Err(e)) => return ToolResult::error(e),
    };
    call_executor(|exec| async move { exec.call_specialist(req).await }).await
}

async fn call_executor<F, Fut>(call: F) -> ToolResult
where
    F: FnOnce(Arc<dyn GameTheoryExecutor>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<String>>,
{
    let Some(exec) = get_gametheory_executor() else {
        return ToolResult::error("gametheory executor not installed");
    };
    match call(exec).await {
        Ok(text) => ToolResult::success(text),
        Err(e) => ToolResult::error(format!("gametheory tool failed: {e}")),
    }
}

fn parse_run(input: Value) -> Result<GameTheoryRunRequest, String> {
    let situation = required_string(&input, "situation")?;
    let budget_usd = opt_f64(&input, "budget_usd");
    let max_concurrent = opt_usize(&input, "max_concurrent");
    if budget_usd.is_some_and(|v| !v.is_finite() || v <= 0.0) {
        return Err("budget_usd must be a positive finite number".into());
    }
    if max_concurrent.is_some_and(|v| v == 0) {
        return Err("max_concurrent must be greater than zero".into());
    }
    let style = opt_string(&input, "style");
    if let Some(ref style) = style
        && !matches!(style.as_str(), "executive" | "academic" | "technical")
    {
        return Err("style must be executive, academic, or technical".into());
    }
    Ok(GameTheoryRunRequest {
        situation,
        budget_usd,
        max_concurrent,
        style,
    })
}

fn parse_replay(input: Value) -> Result<GameTheoryReplayRequest, String> {
    let reclassify = input
        .get("reclassify")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let rerun_specialist = opt_string(&input, "rerun_specialist");
    if reclassify && rerun_specialist.is_some() {
        return Err("reclassify and rerun_specialist cannot be combined".into());
    }
    Ok(GameTheoryReplayRequest {
        run_id: required_string(&input, "run_id")?,
        reclassify,
        rerun_specialist,
    })
}

fn required_string(input: &Value, key: &str) -> Result<String, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{key} is required and must be a non-empty string"))
}

fn opt_string(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn opt_f64(input: &Value, key: &str) -> Option<f64> {
    input.get(key).and_then(Value::as_f64)
}

fn opt_usize(input: &Value, key: &str) -> Option<usize> {
    input.get(key).and_then(Value::as_u64).map(|v| v as usize)
}

fn opt_u8(input: &Value, key: &str) -> Result<Option<u8>, String> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    let Some(raw) = value.as_u64() else {
        return Err(format!("{key} must be an integer"));
    };
    u8::try_from(raw)
        .map(Some)
        .map_err(|_| format!("{key} must be between 0 and 255"))
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

fn run_schema() -> Value {
    object_schema(
        json!({
            "situation": { "type": "string" },
            "budget_usd": { "type": "number" },
            "max_concurrent": { "type": "integer" },
            "style": { "type": "string", "enum": ["executive", "academic", "technical"] }
        }),
        &["situation"],
    )
}

fn status_schema() -> Value {
    object_schema(json!({ "run_id": { "type": "string" } }), &[])
}

fn list_agents_schema() -> Value {
    object_schema(json!({ "tier": { "type": "integer" } }), &[])
}

fn specimens_schema() -> Value {
    object_schema(
        json!({ "filter": { "type": "string" }, "ingest": { "type": "boolean" } }),
        &[],
    )
}

fn inspect_schema() -> Value {
    object_schema(
        json!({ "artifact_id": { "type": "string" } }),
        &["artifact_id"],
    )
}

fn replay_schema() -> Value {
    object_schema(
        json!({
            "run_id": { "type": "string" },
            "reclassify": { "type": "boolean" },
            "rerun_specialist": { "type": "string" }
        }),
        &["run_id"],
    )
}

fn classify_schema() -> Value {
    object_schema(json!({ "situation": { "type": "string" } }), &["situation"])
}

fn call_specialist_schema() -> Value {
    object_schema(
        json!({
            "run_id": { "type": "string" },
            "agent_key": { "type": "string" }
        }),
        &["run_id", "agent_key"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_names_are_the_required_group9_surface() {
        assert_eq!(GAMETHEORY_TOOL_NAMES.len(), 8);
        assert!(GAMETHEORY_TOOL_NAMES.contains(&"GameTheoryRun"));
        assert!(GAMETHEORY_TOOL_NAMES.contains(&"GameTheoryCallSpecialist"));
    }

    #[test]
    fn test_run_parser_rejects_empty_situation() {
        let err = parse_run(json!({ "situation": "  " })).unwrap_err();
        assert!(err.contains("situation is required"));
    }

    #[test]
    fn test_run_parser_rejects_unknown_style() {
        let err = parse_run(json!({ "situation": "duopoly", "style": "vibes" })).unwrap_err();
        assert!(err.contains("style must be"));
    }

    #[test]
    fn test_tier_parser_rejects_out_of_range_value() {
        let err = opt_u8(&json!({ "tier": 999 }), "tier").unwrap_err();
        assert!(err.contains("between 0 and 255"));
    }

    #[test]
    fn test_replay_parser_rejects_conflicting_modes() {
        let err = parse_replay(json!({
            "run_id": "gt-1",
            "reclassify": true,
            "rerun_specialist": "nash-equilibrium-finder"
        }))
        .unwrap_err();
        assert!(err.contains("cannot be combined"));
    }
}
