fn rewrite_aliases(calls: &mut [WorkflowV2HostCall]) {
    let mut aliases = std::collections::BTreeMap::new();
    for call in calls {
        if let Some(source) = call.options.source.as_mut() {
            *source = rewrite_source_expr(source, &aliases);
        }
        if let Some(binding) = call.options.binding.as_ref() {
            aliases.insert(binding.clone(), call.id.clone());
        }
    }
}

fn reject_duplicate_host_call_ids(
    calls: &[WorkflowV2HostCall],
) -> Result<(), WorkflowV2HarnessError> {
    let mut seen = std::collections::BTreeSet::new();
    for call in calls {
        if !seen.insert(call.id.as_str()) {
            return Err(WorkflowV2HarnessError::DuplicateHostCallId(call.id.clone()));
        }
    }
    Ok(())
}

fn rewrite_source_expr(
    source: &str,
    aliases: &std::collections::BTreeMap<String, String>,
) -> String {
    let trimmed = source.trim();
    if let Some(list) = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        let rewritten = list
            .split(',')
            .map(|part| rewrite_source_expr(part.trim(), aliases))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("[{rewritten}]");
    }
    let (head, tail) = trimmed
        .split_once('.')
        .map(|(head, tail)| (head, Some(tail)))
        .unwrap_or((trimmed, None));
    let replacement = aliases.get(head).map(String::as_str).unwrap_or(head);
    match tail {
        Some(tail) => format!("{replacement}.{tail}"),
        None => replacement.to_string(),
    }
}

fn validate_fanout_source(
    method: WorkflowV2HostMethod,
    id: &str,
    args: &str,
) -> Result<(), WorkflowV2HarnessError> {
    if matches!(
        method,
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel
    ) && fanout_item_source_expr(args).is_none()
    {
        return Err(WorkflowV2HarnessError::UntypedFanout(id.to_string()));
    }
    Ok(())
}

fn fanout_item_source_expr(args: &str) -> Option<String> {
    let source = positional_source_expr(args, true)?;
    normalize_fanout_item_source(&source)
}

fn positional_source_expr(args: &str, allow_object_source: bool) -> Option<String> {
    let trimmed = args.trim_start();
    let rest = trimmed.strip_prefix(',')?.trim_start();
    let mut depth = 0usize;
    let mut quote = None::<char>;
    let mut escaped = false;
    for (idx, ch) in rest.char_indices() {
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
            ']' | '}' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return non_empty_expr(&rest[..idx], allow_object_source),
            _ => {}
        }
    }
    non_empty_expr(rest, allow_object_source)
}

fn normalize_fanout_item_source(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("null")
        || value.eq_ignore_ascii_case("undefined")
    {
        return None;
    }
    Some(value.to_string())
}
