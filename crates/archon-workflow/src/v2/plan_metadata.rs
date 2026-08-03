//! Projecting a generated V2 plan back onto the `archon.workflow.v1` vocabulary.
//!
//! A generated run is a list of typed host calls, but the thing an operator
//! approves is a [`WorkflowSpec`]: the approval metadata, the compiled bundle
//! and the persisted run state all speak stages. The mapping between the two —
//! which [`WorkflowV2HostMethod`] is which [`StageKind`], which provider tier it
//! implies, and what a call's authored options become once the typed
//! [`StageSpec`] fields are accounted for — is a fact about those two
//! vocabularies and nothing else.
//!
//! It sat in the CLI planner because that is where the projection was first
//! needed. Nothing in it reads configuration, dispatches, or renders: it is a
//! total function from a host call to a stage, and both types are this crate's.

use std::collections::BTreeMap;

use crate::spec::{ProviderTier, RetryPolicy, StageKind, StageSpec};

use super::host_api::{WorkflowV2HostCall, WorkflowV2HostMethod};

/// The approval-metadata stage for one generated host call.
///
/// `task` is the run's own task text, used only to synthesise a description for
/// a call that declared none.
pub fn approval_metadata_stage(task: &str, call: &WorkflowV2HostCall) -> StageSpec {
    let mut extra = call.options.extra.clone();
    // `condition` is no longer a typed StageSpec field — no evaluator was ever
    // wired up, so it never branched. Leave whatever the plan authored in
    // `extra` so the approval metadata still shows it verbatim.
    strip_reserved_stage_extra(&mut extra);
    StageSpec {
        id: call.id.clone(),
        kind: stage_kind_for_call(call.method),
        task: Some(call.options.task.clone().unwrap_or_else(|| {
            format!(
                "Approval metadata for V2 host call '{}' in generated workflow: {}",
                call.id, task
            )
        })),
        agent: None,
        foreach: None,
        reducer: None,
        tool: declared_tool_name(call),
        depends_on: Vec::new(),
        provider_tier: Some(provider_tier_for_call(call.method)),
        retry: RetryPolicy::default(),
        input: serde_json::json!({
            "runtime": "script_first_v2",
            "metadata_only": true,
            "host_call": call.method.as_str(),
            "write_mode": call.write_mode,
            "source": call.options.source.clone(),
            "role": call.options.role.clone(),
        }),
        model: None,
        provider: None,
        expected_target_files: call.options.target_files.clone(),
        verify_command: None,
        max_parallelism: call.options.max_parallelism.map(|value| value as u32),
        item_kind: call.write_mode.map(|_| StageKind::Implementation),
        filter: None,
        extra,
    }
}

fn declared_tool_name(call: &WorkflowV2HostCall) -> Option<String> {
    if call.method != WorkflowV2HostMethod::Tool {
        return None;
    }
    call.options
        .extra
        .get("tool")
        .or_else(|| call.options.extra.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn strip_reserved_stage_extra(extra: &mut BTreeMap<String, serde_json::Value>) {
    for key in [
        "id",
        "kind",
        "task",
        "agent",
        "foreach",
        "reducer",
        "tool",
        "depends_on",
        "provider_tier",
        "retry",
        "input",
        "model",
        "provider",
        "expected_target_files",
        "verify_command",
        "max_parallelism",
        "item_kind",
        "filter",
    ] {
        extra.remove(key);
    }
}

pub fn stage_kind_for_call(method: WorkflowV2HostMethod) -> StageKind {
    match method {
        WorkflowV2HostMethod::Agent => StageKind::Agent,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => StageKind::Fanout,
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => StageKind::Reduce,
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::Checkpoint
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact => StageKind::Tool,
        WorkflowV2HostMethod::Implementation => StageKind::Implementation,
        WorkflowV2HostMethod::QualityGate => StageKind::QualityGate,
        WorkflowV2HostMethod::HumanGate => StageKind::HumanGate,
    }
}

pub fn provider_tier_for_call(method: WorkflowV2HostMethod) -> ProviderTier {
    match method {
        WorkflowV2HostMethod::Agent => ProviderTier::Researcher,
        WorkflowV2HostMethod::Fanout
        | WorkflowV2HostMethod::Parallel
        | WorkflowV2HostMethod::Implementation => ProviderTier::Coder,
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => ProviderTier::Reducer,
        WorkflowV2HostMethod::QualityGate | WorkflowV2HostMethod::HumanGate => ProviderTier::Critic,
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::Checkpoint
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact => ProviderTier::Local,
    }
}

/// A workflow name derived from the run's task text.
///
/// A generated plan authors no name, and the run directory, the bundle and the
/// approval record all key on one, so it has to come from the only text there
/// is. Bounded to eight slug parts so a paragraph-length task does not become a
/// paragraph-length identifier.
pub fn workflow_name_from_task(task: &str) -> String {
    let slug = task
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "workflow-v2".to_string()
    } else {
        slug
    }
}

/// The JavaScript inside a provider's reply.
///
/// Providers wrap authored `workflow.js` in a fenced code block often enough
/// that unwrapping it is part of reading the reply, not part of being lenient:
/// the fence markers are not valid JavaScript and the QuickJS compile would
/// reject them. Unfenced content is returned trimmed and otherwise untouched.
pub fn extract_javascript(content: &str) -> String {
    let trimmed = content.trim();
    if let Some(start) = trimmed.find("```") {
        let rest = &trimmed[start + 3..];
        let rest = rest
            .strip_prefix("javascript")
            .or_else(|| rest.strip_prefix("js"))
            .unwrap_or(rest);
        let rest = rest.trim_start();
        if let Some(end) = rest.find("```") {
            return rest[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}
