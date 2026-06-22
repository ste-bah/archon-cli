use std::collections::BTreeMap;

use archon_workflow::{
    ProviderTier, RetryPolicy, StageKind, WorkflowSpec, WorkflowV2HostCall, WorkflowV2HostMethod,
};
use serde_json::Value;

use super::workflow_live_v2::source_call_ids;

const ITEMS_OUTPUT_TASK_CONTRACT: &str = concat!(
    "\n\nStructured item output contract: Return one parseable JSON or YAML document ",
    "with a top-level `items` array. Each `items[]` entry must include stable ",
    "`id` or `task_id`, concrete evidence, and `target_files` when downstream ",
    "implementation may edit repository files. If no downstream implementation ",
    "item should run, return `items: []` plus `completed_items` entries with ",
    "`task_ids` or `work_unit_ids`, `verified: true`, accepted status, and ",
    "concrete evidence. Do not return only `idempotent_noop` or status prose."
);

pub(super) fn compatibility_spec_from_v2_calls(
    task: &str,
    calls: &[WorkflowV2HostCall],
) -> WorkflowSpec {
    let mut previous = None::<String>;
    let mut stages = calls
        .iter()
        .map(|call| {
            let depends_on = compatibility_depends_on(call, previous.as_deref());
            previous = Some(call.id.clone());
            let mut extra = call.options.extra.clone();
            let condition = string_extra(&mut extra, "condition");
            apply_empty_completion_contract(call, &mut extra);
            archon_workflow::StageSpec {
                id: call.id.clone(),
                kind: stage_kind_for_v2_call(call.method),
                task: Some(call.options.task.clone().unwrap_or_else(|| {
                    format!("Execute V2 host call '{}' for objective: {}", call.id, task)
                })),
                agent: None,
                foreach: foreach_for_v2_call(call),
                reducer: None,
                tool: None,
                condition,
                depends_on,
                provider_tier: Some(provider_tier_for_v2_call(call.method)),
                retry: RetryPolicy::default(),
                input: serde_json::json!({
                    "runtime": "v2",
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
                item_kind: item_kind_for_v2_call(call),
                filter: None,
                extra,
            }
        })
        .collect::<Vec<_>>();

    mark_item_source_producers(&mut stages, calls);

    WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: workflow_name_from_task(task),
        task: task.to_string(),
        target_repository_root: target_repository_root_from_task(task),
        max_parallelism: 8,
        max_agents: 200,
        provider_tiers: BTreeMap::new(),
        stages,
        artifact_policy: Default::default(),
        permissions: BTreeMap::new(),
        quality_gates: BTreeMap::new(),
        learning_hooks: Vec::new(),
    }
}

fn string_extra(extra: &mut BTreeMap<String, Value>, key: &str) -> Option<String> {
    extra
        .remove(key)
        .and_then(|value| value.as_str().map(str::trim).map(str::to_string))
        .filter(|value| !value.is_empty())
}

fn foreach_for_v2_call(call: &WorkflowV2HostCall) -> Option<String> {
    if !matches!(
        call.method,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel
    ) {
        return None;
    }
    let source = call.options.source.as_deref()?.trim();
    let (stage, accessor) = source.split_once('.')?;
    (!stage.trim().is_empty() && !accessor.trim().is_empty())
        .then(|| format!("${{{}.{}}}", stage.trim(), accessor.trim()))
}

fn item_kind_for_v2_call(call: &WorkflowV2HostCall) -> Option<StageKind> {
    (matches!(
        call.method,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel
    ) && call.write_mode.is_some())
    .then_some(StageKind::Implementation)
}

fn apply_empty_completion_contract(call: &WorkflowV2HostCall, extra: &mut BTreeMap<String, Value>) {
    if item_kind_for_v2_call(call).is_none() {
        return;
    }
    let task_ids = task_ids_for_call(call);
    if task_ids.is_empty() {
        return;
    }
    extra
        .entry("allow_empty_when_completed".into())
        .or_insert(Value::Bool(true));
    extra
        .entry("completion_task_ids".into())
        .or_insert_with(|| Value::Array(task_ids.into_iter().map(Value::String).collect()));
}

fn mark_item_source_producers(
    stages: &mut [archon_workflow::StageSpec],
    calls: &[WorkflowV2HostCall],
) {
    for producer in calls.iter().filter_map(item_source_producer) {
        if let Some(stage) = stages.iter_mut().find(|stage| stage.id == producer) {
            declare_items_output(stage);
            append_items_output_contract(stage);
        }
    }
}

fn item_source_producer(call: &WorkflowV2HostCall) -> Option<String> {
    if !matches!(
        call.method,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel
    ) {
        return None;
    }
    call.options
        .source
        .as_deref()
        .and_then(|source| source_call_ids(source).into_iter().next())
}

fn declare_items_output(stage: &mut archon_workflow::StageSpec) {
    let mut outputs = match stage.extra.remove("outputs") {
        Some(Value::Array(values)) => values,
        Some(Value::String(value)) => vec![Value::String(value)],
        Some(_) | None => Vec::new(),
    };
    if !outputs
        .iter()
        .filter_map(Value::as_str)
        .any(|value| value.eq_ignore_ascii_case("items"))
    {
        outputs.push(Value::String("items".into()));
    }
    stage.extra.insert("outputs".into(), Value::Array(outputs));
}

fn append_items_output_contract(stage: &mut archon_workflow::StageSpec) {
    let task = stage
        .task
        .get_or_insert_with(|| format!("Execute workflow phase '{}'", stage.id));
    if !task.contains("Structured item output contract") {
        task.push_str(ITEMS_OUTPUT_TASK_CONTRACT);
    }
}

fn compatibility_depends_on(call: &WorkflowV2HostCall, previous: Option<&str>) -> Vec<String> {
    let source_deps = call
        .options
        .source
        .as_deref()
        .map(source_call_ids)
        .unwrap_or_default();
    if source_deps.is_empty() {
        previous.into_iter().map(str::to_string).collect()
    } else {
        source_deps
    }
}

fn workflow_name_from_task(task: &str) -> String {
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

pub(super) fn target_repository_root_from_task(task: &str) -> Option<String> {
    [
        "against the repository ",
        "against repository ",
        "repository root ",
        "repository ",
        "repo ",
    ]
    .into_iter()
    .find_map(|marker| path_after_marker(task, marker))
}

fn path_after_marker(task: &str, marker: &str) -> Option<String> {
    let (_, rest) = task.split_once(marker)?;
    let path = rest
        .split_whitespace()
        .next()?
        .trim_matches(|ch: char| matches!(ch, ',' | ';' | ':' | '.' | ')' | ']'));
    looks_like_filesystem_path(path).then(|| path.to_string())
}

fn looks_like_filesystem_path(path: &str) -> bool {
    path.starts_with('/') || path.starts_with("~/") || is_windows_absolute_path(path)
}

fn is_windows_absolute_path(path: &str) -> bool {
    let mut chars = path.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(drive), Some(':'), Some('\\' | '/')) if drive.is_ascii_alphabetic()
    )
}

