fn stage_for_call(
    call: &HostCall,
    kind: StageKind,
    depends_on: Vec<String>,
    workflow_task: &str,
) -> WorkflowResult<StageSpec> {
    let tier = call.tier.or_else(|| match kind {
        StageKind::Implementation => Some(ProviderTier::Coder),
        StageKind::Reduce => Some(ProviderTier::Reducer),
        StageKind::QualityGate | StageKind::Fanout => Some(ProviderTier::Critic),
        _ => Some(ProviderTier::Planner),
    });
    let task = call
        .task
        .clone()
        .unwrap_or_else(|| format!("{workflow_task}\n\nWorkflow phase: {}", call.id));
    let mut stage = StageSpec {
        id: call.id.clone(),
        kind,
        task: Some(task),
        agent: matches!(
            kind,
            StageKind::Agent | StageKind::Fanout | StageKind::Tool | StageKind::Implementation
        )
        .then(|| call.id.clone()),
        foreach: None,
        reducer: (kind == StageKind::Reduce).then_some(ReducerKind::EvidenceWeightedReport),
        tool: (kind == StageKind::Tool)
            .then(|| call.tool.clone().unwrap_or_else(|| call.id.clone())),
        condition: None,
        depends_on,
        provider_tier: tier,
        retry: Default::default(),
        input: serde_json::Value::Object(Default::default()),
        model: None,
        provider: None,
        expected_target_files: call.target_files.clone(),
        verify_command: call.verify_command.clone(),
        max_parallelism: call.max_parallelism,
        item_kind: call.item_kind,
        filter: None,
        extra: BTreeMap::new(),
    };
    if stage.kind == StageKind::Fanout {
        if !call.inline_items.is_empty() {
            stage.input = serde_json::json!({ "items": call.inline_items });
        } else if let Some(dep) = fanout_source_stage(call, &stage.depends_on) {
            stage.foreach = Some(format!("${{{dep}.items}}"));
        }
        stage.filter = call.filter.clone();
        if stage.item_kind == Some(StageKind::Implementation) {
            if stage.expected_target_files.is_empty() && !call.requires_item_target_files {
                return Err(WorkflowError::SpecInvalid(format!(
                    "implementation fanout stage '{}' requires targetFiles or targetFilesFromItem: true",
                    call.id
                )));
            }
            stage.extra.insert(
                "required_work_units".into(),
                serde_json::json!([call.id.clone()]),
            );
            if call.allow_empty_items {
                stage
                    .extra
                    .insert("allow_empty_items".into(), serde_json::json!(true));
            }
        }
    }
    if stage.kind == StageKind::Implementation {
        if stage.expected_target_files.is_empty() {
            return Err(WorkflowError::SpecInvalid(format!(
                "implementation stage '{}' requires targetFiles",
                call.id
            )));
        }
        stage.extra.insert(
            "required_work_units".into(),
            serde_json::json!([call.id.clone()]),
        );
    }
    Ok(stage)
}

fn call_depends_on(
    call: &HostCall,
    prior_stage: Option<&str>,
    variables: &BTreeMap<String, String>,
    artifacts: &BTreeMap<String, String>,
) -> Vec<String> {
    if let Some(explicit) = &call.depends_on {
        return explicit.clone();
    }
    let mut deps = Vec::new();
    if let Some(source) = call
        .items_from_artifact
        .as_ref()
        .and_then(|artifact| artifacts.get(artifact))
        .or_else(|| {
            call.items_from_var
                .as_ref()
                .and_then(|var| variables.get(var))
        })
        .or_else(|| call.source_var.as_ref().and_then(|var| variables.get(var)))
    {
        push_unique_dep(&mut deps, source);
    }
    if let Some(prior) = prior_stage {
        push_unique_dep(&mut deps, prior);
    }
    deps
}

fn fanout_source_stage(call: &HostCall, depends_on: &[String]) -> Option<String> {
    if !call.inline_items.is_empty() {
        return None;
    }
    if let Some(_artifact) = &call.items_from_artifact {
        return depends_on.first().cloned();
    }
    if call.items_from_var.is_some() {
        return depends_on.first().cloned();
    }
    if call.source_var.is_some() {
        return depends_on.first().cloned();
    }
    depends_on.last().cloned()
}

fn push_unique_dep(deps: &mut Vec<String>, dep: &str) {
    if !deps.iter().any(|existing| existing == dep) {
        deps.push(dep.to_string());
    }
}

fn attach_required_artifacts(
    stages: &mut [StageSpec],
    required_artifacts: Vec<String>,
) -> WorkflowResult<()> {
    let Some(stage) = stages
        .iter_mut()
        .rev()
        .find(|stage| stage.kind == StageKind::QualityGate)
    else {
        return Err(WorkflowError::SpecInvalid(
            "workflow harness requires artifacts but declares no quality gate".into(),
        ));
    };
    let mut input = stage.input.clone();
    let object = input
        .as_object_mut()
        .ok_or_else(|| WorkflowError::SpecInvalid("quality gate input must be an object".into()))?;
    object.insert(
        "required_artifacts".into(),
        serde_json::Value::Array(
            required_artifacts
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    stage.input = input;
    Ok(())
}

fn ensure_fanout_sources(spec: &mut WorkflowSpec) {
    let fanout_deps = spec
        .stages
        .iter()
        .filter(|stage| stage.kind == StageKind::Fanout)
        .filter_map(|stage| stage.foreach.as_deref())
        .filter_map(|foreach| {
            crate::spec::parse_foreach_accessor(foreach).map(|(stage, _accessor)| stage.to_string())
        })
        .collect::<BTreeSet<_>>();
    for stage in &mut spec.stages {
        if fanout_deps.contains(&stage.id) {
            ensure_items_output(stage);
            append_items_output_contract(stage);
        }
    }
}

const ITEMS_OUTPUT_TASK_CONTRACT: &str = concat!(
    "\n\nStructured item output contract: Return a single parseable JSON or YAML document with a top-level `items` array. ",
    "Do not return only markdown/prose. Each `items[]` entry must include a stable `id` or `task_id`, a concise `task` or `summary`, concrete evidence, ",
    "and `target_files` when downstream implementation work may edit repository files. If no downstream item should run, return `{\"items\": []}` with concise evidence."
);

fn ensure_items_output(stage: &mut StageSpec) {
    let mut outputs = match stage.extra.remove("outputs") {
        Some(serde_json::Value::Array(values)) => values,
        Some(serde_json::Value::String(value)) => vec![serde_json::Value::String(value)],
        Some(_) | None => Vec::new(),
    };
    if !outputs
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|value| value.eq_ignore_ascii_case("items"))
    {
        outputs.push(serde_json::json!("items"));
    }
    stage
        .extra
        .insert("outputs".to_string(), serde_json::Value::Array(outputs));
}

fn append_items_output_contract(stage: &mut StageSpec) {
    let task = stage
        .task
        .get_or_insert_with(|| format!("Workflow phase: {}", stage.id));
    if !task.contains("Structured item output contract") {
        task.push_str(ITEMS_OUTPUT_TASK_CONTRACT);
    }
}
