fn value_present(value: Option<&serde_json::Value>) -> bool {
    match value {
        Some(serde_json::Value::String(value)) => !value.trim().is_empty(),
        Some(serde_json::Value::Array(values)) => !values.is_empty(),
        Some(serde_json::Value::Object(values)) => !values.is_empty(),
        Some(serde_json::Value::Bool(_)) | Some(serde_json::Value::Number(_)) => true,
        _ => false,
    }
}

fn raw_task_refs(value: &serde_json::Value) -> Vec<String> {
    sorted_unique(non_empty_strings(
        value
            .get("canonical_task_ids")
            .or_else(|| value.get("canonicalTaskIds"))
            .or_else(|| value.get("task_ids"))
            .or_else(|| value.get("taskIds"))
            .or_else(|| value.get("task_id")),
    ))
}

fn item_id(value: &serde_json::Value) -> Option<String> {
    value
        .get("item_id")
        .or_else(|| value.get("id"))
        .or_else(|| value.get("task_id"))
        .or_else(|| value.get("work_unit_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn dependency_refs(value: &serde_json::Value) -> Vec<String> {
    sorted_unique(non_empty_strings(
        value
            .get("dependency_ids")
            .or_else(|| value.get("dependencyIds"))
            .or_else(|| value.get("depends_on"))
            .or_else(|| value.get("dependsOn")),
    ))
}

fn normalize_dependency_refs(
    value: &serde_json::Value,
    universe: &TaskUniverse,
    item_to_tasks: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut normalized = BTreeSet::new();
    for dep in dependency_refs(value) {
        let raw_dep = dep.strip_prefix("__unresolved__:").unwrap_or(&dep);
        if let Some(canonical) = universe.resolve(raw_dep) {
            normalized.insert(canonical);
            continue;
        }
        if let Some(task_ids) = item_to_tasks.get(raw_dep) {
            normalized.extend(task_ids.iter().cloned());
        } else {
            normalized.insert(format!("__unresolved__:{raw_dep}"));
        }
    }
    normalized.into_iter().collect()
}

fn short_task_alias(canonical: &str) -> Option<String> {
    let digits = canonical.rsplit('-').next()?;
    (!digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| format!("T{digits}"))
}

fn unresolved_dependencies(graph: &WorkflowV2SourceTaskGraph) -> Vec<String> {
    graph
        .items
        .iter()
        .flat_map(|item| item.dependency_ids.iter())
        .filter_map(|dep| dep.strip_prefix("__unresolved__:").map(str::to_string))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn graph_invalid_reasons(
    wave_kind: DynamicSourceKind,
    graph: &WorkflowV2SourceTaskGraph,
) -> Vec<String> {
    let mut reasons = BTreeSet::new();
    let mut assigned: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for item in &graph.items {
        let item_tasks = item
            .canonical_task_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for dep in &item.dependency_ids {
            if dep.starts_with("__unresolved__:") {
                continue;
            }
            if item_tasks.contains(dep.as_str()) {
                reasons.insert(format!(
                    "source graph item '{}' depends on canonical task '{}' that it also claims",
                    item.item_id, dep
                ));
            }
        }
        for task_id in &item.canonical_task_ids {
            assigned
                .entry(task_id.as_str())
                .or_default()
                .push(item.item_id.as_str());
        }
    }
    if should_reject_duplicate_task_assignment(wave_kind) {
        insert_duplicate_task_reasons(&mut reasons, assigned);
    }
    reasons.into_iter().collect()
}

fn insert_duplicate_task_reasons(
    reasons: &mut BTreeSet<String>,
    assigned: BTreeMap<&str, Vec<&str>>,
) {
    for (task_id, item_ids) in assigned {
        let unique_item_ids = item_ids.into_iter().collect::<BTreeSet<_>>();
        if unique_item_ids.len() > 1 {
            reasons.insert(format!(
                "source graph canonical task '{}' is assigned by multiple source items: {}",
                task_id,
                unique_item_ids.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }
}

fn should_reject_duplicate_task_assignment(wave_kind: DynamicSourceKind) -> bool {
    !allows_duplicate_task_assignment(wave_kind)
}

fn allows_duplicate_task_assignment(wave_kind: DynamicSourceKind) -> bool {
    matches!(
        wave_kind,
        DynamicSourceKind::Remediation
            | DynamicSourceKind::ReviewRemediation
            | DynamicSourceKind::FocusedVerification
            | DynamicSourceKind::ReviewVerification
    )
}

fn non_empty_strings(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items.iter().flat_map(value_to_strings).collect(),
        Some(serde_json::Value::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        Some(other) => value_to_strings(other).into_iter().collect(),
        None => Vec::new(),
    }
}

fn value_to_strings(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::Array(items) => items.iter().flat_map(value_to_strings).collect(),
        serde_json::Value::String(value) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        serde_json::Value::Object(object) => object
            .get("path")
            .or_else(|| object.get("id"))
            .or_else(|| object.get("summary"))
            .or_else(|| object.get("command"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| vec![value.to_string()])
            .unwrap_or_default(),
        serde_json::Value::Number(value) => vec![value.to_string()],
        serde_json::Value::Bool(value) => vec![value.to_string()],
        serde_json::Value::Null => Vec::new(),
    }
}

fn string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    non_empty_strings(value)
}

fn sorted_unique(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Fingerprint the source graph, or report why it could not be fingerprinted.
///
/// `graph` is a typed struct, so `to_value` genuinely can fail. Defaulting to
/// `Value::Null` on failure would make every unserializable graph hash to the
/// digest of `null` — colliding with each other and with a legitimately empty
/// graph — and that hash lands in a reuse cache key. The error is returned so
/// the caller records "no fingerprint" plus a reason instead.
fn source_fingerprint(graph: &WorkflowV2SourceTaskGraph) -> Result<String, serde_json::Error> {
    Ok(stable_hash(&serde_json::to_value(graph)?))
}