fn stage_kind_for_v2_call(method: WorkflowV2HostMethod) -> StageKind {
    match method {
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => StageKind::Fanout,
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => StageKind::Reduce,
        WorkflowV2HostMethod::QualityGate => StageKind::QualityGate,
        WorkflowV2HostMethod::HumanGate => StageKind::HumanGate,
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact
        | WorkflowV2HostMethod::Checkpoint => StageKind::Tool,
        WorkflowV2HostMethod::Implementation => StageKind::Implementation,
        WorkflowV2HostMethod::Agent => StageKind::Agent,
    }
}

fn provider_tier_for_v2_call(method: WorkflowV2HostMethod) -> ProviderTier {
    match method {
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => ProviderTier::Reducer,
        WorkflowV2HostMethod::QualityGate | WorkflowV2HostMethod::HumanGate => ProviderTier::Critic,
        WorkflowV2HostMethod::Implementation => ProviderTier::Coder,
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact
        | WorkflowV2HostMethod::Checkpoint => ProviderTier::Local,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => ProviderTier::Coder,
        WorkflowV2HostMethod::Agent => ProviderTier::Researcher,
    }
}

fn task_ids_for_call(call: &WorkflowV2HostCall) -> Vec<String> {
    let mut ids = task_ids_from_text(&call.id);
    if let Some(task) = call.options.task.as_deref() {
        ids.extend(task_ids_from_text(task));
    }
    ids.sort();
    ids.dedup();
    ids
}

fn task_ids_from_text(text: &str) -> Vec<String> {
    let normalized = text
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>();
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let mut out = Vec::new();
    for idx in 0..tokens.len() {
        let token = tokens[idx].to_ascii_uppercase();
        if is_short_task_id(&token) {
            out.push(token);
            continue;
        }
        if token == "TASK" {
            for offset in 1..=2 {
                if let Some(next) = tokens.get(idx + offset)
                    && let Some(task) = numeric_task_id(next)
                {
                    out.push(task);
                    break;
                }
            }
        }
    }
    out
}

fn is_short_task_id(token: &str) -> bool {
    token.len() == 4
        && token.starts_with('T')
        && token.chars().skip(1).all(|ch| ch.is_ascii_digit())
}

fn numeric_task_id(text: &str) -> Option<String> {
    let digits = text
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then(|| format!("T{:0>3}", digits))
}

#[cfg(test)]
mod tests {
    use archon_workflow::{
        WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode,
    };

    use super::compatibility_spec_from_v2_calls;

