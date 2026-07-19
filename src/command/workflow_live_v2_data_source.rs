pub(super) fn fanout_items_for_call(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<Vec<WorkflowV2FanoutItem>> {
    let (source, values) = fanout_source_values(execution, v2_store)?;
    let role = execution
        .call
        .options
        .role
        .clone()
        .unwrap_or_else(|| role_for_v2_call(execution.call.method).to_string());
    Ok(values
        .into_iter()
        .enumerate()
        .map(|(idx, value)| {
            let item_id = fanout_item_id(&value, idx);
            let mut branch_call = execution.call.clone();
            branch_call.id = format!("{}-{item_id}", execution.call.id);
            branch_call.options.source = None;
            if branch_call.options.target_files_from_item {
                let item_targets = target_files_from_value(&value);
                if !item_targets.is_empty() {
                    branch_call.options.target_files = item_targets;
                }
            }
            branch_call.method = if branch_call.write_mode.is_some() {
                WorkflowV2HostMethod::Implementation
            } else {
                WorkflowV2HostMethod::Agent
            };
            let mut input = serde_json::json!({
                "fanout_call_id": execution.call.id,
                "fanout_item_id": item_id,
                "source": source,
                "item": value,
            });
            stamp_focused_verification_input(&execution.call.id, &mut input);
            WorkflowV2FanoutItem::read_only(
                branch_call.id.clone(),
                role.clone(),
                branch_call,
                input,
            )
        })
        .collect())
}

fn fanout_source_values(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<(String, Vec<serde_json::Value>)> {
    if let Some(source_data) = execution.input.get("source_data") {
        return Ok((
            execution
                .call
                .options
                .source
                .clone()
                .unwrap_or_else(|| "workflow.js source argument".to_string()),
            array_from_source_data(source_data)?,
        ));
    }
    let source = execution.call.options.source.as_deref().ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "w.{}('{}') requires a typed source expression or runtime source argument",
            execution.call.method.as_str(),
            execution.call.id
        ))
    })?;
    Ok((source.to_string(), resolve_fanout_source(source, v2_store)?))
}

fn array_from_source_data(
    source_data: &serde_json::Value,
) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
    if let Some(values) = source_data.as_array() {
        return Ok(values.clone());
    }
    if let Some(values) = source_data
        .get("items")
        .and_then(serde_json::Value::as_array)
    {
        return Ok(values.clone());
    }
    Err(WorkflowError::SpecInvalid(
        "fanout runtime source argument resolved to non-array typed data".to_string(),
    ))
}

fn resolve_fanout_source(
    source: &str,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
    let cursor = resolve_source_value(source, v2_store)?;
    cursor.as_array().cloned().ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "fanout source '{source}' resolved to non-array typed data"
        ))
    })
}

pub(super) fn resolve_source_value(
    source: &str,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<serde_json::Value> {
    let trimmed = source.trim();
    if let Some(list) = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        let mut values = Vec::new();
        for part in list.split(',') {
            let source = part.trim();
            if !source.is_empty() {
                values.push(resolve_single_source_value(source, v2_store)?);
            }
        }
        return Ok(serde_json::Value::Array(values));
    }
    resolve_single_source_value(trimmed, v2_store)
}

fn resolve_single_source_value(
    source: &str,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<serde_json::Value> {
    if let Some((call_id, path)) = source.split_once('.') {
        return source_value_from_call_path(call_id, Some(path), source, v2_store);
    }
    if source
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return source_value_from_call_path(source, None, source, v2_store);
    }
    Err(WorkflowError::SpecInvalid(format!(
        "source '{source}' must reference a prior call or field, for example inventory.items"
    )))
}

fn source_value_from_call_path(
    call_id: &str,
    path: Option<&str>,
    source: &str,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<serde_json::Value> {
    let record = v2_store.load_call_record(call_id)?.ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "source '{source}' references missing prior call '{call_id}'"
        ))
    })?;
    let mut cursor = if record.result.data.is_null() {
        serde_json::to_value(&record.result)?
    } else {
        record.result.data.clone()
    };
    if let Some(path) = path {
        for segment in path.split('.') {
            cursor = cursor.get(segment).cloned().ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "source '{source}' field '{segment}' is absent from prior result data"
                ))
            })?;
        }
    }
    Ok(cursor)
}

fn fanout_item_id(value: &serde_json::Value, idx: usize) -> String {
    value
        .get("id")
        .or_else(|| value.get("task_id"))
        .or_else(|| value.get("work_unit_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(sanitize_v2_id)
        .unwrap_or_else(|| idx.to_string())
}

fn target_files_from_value(value: &serde_json::Value) -> Vec<String> {
    value
        .get("target_files")
        .or_else(|| value.get("expected_target_files"))
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn sanitize_v2_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}
