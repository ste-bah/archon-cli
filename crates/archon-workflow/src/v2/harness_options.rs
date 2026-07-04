fn parse_write_mode(
    method: WorkflowV2HostMethod,
    id: &str,
    args: &str,
) -> Result<Option<WorkflowV2WriteMode>, WorkflowV2HarnessError> {
    let Some(raw) = string_prop(args, "write") else {
        return Ok(None);
    };
    if raw.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    WorkflowV2WriteMode::parse(&raw).map(Some).ok_or_else(|| {
        WorkflowV2HarnessError::InvalidWriteMode {
            method: method.as_str().to_string(),
            id: id.to_string(),
            value: raw,
        }
    })
}

fn parse_options(method: WorkflowV2HostMethod, args: &str) -> WorkflowV2HostOptions {
    let source = source_expr(method, args);
    let mut options = WorkflowV2HostOptions {
        binding: None,
        role: string_prop(args, "role").or_else(|| string_prop(args, "tier")),
        task: string_prop(args, "task"),
        source: source.clone(),
        item_kind: string_prop(args, "itemKind").or_else(|| string_prop(args, "item_kind")),
        target_files: string_array_prop(args, "targetFiles")
            .or_else(|| string_array_prop(args, "target_files"))
            .unwrap_or_default(),
        target_files_from_item: bool_prop(args, "targetFilesFromItem")
            .or_else(|| bool_prop(args, "target_files_from_item"))
            .unwrap_or(false),
        max_parallelism: usize_prop(args, "maxParallelism")
            .or_else(|| usize_prop(args, "max_parallelism")),
        extra: Default::default(),
    };
    if let Some(raw_source) = source {
        options.extra.insert(
            "raw_source_expr".to_string(),
            serde_json::Value::String(raw_source),
        );
    }
    if method == WorkflowV2HostMethod::Tool
        && let Some(tool) = string_prop(args, "tool").or_else(|| string_prop(args, "name"))
    {
        options.extra.insert(
            "tool".to_string(),
            serde_json::Value::String(tool.trim().to_string()),
        );
    }
    options
}

fn is_write_intent(method: WorkflowV2HostMethod, args: &str) -> bool {
    matches!(
        method,
        WorkflowV2HostMethod::Agent
            | WorkflowV2HostMethod::Fanout
            | WorkflowV2HostMethod::Implementation
    ) && (string_prop(args, "role").is_some_and(|role| role.eq_ignore_ascii_case("coder"))
        || string_prop(args, "itemKind")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("implementation"))
        || string_prop(args, "item_kind")
            .is_some_and(|kind| kind.eq_ignore_ascii_case("implementation"))
        || method == WorkflowV2HostMethod::Implementation)
}

fn source_expr(method: WorkflowV2HostMethod, args: &str) -> Option<String> {
    let allow_object_source = matches!(
        method,
        WorkflowV2HostMethod::Checkpoint | WorkflowV2HostMethod::SaveArtifact
    );
    if !matches!(
        method,
        WorkflowV2HostMethod::Agent
            | WorkflowV2HostMethod::Checkpoint
            | WorkflowV2HostMethod::Fanout
            | WorkflowV2HostMethod::Parallel
            | WorkflowV2HostMethod::Reduce
            | WorkflowV2HostMethod::FinalReport
            | WorkflowV2HostMethod::QualityGate
            | WorkflowV2HostMethod::SaveArtifact
            | WorkflowV2HostMethod::RequireArtifact
    ) {
        return None;
    }
    if matches!(
        method,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel
    ) {
        return fanout_item_source_expr(args);
    }
    let rest = args.trim_start().strip_prefix(',')?.trim_start();
    if let Some(object_source) = object_option_source_expr(method, rest) {
        return Some(object_source);
    }
    positional_source_expr(args, allow_object_source)
}

