use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use archon_core::config::GeneratedWorkflowConfig;
use archon_pipeline::runner::LlmClient;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_workflow::{
    GeneratedWorkflowKind, GeneratedWorkflowLearningContext, LifecycleAction, LifecycleController,
    ProviderTier, RunStatus, WorkflowBundle, WorkflowBundleOrigin, WorkflowError,
    WorkflowEventKind, WorkflowEventLog, WorkflowGeneratedScaffold, WorkflowLearningEvent,
    WorkflowLearningEvidenceRef, WorkflowRun, WorkflowStore, WorkflowV2AgentAdapter,
    WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2BranchOutcome, WorkflowV2CallExecution,
    WorkflowV2CallRecord, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
    WorkflowV2FanoutReport, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2RejectedOutput,
    WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Scheduler,
    WorkflowV2SchedulerConfig, WorkflowV2Status, workflow_scaffold_hash,
};

#[path = "workflow_live_provider_env.rs"]
mod workflow_live_provider_env;
#[path = "workflow_live_v2_aggregate.rs"]
mod workflow_live_v2_aggregate;
#[path = "workflow_live_v2_artifact_paths.rs"]
mod workflow_live_v2_artifact_paths;
#[path = "workflow_live_v2_data.rs"]
mod workflow_live_v2_data;
#[cfg(test)]
#[path = "workflow_live_v2_data_tests.rs"]
mod workflow_live_v2_data_tests;
#[path = "workflow_live_v2_source_graph.rs"]
mod workflow_live_v2_source_graph;
#[path = "workflow_live_v2_stable_json.rs"]
mod workflow_live_v2_stable_json;
#[path = "workflow_live_v2_target_expansion.rs"]
mod workflow_live_v2_target_expansion;
#[path = "workflow_live_v2_verification.rs"]
mod workflow_live_v2_verification;

use workflow_live_v2_data::{
    execution_with_resolved_source, fanout_items_for_call, result_from_fanout_report,
    v2_agent_request,
};
#[path = "workflow_live_v2_client.rs"]
mod workflow_live_v2_client;

use workflow_live_v2_client::LiveV2AgentClient;
#[path = "workflow_live_v2_contracts.rs"]
mod workflow_live_v2_contracts;

#[path = "workflow_live_v2_write.rs"]
mod workflow_live_v2_write;

use workflow_live_v2_write::run_write_capable_v2_fanout;
#[path = "workflow_live_v2_state.rs"]
mod workflow_live_v2_state;

use workflow_live_v2_state::{poll_v2_run_control, sync_v2_summary_to_run};
#[path = "workflow_live_v2_script.rs"]
mod workflow_live_v2_script;

use workflow_live_v2_script::WorkflowV2ScriptRunner;
pub(super) use workflow_live_v2_script::dry_run_workflow_plan;

use super::LiveApprovalMode;
use super::workflow_live_approval::{LiveApprovalOutcome, gate_live_approval};
use super::workflow_live_planner::WorkflowScriptPlan;
use super::workflow_live_task_universe::WorkflowV2TaskUniverse;
use super::workflow_live_v2_host::execute_local_host_call;

const GENERATED_V2_METADATA_PATH: &str = "v2/generated-metadata.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct GeneratedV2Metadata {
    schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generated_kind: Option<GeneratedWorkflowKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scaffold_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generated_scaffold: Option<WorkflowGeneratedScaffold>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_universe: Option<WorkflowV2TaskUniverse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    script_args: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    governed_learning_context: Vec<GeneratedWorkflowLearningContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generated_config: Option<GeneratedWorkflowConfig>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct WorkflowV2ScriptRuntime {
    pub(super) target_repository_root: Option<String>,
    pub(super) generated_config: GeneratedWorkflowConfig,
}

include!("workflow_live_v2_run.rs");

include!("workflow_live_v2_learning.rs");

include!("workflow_live_v2_host_dispatch.rs");

include!("workflow_live_v2_read_only.rs");

include!("workflow_live_v2_branch_cache.rs");

