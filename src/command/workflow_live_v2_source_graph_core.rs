#[derive(Debug, Clone)]
pub(super) struct DynamicWaveSourceMetadata {
    pub(super) source_metadata_required: bool,
    pub(super) source_fingerprint: Option<String>,
    pub(super) source_task_graph: Option<WorkflowV2SourceTaskGraph>,
    pub(super) unresolved_dependencies: Vec<String>,
    pub(super) invalid_reason: Option<String>,
}

impl DynamicWaveSourceMetadata {
    pub(super) fn empty() -> Self {
        Self {
            source_metadata_required: false,
            source_fingerprint: None,
            source_task_graph: None,
            unresolved_dependencies: Vec::new(),
            invalid_reason: None,
        }
    }
}

pub(super) fn dynamic_wave_source_metadata(
    execution: &WorkflowV2CallExecution,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    target_repository_root: Option<&str>,
) -> DynamicWaveSourceMetadata {
    let Some(wave_kind) = dynamic_source_kind(execution) else {
        return DynamicWaveSourceMetadata::empty();
    };
    let Some(task_universe) = task_universe else {
        return DynamicWaveSourceMetadata {
            source_metadata_required: true,
            invalid_reason: Some("authoritative task universe is unavailable".to_string()),
            ..DynamicWaveSourceMetadata::empty()
        };
    };
    let Some(items) = execution
        .input
        .get("source_data")
        .and_then(serde_json::Value::as_array)
    else {
        return DynamicWaveSourceMetadata {
            source_metadata_required: true,
            invalid_reason: Some("write fanout source_data must be an array".to_string()),
            ..DynamicWaveSourceMetadata::empty()
        };
    };
    let authoritative_task_universe = task_universe;
    let task_universe = TaskUniverse::from_authoritative(authoritative_task_universe);
    let Some(graph) = source_task_graph_from_items(
        items,
        &task_universe,
        authoritative_task_universe,
        wave_kind,
        target_repository_root,
    ) else {
        let issue = source_data_contract_issue(
            items,
            &task_universe,
            authoritative_task_universe,
            wave_kind,
        );
        return DynamicWaveSourceMetadata {
            source_metadata_required: true,
            invalid_reason: Some(format!("{wave_kind} {issue}")),
            ..DynamicWaveSourceMetadata::empty()
        };
    };
    let graph_invalid_reasons = graph_invalid_reasons(wave_kind, &graph);
    if !graph_invalid_reasons.is_empty() {
        return DynamicWaveSourceMetadata {
            source_metadata_required: true,
            source_fingerprint: None,
            source_task_graph: Some(graph),
            unresolved_dependencies: Vec::new(),
            invalid_reason: Some(graph_invalid_reasons.join("; ")),
        };
    }
    let unresolved_dependencies = unresolved_dependencies(&graph);
    if !unresolved_dependencies.is_empty() {
        return DynamicWaveSourceMetadata {
            source_metadata_required: true,
            source_fingerprint: None,
            source_task_graph: Some(graph),
            unresolved_dependencies,
            invalid_reason: None,
        };
    }
    let source_fingerprint = Some(source_fingerprint(&graph));
    DynamicWaveSourceMetadata {
        source_metadata_required: true,
        source_fingerprint,
        source_task_graph: Some(graph),
        unresolved_dependencies,
        invalid_reason: None,
    }
}

pub(super) fn complete_source_task_graph(
    mut graph: WorkflowV2SourceTaskGraph,
    result: &archon_workflow::WorkflowV2Result,
) -> WorkflowV2SourceTaskGraph {
    let mut completed = BTreeSet::new();
    let item_assignments = graph
        .items
        .iter()
        .map(|item| {
            (
                item.item_id.clone(),
                item.canonical_task_ids
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if let Some(outcomes) = result
        .data
        .get("outcomes")
        .and_then(serde_json::Value::as_array)
    {
        for outcome in outcomes {
            let Some(item_id) = outcome
                .get("item_id")
                .or_else(|| outcome.get("id"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let Some(assigned_ids) = assigned_ids_for_outcome(item_id, &graph, &item_assignments)
            else {
                continue;
            };
            let Some(status) = outcome.get("status").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if !matches!(status, "accepted" | "noop") {
                continue;
            }
            for task_id in string_array(outcome.get("canonical_task_ids")) {
                if assigned_ids.contains(&task_id)
                    && graph
                        .canonical_task_universe
                        .iter()
                        .any(|known| known == &task_id)
                {
                    completed.insert(task_id);
                }
            }
        }
    }
    graph.completed_ids = completed.into_iter().collect();
    graph
}

pub(super) fn input_hash_with_source_fingerprint(
    input: &serde_json::Value,
    source_fingerprint: Option<&str>,
) -> String {
    let mut value = input.clone();
    if let Some(fingerprint) = source_fingerprint
        && let Some(object) = value.as_object_mut() {
            object.remove("source_data");
            object.insert(
                "source_fingerprint".to_string(),
                serde_json::Value::String(fingerprint.to_string()),
            );
        }
    stable_hash(&value)
}

fn assigned_ids_for_outcome(
    item_id: &str,
    graph: &WorkflowV2SourceTaskGraph,
    item_assignments: &BTreeMap<String, BTreeSet<String>>,
) -> Option<BTreeSet<String>> {
    if let Some(assigned_ids) = item_assignments.get(item_id) {
        return Some(assigned_ids.clone());
    }
    if graph
        .items
        .iter()
        .any(|item| item.canonical_task_ids.iter().any(|known| known == item_id))
    {
        return Some(BTreeSet::from([item_id.to_string()]));
    }
    None
}