    #[test]
    fn v2_write_fanout_sources_compile_to_legacy_item_contracts() {
        let calls = vec![
            WorkflowV2HostCall {
                id: "plan-t001".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: WorkflowV2HostOptions {
                    task: Some("Plan T001 work items.".into()),
                    ..WorkflowV2HostOptions::default()
                },
            },
            WorkflowV2HostCall {
                id: "implement-t001".into(),
                method: WorkflowV2HostMethod::Fanout,
                write_mode: Some(WorkflowV2WriteMode::Coordinated),
                options: WorkflowV2HostOptions {
                    source: Some("plan-t001.items".into()),
                    task: Some("Implement T001 work items.".into()),
                    target_files_from_item: true,
                    ..WorkflowV2HostOptions::default()
                },
            },
        ];
        let spec = compatibility_spec_from_v2_calls("Implement a decomposed PRD.", &calls);
        spec.validate().expect("compatibility spec validates");

        let plan = spec
            .stages
            .iter()
            .find(|stage| stage.id == "plan-t001")
            .unwrap();
        assert!(
            plan.extra["outputs"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("items"))
        );
        assert!(plan.task.as_deref().unwrap().contains("completed_items"));

        let implement = spec
            .stages
            .iter()
            .find(|stage| stage.id == "implement-t001")
            .unwrap();
        assert_eq!(implement.foreach.as_deref(), Some("${plan-t001.items}"));
        assert_eq!(
            implement.item_kind,
            Some(archon_workflow::StageKind::Implementation)
        );
        assert_eq!(
            implement.extra["allow_empty_when_completed"],
            serde_json::json!(true)
        );
        assert_eq!(
            implement.extra["completion_task_ids"],
            serde_json::json!(["T001"])
        );
    }

    #[test]
    fn v2_condition_extra_is_serialized_as_typed_stage_condition_once() {
        let calls = vec![
            WorkflowV2HostCall {
                id: "implementationItems".into(),
                method: WorkflowV2HostMethod::Agent,
                write_mode: None,
                options: WorkflowV2HostOptions {
                    task: Some("Create implementation items.".into()),
                    ..WorkflowV2HostOptions::default()
                },
            },
            WorkflowV2HostCall {
                id: "tdl-implementation-wave-1".into(),
                method: WorkflowV2HostMethod::Fanout,
                write_mode: Some(WorkflowV2WriteMode::Coordinated),
                options: WorkflowV2HostOptions {
                    source: Some("implementationItems".into()),
                    task: Some("Implement wave 1.".into()),
                    target_files_from_item: true,
                    extra: [(
                        "condition".to_string(),
                        serde_json::json!("implementationItems.length > 0"),
                    )]
                    .into_iter()
                    .collect(),
                    ..WorkflowV2HostOptions::default()
                },
            },
        ];

        let spec = compatibility_spec_from_v2_calls("Implement a decomposed PRD.", &calls);
        let stage = spec
            .stages
            .iter()
            .find(|stage| stage.id == "tdl-implementation-wave-1")
            .unwrap();

        assert_eq!(
            stage.condition.as_deref(),
            Some("implementationItems.length > 0")
        );
        assert!(!stage.extra.contains_key("condition"));
        let yaml = serde_yaml_ng::to_string(&spec).expect("serialize spec");
        let reparsed: archon_workflow::WorkflowSpec =
            serde_yaml_ng::from_str(&yaml).expect("deserialize spec without duplicate fields");

        assert_eq!(
            reparsed
                .stages
                .iter()
                .find(|stage| stage.id == "tdl-implementation-wave-1")
                .and_then(|stage| stage.condition.as_deref()),
            Some("implementationItems.length > 0")
        );
    }

    #[test]
    fn v2_compatibility_spec_preserves_repository_root_from_task() {
        let calls = vec![WorkflowV2HostCall {
            id: "discover".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        }];

        let spec = compatibility_spec_from_v2_calls(
            "Implement against the repository /Volumes/Externalwork/archon-cli/archon-cli. Read the PRD.",
            &calls,
        );

        assert_eq!(
            spec.target_repository_root.as_deref(),
            Some("/Volumes/Externalwork/archon-cli/archon-cli")
        );
    }

    #[test]
    fn task_family_ids_are_parsed_without_domain_specific_prefixes() {
        let calls = vec![WorkflowV2HostCall {
            id: "implement-task-alpha-050".into(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions {
                source: Some("discover.items".into()),
                task: Some("Implement TASK-ALPHA-050 work items.".into()),
                ..WorkflowV2HostOptions::default()
            },
        }];

        let spec = compatibility_spec_from_v2_calls("Implement a decomposed PRD.", &calls);
        let stage = spec
            .stages
            .iter()
            .find(|stage| stage.id == "implement-task-alpha-050")
            .unwrap();

        assert_eq!(
            stage.extra["completion_task_ids"],
            serde_json::json!(["T050"])
        );
    }
}