#[cfg(test)]
mod branch_cache_tests {
    use super::*;
    use archon_workflow::{
        WorkflowV2BranchOutcome, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
        WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
        WorkflowV2ResultStore, WorkflowV2Status,
    };

    #[test]
    fn reusable_branch_outcomes_preserve_siblings_for_restart_item() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let mut accepted = WorkflowV2Result::accepted("accepted sibling branch");
        accepted.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "sibling branch has concrete cached evidence",
        ));
        let base_call = WorkflowV2HostCall {
            id: "review".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        };
        let items = vec![
            WorkflowV2FanoutItem::read_only(
                "review-a",
                "critic",
                base_call.clone(),
                serde_json::json!({"item": "a"}),
            ),
            WorkflowV2FanoutItem::read_only(
                "review-b",
                "critic",
                base_call,
                serde_json::json!({"item": "b"}),
            ),
        ];
        store
            .save_branch_outcome(
                "review",
                &WorkflowV2BranchOutcome {
                    item_id: "review-a".to_string(),
                    role: "critic".to_string(),
                    status: WorkflowV2Status::Accepted,
                    result: Some(accepted),
                    error: None,
                    failure_kind: None,
                    item_input_hash: Some(items[0].input_hash()),
                    completion_evidence: Vec::new(),
                },
            )
            .expect("save branch");

        let (reused, pending) =
            split_reusable_branch_outcomes(&store, "review", items).expect("split branches");

        assert_eq!(reused.len(), 1);
        assert_eq!(reused[0].item_id, "review-a");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "review-b");
    }

    #[test]
    fn branch_outcomes_needing_review_are_not_reused() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: "review found unresolved issues".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Review,
                "review branch requires remediation before reuse",
            )],
            ..WorkflowV2Result::default()
        };
        store
            .save_branch_outcome(
                "review",
                &WorkflowV2BranchOutcome {
                    item_id: "review-a".to_string(),
                    role: "critic".to_string(),
                    status: WorkflowV2Status::NeedsReview,
                    result: Some(result),
                    error: None,
                    failure_kind: None,
                    item_input_hash: None,
                    completion_evidence: Vec::new(),
                },
            )
            .expect("save branch");
        let base_call = WorkflowV2HostCall {
            id: "review".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        };
        let items = vec![WorkflowV2FanoutItem::read_only(
            "review-a",
            "critic",
            base_call,
            serde_json::json!({"item": "a"}),
        )];

        let (reused, pending) =
            split_reusable_branch_outcomes(&store, "review", items).expect("split branches");

        assert!(reused.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "review-a");
    }
}

#[cfg(test)]
mod local_tool_tests {
    use super::*;
    use archon_workflow::{WorkflowV2HostCall, WorkflowV2HostOptions};

    #[test]
    fn declared_checkpoint_tool_delegates_to_local_host_api() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let execution = tool_execution("checkpoint-tool", "checkpoint");

        let result =
            execute_declared_local_tool(execution, &store, None).expect("checkpoint tool result");

        assert_eq!(result.status, WorkflowV2Status::Accepted);
        assert!(result.summary.contains("checkpoint-tool"));
    }

    #[test]
    fn undeclared_tool_fails_closed_before_execution() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let execution = tool_execution("mystery-tool", "shell");

        let err = execute_declared_local_tool(execution, &store, None)
            .expect_err("unknown tools must fail closed");

        assert!(err.to_string().contains("unknown local tool"));
    }

    fn tool_execution(id: &str, tool: &str) -> WorkflowV2CallExecution {
        let mut options = WorkflowV2HostOptions::default();
        options
            .extra
            .insert("tool".to_string(), serde_json::json!(tool));
        WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: id.to_string(),
                method: WorkflowV2HostMethod::Tool,
                write_mode: None,
                options,
            },
            input: serde_json::json!({
                "options": {
                    "tool": tool,
                    "inputs": {
                        "payload": id
                    }
                },
                "source_data": {
                    "payload": id
                }
            }),
            depends_on: Vec::new(),
        }
    }
}
