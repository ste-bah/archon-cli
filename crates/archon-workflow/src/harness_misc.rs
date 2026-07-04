fn parse_tier(args: &str) -> Option<ProviderTier> {
    parse_string_prop(args, &["tier", "providerTier", "provider_tier"])
        .map(|value| value.to_ascii_lowercase())
        .and_then(|value| match value.as_str() {
            "planner" => Some(ProviderTier::Planner),
            "researcher" => Some(ProviderTier::Researcher),
            "coder" => Some(ProviderTier::Coder),
            "critic" => Some(ProviderTier::Critic),
            "cheap" => Some(ProviderTier::Cheap),
            "vision" => Some(ProviderTier::Vision),
            "local" => Some(ProviderTier::Local),
            "reducer" => Some(ProviderTier::Reducer),
            _ => None,
        })
}

fn parse_item_kind(args: &str) -> Option<StageKind> {
    parse_string_prop(args, &["itemKind", "item_kind", "kind"]).and_then(|value| {
        if matches!(
            value.as_str(),
            "implementation" | "implementation_fanout" | "write"
        ) {
            Some(StageKind::Implementation)
        } else {
            None
        }
    })
}

fn task_allows_repository_edits(task: &str) -> bool {
    let lower = task.to_ascii_lowercase();
    [
        "implement",
        "edit",
        "modify",
        "change",
        "fix",
        "repair",
        "remediate",
        "migrate",
        "refactor",
        "update",
        "add ",
        "create ",
        "delete",
        "remove",
        "write code",
        "repository modifications",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn sanitize_stage_id(value: &str) -> WorkflowResult<String> {
    let safe: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = safe.trim_matches('-').to_string();
    if trimmed.is_empty() {
        return Err(WorkflowError::SpecInvalid(
            "workflow harness stage id is empty".to_string(),
        ));
    }
    Ok(trimmed)
}

fn sanitize_name(value: &str) -> String {
    let safe: String = value
        .chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch.is_whitespace() || ch == '-' || ch == '_' {
                Some('-')
            } else {
                None
            }
        })
        .take(48)
        .collect();
    let trimmed = safe.trim_matches('-');
    if trimmed.is_empty() {
        "dynamic-workflow".to_string()
    } else {
        trimmed.to_string()
    }
}