fn object_option_source_expr(method: WorkflowV2HostMethod, rest: &str) -> Option<String> {
    let trimmed = rest.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    let keys: &[&str] = match method {
        WorkflowV2HostMethod::Agent
        | WorkflowV2HostMethod::Checkpoint
        | WorkflowV2HostMethod::Reduce
        | WorkflowV2HostMethod::FinalReport
        | WorkflowV2HostMethod::QualityGate => &["inputs", "source", "input"],
        WorkflowV2HostMethod::SaveArtifact => &["artifact", "inputs", "source", "input"],
        WorkflowV2HostMethod::RequireArtifact => &["artifact", "inputs", "source", "input"],
        _ => &[],
    };
    keys.iter()
        .find_map(|key| expr_prop(trimmed, key).and_then(|value| normalize_source_prop(&value)))
}

fn non_empty_expr(raw: &str, allow_object_source: bool) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || (!allow_object_source && value.starts_with('{')) {
        None
    } else {
        Some(value.to_string())
    }
}

fn expr_prop(args: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(
        r#"(?s)(?:{}|["']{}["'])\s*:\s*"#,
        regex::escape(key),
        regex::escape(key)
    ))
    .ok()?;
    let value = &args[re.find(args)?.end()..];
    take_value_expr(value)
}

fn take_value_expr(value: &str) -> Option<String> {
    let value = value.trim_start();
    if value.is_empty() {
        return None;
    }
    let mut depth = 0usize;
    let mut quote = None::<char>;
    let mut escaped = false;
    for (idx, ch) in value.char_indices() {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' | '`' => quote = Some(ch),
            '[' | '{' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            '}' if depth == 0 => return non_empty_expr(&value[..idx], true),
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return non_empty_expr(&value[..idx], true),
            _ => {}
        }
    }
    non_empty_expr(value, true)
}

fn normalize_source_prop(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(body) = value
        .strip_prefix('{')
        .and_then(|inner| inner.strip_suffix('}'))
    {
        let ids = body
            .split(',')
            .filter_map(|part| {
                let candidate = part
                    .split_once(':')
                    .map(|(_, rhs)| rhs)
                    .unwrap_or(part)
                    .trim()
                    .trim_matches(|ch| ch == '"' || ch == '\'');
                if candidate.is_empty() {
                    None
                } else {
                    Some(candidate.to_string())
                }
            })
            .collect::<Vec<_>>();
        return match ids.as_slice() {
            [] => None,
            [single] => Some(single.clone()),
            many => Some(format!("[{}]", many.join(", "))),
        };
    }
    Some(value.to_string())
}

fn string_prop(args: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(
        r#"(?s)(?:{}|["']{}["'])\s*:\s*["']([^"']+)["']"#,
        regex::escape(key),
        regex::escape(key)
    ))
    .ok()?;
    re.captures(args)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().trim().to_string())
        .filter(|value| !value.is_empty())
}

fn bool_prop(args: &str, key: &str) -> Option<bool> {
    let re = Regex::new(&format!(
        r#"(?s)(?:{}|["']{}["'])\s*:\s*(true|false)"#,
        regex::escape(key),
        regex::escape(key)
    ))
    .ok()?;
    re.captures(args)
        .and_then(|captures| captures.get(1))
        .and_then(|m| match m.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
}

fn usize_prop(args: &str, key: &str) -> Option<usize> {
    let re = Regex::new(&format!(
        r#"(?s)(?:{}|["']{}["'])\s*:\s*([0-9]+)"#,
        regex::escape(key),
        regex::escape(key)
    ))
    .ok()?;
    re.captures(args)
        .and_then(|captures| captures.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

fn string_array_prop(args: &str, key: &str) -> Option<Vec<String>> {
    let re = Regex::new(&format!(
        r#"(?s)(?:{}|["']{}["'])\s*:\s*\[(?P<body>[^\]]*)\]"#,
        regex::escape(key),
        regex::escape(key)
    ))
    .ok()?;
    let body = re.captures(args)?.name("body")?.as_str();
    let string_re = Regex::new(r#"["']([^"']+)["']"#).ok()?;
    Some(
        string_re
            .captures_iter(body)
            .filter_map(|captures| captures.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
    )
}
