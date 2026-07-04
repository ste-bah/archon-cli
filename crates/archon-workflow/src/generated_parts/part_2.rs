fn normalize_under_specified_stages(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        let missing_tool = stage.kind == StageKind::Tool && !has_text(stage.tool.as_deref());
        let missing_condition =
            stage.kind == StageKind::Condition && !has_text(stage.condition.as_deref());
        if missing_tool || missing_condition {
            let original = format!("{:?}", stage.kind);
            stage.kind = StageKind::Agent;
            stage
                .extra
                .insert("normalized_from_kind".into(), Value::String(original));
        }
    }
}

fn promote_generated_implementation_agents(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        if stage.kind != StageKind::Agent || !agent_stage_implements_repo(stage) {
            continue;
        }
        stage.kind = StageKind::Implementation;
        stage.provider_tier.get_or_insert(ProviderTier::Coder);
        stage
            .extra
            .insert("normalized_from_kind".into(), Value::String("Agent".into()));
    }
}

fn agent_stage_implements_repo(stage: &StageSpec) -> bool {
    let id = stage.id.to_ascii_lowercase();
    let task = stage
        .task
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if contains_any(
        &id,
        &[
            "review",
            "audit",
            "test",
            "verify",
            "plan",
            "inventory",
            "discover",
            "synthesis",
            "report",
            "quality",
        ],
    ) || task.starts_with("perform read-only")
        || task.starts_with("produce an ordered implementation plan")
    {
        return false;
    }
    id == "implement"
        || id.starts_with("implement_")
        || id.starts_with("implement-")
        || id.ends_with("_implement")
        || id.ends_with("-implement")
        || id.contains("_implement_")
        || id.contains("-implement-")
        || task.starts_with("implement ")
        || task.contains("implement only")
        || task.contains("implement missing")
        || task.contains("modify repository")
        || task.contains("modify the repository")
}

fn infer_dependencies_from_io(spec: &mut WorkflowSpec) {
    let mut producers = BTreeMap::new();
    for stage in &spec.stages {
        for output in text_values(stage.extra.get("outputs")) {
            producers.insert(output, stage.id.clone());
        }
    }

    for stage in &mut spec.stages {
        if !stage.depends_on.is_empty() {
            continue;
        }
        for input in text_values(stage.extra.get("inputs")) {
            if let Some(dep) = producers.get(&input)
                && dep != &stage.id
                && !stage.depends_on.contains(dep)
            {
                stage.depends_on.push(dep.clone());
            }
        }
    }
}

fn normalize_targetless_implementation_stages(
    spec: &mut WorkflowSpec,
    allow_generated_target_inventory: bool,
) {
    let mut existing = spec
        .stages
        .iter()
        .map(|stage| stage.id.clone())
        .collect::<BTreeSet<_>>();
    let mut normalized = Vec::with_capacity(spec.stages.len());

    for mut stage in std::mem::take(&mut spec.stages) {
        if stage.kind != StageKind::Implementation || has_declared_targets(&stage) {
            normalized.push(stage);
            continue;
        }

        if let Some(targets) = loose_target_files(&stage) {
            stage.expected_target_files = targets;
            normalized.push(stage);
            continue;
        }

        if !allow_generated_target_inventory {
            normalized.push(stage);
            continue;
        }

        if let Some(items) = generated_task_items::implementation_items(&spec.task, &stage) {
            stage.extra.insert(
                "normalized_from_kind".into(),
                Value::String("Implementation".into()),
            );
            stage.kind = StageKind::Fanout;
            stage.item_kind = Some(StageKind::Implementation);
            stage.input = merge_inline_items(std::mem::take(&mut stage.input), items);
            normalized.push(stage);
            continue;
        }

        let inventory_id = unique_stage_id(&format!("{}-target-inventory", stage.id), &existing);
        existing.insert(inventory_id.clone());
        let inventory = implementation_target_inventory_stage(&inventory_id, &stage);
        stage.extra.insert(
            "normalized_from_kind".into(),
            Value::String("Implementation".into()),
        );
        stage.kind = StageKind::Fanout;
        stage.foreach = Some(format!("${{{inventory_id}.items}}"));
        stage.item_kind = Some(StageKind::Implementation);
        stage.max_parallelism.get_or_insert(1);
        if !stage.depends_on.contains(&inventory_id) {
            stage.depends_on.insert(0, inventory_id.clone());
        }
        normalized.push(inventory);
        normalized.push(stage);
    }

    spec.stages = normalized;
}

fn ensure_direct_implementation_work_units(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        if stage.kind != StageKind::Implementation
            || !crate::work_unit_coverage::stage_required_units(stage).is_empty()
        {
            continue;
        }
        stage.extra.insert(
            "required_work_units".into(),
            Value::Array(vec![Value::String(stage.id.clone())]),
        );
        stage.extra.insert(
            "normalized_work_unit_scope".into(),
            Value::String("stage_id".into()),
        );
    }
}

fn merge_inline_items(mut input: Value, items: Vec<Value>) -> Value {
    match input.as_object_mut() {
        Some(map) => {
            map.insert("items".into(), Value::Array(items));
            input
        }
        None => serde_json::json!({ "items": items }),
    }
}

fn has_declared_targets(stage: &StageSpec) -> bool {
    stage
        .expected_target_files
        .iter()
        .any(|target| has_text(Some(target)))
}

fn loose_target_files(stage: &StageSpec) -> Option<Vec<String>> {
    let mut targets = Vec::new();
    for key in [
        "target_files",
        "target_file",
        "target_path",
        "expected_target_files",
    ] {
        targets.extend(text_values(stage.extra.get(key)));
        targets.extend(text_values(stage.input.get(key)));
    }
    targets.retain(|target| !target.trim().is_empty());
    targets.sort();
    targets.dedup();
    (!targets.is_empty()).then_some(targets)
}

fn has_text(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn text_values(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}
