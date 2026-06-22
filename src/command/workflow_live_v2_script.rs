use std::sync::Arc;

use archon_tui::app::TuiEvent;
use archon_workflow::{
    WorkflowError, WorkflowSpec, WorkflowStore, WorkflowV2AgentAdapter, WorkflowV2CallExecution,
    WorkflowV2CallRecord, WorkflowV2Checkpoint, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
    WorkflowV2WriteMode,
};
use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
use tokio::sync::Mutex;

use super::execute_v2_live_call;
use super::workflow_live_v2_client::LiveV2AgentClient;
use super::workflow_live_v2_contracts::failed_v2_result;
use super::workflow_live_v2_state::{mark_v2_call_running, poll_v2_run_control};

#[derive(Debug, Clone)]
pub(super) struct WorkflowV2ScriptSummary {
    pub(super) status: WorkflowV2Status,
    pub(super) completed: usize,
    pub(super) executed: usize,
    pub(super) reused: usize,
    pub(super) calls: Vec<WorkflowV2HostCall>,
}

#[derive(Clone)]
pub(super) struct WorkflowV2ScriptRunner {
    task: String,
    spec: WorkflowSpec,
    adapter: WorkflowV2AgentAdapter,
    client: LiveV2AgentClient,
    v2_store: WorkflowV2ResultStore,
    workflow_store: WorkflowStore,
    run_id: String,
    workspace_boundary_supported: bool,
}

impl WorkflowV2ScriptRunner {
    pub(super) fn new(
        task: String,
        spec: WorkflowSpec,
        adapter: WorkflowV2AgentAdapter,
        client: LiveV2AgentClient,
        v2_store: WorkflowV2ResultStore,
        workflow_store: WorkflowStore,
        run_id: String,
        workspace_boundary_supported: bool,
    ) -> Self {
        Self {
            task,
            spec,
            adapter,
            client,
            v2_store,
            workflow_store,
            run_id,
            workspace_boundary_supported,
        }
    }

    pub(super) async fn run(
        self,
        harness_source: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let harness_source = harness_source.to_string();
        tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| {
                    WorkflowError::SpecInvalid(format!(
                        "workflow.js local async runtime failed: {err}"
                    ))
                })?;
            runtime.block_on(self.run_on_current_thread(&harness_source))
        })
        .await
        .map_err(|err| WorkflowError::SpecInvalid(format!("workflow.js task failed: {err}")))?
    }

    async fn run_on_current_thread(
        self,
        harness_source: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let host = Arc::new(WorkflowScriptHost {
            runner: self,
            accumulator: Arc::new(Mutex::new(WorkflowScriptAccumulator::default())),
        });
        let runtime = AsyncRuntime::new()
            .map_err(|err| WorkflowError::SpecInvalid(format!("quickjs runtime failed: {err}")))?;
        let context = AsyncContext::full(&runtime)
            .await
            .map_err(|err| WorkflowError::SpecInvalid(format!("quickjs context failed: {err}")))?;
        let source = script_source(harness_source);
        let host_for_js = host.clone();
        let js_result = context
            .async_with(async move |ctx| {
                ctx.globals().set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let host = host_for_js.clone();
                        async move {
                            host.execute(method, payload).await.map_err(|err| {
                                rquickjs::Error::new_from_js_message(
                                    "archon workflow host",
                                    "string",
                                    err.to_string(),
                                )
                            })
                        }
                    })),
                )?;
                let promise: Promise = match ctx.eval(source.as_str()).catch(&ctx) {
                    Ok(promise) => promise,
                    Err(err) => {
                        return Err(rquickjs::Error::new_from_js_message(
                            "workflow.js",
                            "promise",
                            err.to_string(),
                        ));
                    }
                };
                match promise.into_future::<String>().await.catch(&ctx) {
                    Ok(result) => Ok(result),
                    Err(err) => Err(rquickjs::Error::new_from_js_message(
                        "workflow.js",
                        "string",
                        err.to_string(),
                    )),
                }
            })
            .await;
        match js_result {
            Ok(_) => Ok(host.summary().await),
            Err(err) => Err(workflow_js_error(err.to_string())),
        }
    }
}

fn workflow_js_error(error: String) -> WorkflowError {
    if let Some(message) = extract_run_control_message(&error, "workflow paused by run control:") {
        return WorkflowError::ControlPaused(message);
    }
    if let Some(message) = extract_run_control_message(&error, "workflow cancelled by run control:")
    {
        return WorkflowError::ControlCancelled(message);
    }
    WorkflowError::SpecInvalid(format!("workflow.js execution failed: {error}"))
}

fn extract_run_control_message(error: &str, marker: &str) -> Option<String> {
    let start = error.find(marker)?;
    Some(error[start..].trim().to_string())
}

struct WorkflowScriptAccumulator {
    status: WorkflowV2Status,
    completed: usize,
    executed: usize,
    reused: usize,
    calls: Vec<WorkflowV2HostCall>,
}

impl Default for WorkflowScriptAccumulator {
    fn default() -> Self {
        Self {
            status: WorkflowV2Status::Accepted,
            completed: 0,
            executed: 0,
            reused: 0,
            calls: Vec::new(),
        }
    }
}

struct WorkflowScriptHost {
    runner: WorkflowV2ScriptRunner,
    accumulator: Arc<Mutex<WorkflowScriptAccumulator>>,
}

impl WorkflowScriptHost {
    async fn execute(
        &self,
        method: String,
        payload: String,
    ) -> archon_workflow::WorkflowResult<String> {
        let request: ScriptHostRequest = serde_json::from_str(&payload)?;
        let execution = self.execution_from_request(&method, request)?;
        let input_hash = stable_input_hash(&execution.input);
        if let Some(record) = self.runner.v2_store.load_call_record(&execution.call.id)? {
            if record.is_reusable_for(&input_hash) {
                self.mark_reused(&record).await?;
                return result_view_json(&record.result);
            }
        }

        poll_v2_run_control(
            &self.runner.workflow_store,
            &self.runner.run_id,
            &execution.call.id,
        )?;
        mark_v2_call_running(
            &self.runner.workflow_store,
            &self.runner.run_id,
            &execution.call.id,
        )?;
        let _ = self.runner.client.tui_tx.send(TuiEvent::TextDelta(format!(
            "Workflow V2 script call running: {} via w.{}\n",
            execution.call.id,
            execution.call.method.as_str()
        )));
        let attempt = self
            .runner
            .v2_store
            .load_call_record(&execution.call.id)?
            .map_or(1, |record| record.attempt.saturating_add(1));
        let call_id = execution.call.id.clone();
        let result = match execute_v2_live_call(
            &self.runner.task,
            &self.runner.spec,
            execution.clone(),
            self.runner.adapter.clone(),
            &self.runner.client,
            &self.runner.v2_store,
            &self.runner.workflow_store,
            &self.runner.run_id,
            self.runner.workspace_boundary_supported,
        )
        .await
        {
            Ok(result) => result,
            Err(err) => {
                if matches!(
                    &err,
                    WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
                ) {
                    return Err(err);
                }
                failed_v2_result(&call_id, err)
            }
        };
        let result = match result.validate() {
            Ok(()) => result,
            Err(err) => failed_v2_result(&call_id, WorkflowError::SpecInvalid(err.to_string())),
        };
        let status = result.status;
        let record = WorkflowV2CallRecord::new(
            self.runner.v2_store.run_id(),
            execution.call.clone(),
            attempt,
            input_hash,
            result,
            execution.depends_on,
        );
        self.runner.v2_store.save_call_record(&record)?;
        self.update_checkpoint(&record)?;
        self.mark_executed(&record, status).await;
        poll_v2_run_control(&self.runner.workflow_store, &self.runner.run_id, "")?;
        result_view_json(&record.result)
    }

    fn execution_from_request(
        &self,
        method: &str,
        request: ScriptHostRequest,
    ) -> archon_workflow::WorkflowResult<WorkflowV2CallExecution> {
        let method = WorkflowV2HostMethod::parse(method).ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "workflow.js used unsupported host method w.{method}"
            ))
        })?;
        let (options, mut write_mode) = parse_script_options(&request.options)?;
        if method == WorkflowV2HostMethod::Implementation && write_mode.is_none() {
            write_mode = Some(WorkflowV2WriteMode::Serial);
        }
        let mut input = serde_json::json!({
            "objective": self.runner.task.clone(),
            "call_id": request.id.clone(),
            "method": method.as_str(),
            "write_mode": write_mode,
            "options": request.options,
        });
        if let Some(source) = request.source {
            input["source_data"] = source;
        }
        if let Some(inputs) = input
            .get("options")
            .and_then(|options| options.get("inputs"))
            .cloned()
        {
            input["inputs"] = inputs.clone();
            input["source_data"] = inputs;
        }
        if let Some(source) = options.source.as_deref() {
            input["source"] = serde_json::Value::String(source.to_string());
        }
        Ok(WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: request.id,
                method,
                write_mode,
                options,
            },
            input,
            depends_on: Vec::new(),
        })
    }

    fn update_checkpoint(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<()> {
        let mut checkpoint = self
            .runner
            .v2_store
            .load_checkpoint()?
            .unwrap_or_else(WorkflowV2Checkpoint::default);
        if is_reusable_status(record.status) {
            checkpoint.mark_completed(&record.call.id);
        } else {
            checkpoint.remove_completed_call(&record.call.id);
        }
        self.runner.v2_store.save_checkpoint(&checkpoint)
    }

    async fn mark_reused(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<()> {
        let mut checkpoint = self
            .runner
            .v2_store
            .load_checkpoint()?
            .unwrap_or_else(WorkflowV2Checkpoint::default);
        checkpoint.mark_completed(&record.call.id);
        self.runner.v2_store.save_checkpoint(&checkpoint)?;
        let mut acc = self.accumulator.lock().await;
        acc.status = record.status;
        acc.reused += 1;
        acc.completed += 1;
        acc.calls.push(record.call.clone());
        Ok(())
    }

    async fn mark_executed(&self, record: &WorkflowV2CallRecord, status: WorkflowV2Status) {
        let mut acc = self.accumulator.lock().await;
        acc.status = status;
        acc.executed += 1;
        if is_reusable_status(status) {
            acc.completed += 1;
        }
        acc.calls.push(record.call.clone());
    }

    async fn summary(&self) -> WorkflowV2ScriptSummary {
        let acc = self.accumulator.lock().await;
        WorkflowV2ScriptSummary {
            status: acc.status,
            completed: acc.completed,
            executed: acc.executed,
            reused: acc.reused,
            calls: acc.calls.clone(),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct ScriptHostRequest {
    id: String,
    #[serde(default)]
    options: serde_json::Value,
    #[serde(default)]
    source: Option<serde_json::Value>,
}

fn parse_script_options(
    value: &serde_json::Value,
) -> archon_workflow::WorkflowResult<(WorkflowV2HostOptions, Option<WorkflowV2WriteMode>)> {
    let mut options = WorkflowV2HostOptions::default();
    let mut write_mode = None;
    let Some(object) = value.as_object() else {
        if !value.is_null() {
            options.extra.insert("value".to_string(), value.clone());
        }
        return Ok((options, write_mode));
    };
    for (key, value) in object {
        match key.as_str() {
            "role" | "tier" => options.role = string_value(value),
            "task" => options.task = string_value(value),
            "source" => options.source = string_value(value),
            "itemKind" | "item_kind" => options.item_kind = string_value(value),
            "targetFiles" | "target_files" => {
                options.target_files = string_array(value);
            }
            "targetFilesFromItem" | "target_files_from_item" => {
                options.target_files_from_item = value.as_bool().unwrap_or(false);
            }
            "maxParallelism" | "max_parallelism" => {
                options.max_parallelism =
                    value.as_u64().and_then(|value| usize::try_from(value).ok());
            }
            "write" | "writeMode" | "write_mode" => {
                if let Some(raw) = value.as_str() {
                    if raw.eq_ignore_ascii_case("none") {
                        write_mode = None;
                    } else {
                        write_mode = Some(WorkflowV2WriteMode::parse(raw).ok_or_else(|| {
                            WorkflowError::SpecInvalid(format!(
                                "invalid workflow.js write mode '{raw}'"
                            ))
                        })?);
                    }
                }
            }
            _ => {
                options.extra.insert(key.clone(), value.clone());
            }
        }
    }
    Ok((options, write_mode))
}

fn string_value(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_array(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn script_source(harness_source: &str) -> String {
    let normalized = normalize_workflow_export(harness_source);
    format!(
        r#"
{normalized}

const __archonW = Object.freeze({{
  agent: (id, options = {{}}) => __archonCall("agent", id, undefined, options),
  implementation: (id, options = {{}}) => __archonCall("implementation", id, undefined, options),
  fanout: (id, source, options = {{}}) => __archonCall("fanout", id, source, options),
  parallel: (id, source, options = {{}}) => __archonCall("parallel", id, source, options),
  tool: (id, options = {{}}) => __archonCall("tool", id, undefined, options),
  checkpoint: (id, options = {{}}) => __archonCall("checkpoint", id, undefined, options),
  saveArtifact: (id, options = {{}}) => __archonCall("saveArtifact", id, undefined, options),
  requireArtifact: (id, options = {{}}) => __archonCall("requireArtifact", id, undefined, options),
  reduce: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("reduce", id, sourceOrOptions, options),
  qualityGate: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("qualityGate", id, sourceOrOptions, options),
  humanGate: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("humanGate", id, sourceOrOptions, options),
  finalReport: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("finalReport", id, sourceOrOptions, options),
}});

function __archonMaybeSourceCall(method, id, sourceOrOptions, options) {{
  if (options === undefined) {{
    return __archonCall(method, id, undefined, sourceOrOptions || {{}});
  }}
  return __archonCall(method, id, sourceOrOptions, options || {{}});
}}

async function __archonCall(method, id, source, options) {{
  if (typeof id !== "string" || id.trim() === "") {{
    throw new Error(`w.${{method}} requires a non-empty string id`);
  }}
  const payload = {{ id, options: options || {{}} }};
  if (source !== undefined) {{
    payload.source = source;
  }}
  const json = await __archonHost(method, JSON.stringify(payload));
  return JSON.parse(json);
}}

async function __archonRun() {{
  if (typeof workflow !== "function") {{
    throw new Error("workflow.js must export or define function workflow(w)");
  }}
  const result = await workflow(__archonW);
  return JSON.stringify(result ?? null);
}}

__archonRun()
"#
    )
}

fn normalize_workflow_export(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.starts_with("export default async function workflow") {
        return trimmed.replacen(
            "export default async function workflow",
            "async function workflow",
            1,
        );
    }
    if trimmed.starts_with("export default function workflow") {
        return trimmed.replacen("export default function workflow", "function workflow", 1);
    }
    if trimmed.starts_with("export default async function(") {
        return trimmed.replacen(
            "export default async function",
            "async function workflow",
            1,
        );
    }
    if trimmed.starts_with("export default function(") {
        return trimmed.replacen("export default function", "function workflow", 1);
    }
    trimmed
        .replace("export default workflow;", "")
        .replace("export default workflow", "")
}

fn result_view_json(result: &WorkflowV2Result) -> archon_workflow::WorkflowResult<String> {
    let mut view = match &result.data {
        serde_json::Value::Object(object) => object.clone(),
        serde_json::Value::Null => serde_json::Map::new(),
        value => {
            let mut object = serde_json::Map::new();
            object.insert("data".to_string(), value.clone());
            object
        }
    };
    view.insert("status".to_string(), serde_json::to_value(result.status)?);
    view.insert(
        "summary".to_string(),
        serde_json::Value::String(result.summary.clone()),
    );
    view.insert("result".to_string(), serde_json::to_value(result)?);
    serde_json::to_string(&serde_json::Value::Object(view)).map_err(Into::into)
}

fn is_reusable_status(status: WorkflowV2Status) -> bool {
    matches!(status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
}

fn stable_input_hash(input: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};

    let bytes = serde_json::to_vec(input).unwrap_or_default();
    hex::encode(Sha256::digest(&bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use anyhow::Result;
    use archon_pipeline::runner::{LlmClient, LlmResponse};
    use archon_tui::event_channel::bounded_tui_event_channel;
    use archon_workflow::StageStatus;

    use super::*;

    #[tokio::test]
    async fn semantic_host_result_returns_to_script_without_stopping_next_call() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec = test_spec();
        let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
        let run = workflow_store.create_run(spec.clone()).expect("run");
        let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client =
            LiveV2AgentClient::new(Arc::new(PanicLlm), tui_tx, Vec::new(), run.id.clone(), None);
        let runner = WorkflowV2ScriptRunner::new(
            "needs confirmation".to_string(),
            spec,
            WorkflowV2AgentAdapter::new(),
            client,
            v2_store.clone(),
            workflow_store,
            run.id.clone(),
            true,
        );

        let summary = runner
            .run(
                r#"
	async function workflow(w) {
	  const gate = await w.humanGate("confirm-before-write", { task: "Confirm before writing" });
	  if (gate.status !== "needs_review") {
	    throw new Error("humanGate did not return typed review data");
	  }
	  await w.checkpoint("should-not-run");
	}
	"#,
            )
            .await
            .expect("script summary");

        assert_eq!(summary.status, WorkflowV2Status::Accepted);
        assert_eq!(summary.executed, 2);
        assert_eq!(summary.completed, 1);
        assert!(
            v2_store
                .load_call_record("confirm-before-write")
                .expect("confirm record")
                .is_some()
        );
        assert!(
            v2_store
                .load_call_record("should-not-run")
                .expect("checkpoint lookup")
                .is_some()
        );
    }

    #[tokio::test]
    async fn script_can_choose_to_return_after_semantic_review_result() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec = test_spec();
        let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
        let run = workflow_store.create_run(spec.clone()).expect("run");
        let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client =
            LiveV2AgentClient::new(Arc::new(PanicLlm), tui_tx, Vec::new(), run.id.clone(), None);
        let runner = WorkflowV2ScriptRunner::new(
            "script-owned branch".to_string(),
            spec,
            WorkflowV2AgentAdapter::new(),
            client,
            v2_store.clone(),
            workflow_store,
            run.id.clone(),
            true,
        );

        let summary = runner
            .run(
                r#"
	async function workflow(w) {
	  const gate = await w.humanGate("needs-user-choice", { task: "Ask user before continuing" });
	  if (gate.status === "needs_review") {
	    return gate;
	  }
	  await w.checkpoint("script-continued");
	}
	"#,
            )
            .await
            .expect("script summary");

        assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
        assert_eq!(summary.executed, 1);
        assert!(
            v2_store
                .load_call_record("needs-user-choice")
                .expect("gate lookup")
                .is_some()
        );
        assert!(
            v2_store
                .load_call_record("script-continued")
                .expect("checkpoint lookup")
                .is_none()
        );
    }

    #[tokio::test]
    async fn dynamic_loop_host_calls_are_recorded_with_runtime_ids() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec = test_spec();
        let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
        let run = workflow_store.create_run(spec.clone()).expect("run");
        let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client =
            LiveV2AgentClient::new(Arc::new(PanicLlm), tui_tx, Vec::new(), run.id.clone(), None);
        let runner = WorkflowV2ScriptRunner::new(
            "dynamic loop checkpoints".to_string(),
            spec,
            WorkflowV2AgentAdapter::new(),
            client,
            v2_store.clone(),
            workflow_store.clone(),
            run.id.clone(),
            true,
        );

        let summary = runner
            .run(
                r#"
async function workflow(w) {
  let iteration = 1;
  while (iteration <= 2) {
    await w.checkpoint("loop-checkpoint-" + iteration, { iteration });
    iteration += 1;
  }
}
"#,
            )
            .await
            .expect("script summary");

        assert_eq!(summary.status, WorkflowV2Status::Accepted);
        assert_eq!(summary.executed, 2);
        assert_eq!(summary.completed, 2);
        assert_eq!(
            summary
                .calls
                .iter()
                .map(|call| call.id.as_str())
                .collect::<Vec<_>>(),
            vec!["loop-checkpoint-1", "loop-checkpoint-2"]
        );
        assert!(
            v2_store
                .load_call_record("loop-checkpoint-1")
                .expect("first checkpoint lookup")
                .is_some()
        );
        assert!(
            v2_store
                .load_call_record("loop-checkpoint-2")
                .expect("second checkpoint lookup")
                .is_some()
        );
        let run = workflow_store.load_state(&run.id).expect("run state");
        assert_eq!(
            run.stages
                .get("loop-checkpoint-1")
                .expect("first runtime stage")
                .status,
            StageStatus::Running
        );
        assert_eq!(
            run.stages
                .get("loop-checkpoint-2")
                .expect("second runtime stage")
                .status,
            StageStatus::Running
        );
    }

    fn test_spec() -> WorkflowSpec {
        WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "script-stop-test".to_string(),
            task: "test".to_string(),
            target_repository_root: None,
            max_parallelism: 4,
            max_agents: 16,
            provider_tiers: BTreeMap::new(),
            stages: Vec::new(),
            artifact_policy: Default::default(),
            permissions: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            learning_hooks: Vec::new(),
        }
    }

    struct PanicLlm;

    #[async_trait::async_trait]
    impl LlmClient for PanicLlm {
        async fn send_message(
            &self,
            _messages: Vec<serde_json::Value>,
            _system: Vec<serde_json::Value>,
            _tools: Vec<serde_json::Value>,
            _model: &str,
        ) -> Result<LlmResponse> {
            panic!("local-host workflow script test must not call the LLM")
        }
    }
}
