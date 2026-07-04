fn reject_opaque_host_api_usage(source: &str) -> Result<(), WorkflowV2HarnessError> {
    let code = code_without_string_literals(source);
    let alias_re = Regex::new(
        r#"\b(?:const|let|var)\s+(?:[A-Za-z_][A-Za-z0-9_]*|\{[^}]+\})\s*=\s*w\s*(?:[;\n,\)]|$)"#,
    )
    .expect("host alias regex compiles");
    if alias_re.is_match(&code) {
        return Err(WorkflowV2HarnessError::ForbiddenToken("host API alias"));
    }

    let mut idx = 0usize;
    while idx < source.len() {
        let Some(ch) = source[idx..].chars().next() else {
            break;
        };
        if matches!(ch, '"' | '\'' | '`') {
            idx = skip_quoted(source, idx, ch);
            continue;
        }
        if ch == 'w' && is_boundary_before(source, idx) {
            let after_w = idx + ch.len_utf8();
            let after_ws = skip_ws(source, after_w);
            if source[after_ws..].starts_with('[') {
                return Err(WorkflowV2HarnessError::ForbiddenToken(
                    "host API bracket access",
                ));
            }
            if source[after_ws..].starts_with('.') {
                if after_ws != after_w {
                    return Err(WorkflowV2HarnessError::ForbiddenToken(
                        "host API reference outside direct call",
                    ));
                }
                let method_start = after_ws + 1;
                let method_end = take_ident(source, method_start);
                let open = skip_ws(source, method_end);
                if source[open..].starts_with('(') {
                    idx = open + 1;
                    continue;
                }
                return Err(WorkflowV2HarnessError::ForbiddenToken(
                    "host API reference outside direct call",
                ));
            }
        }
        idx += ch.len_utf8();
    }
    Ok(())
}

fn extract_host_calls(source: &str) -> Result<Vec<WorkflowV2HostCall>, WorkflowV2HarnessError> {
    let mut calls = Vec::new();
    let body = workflow_body(source).unwrap_or(source);
    collect_executable_calls(body, &mut calls, &[])?;
    rewrite_aliases(&mut calls);
    reject_duplicate_host_call_ids(&calls)?;
    Ok(calls)
}

fn workflow_body(source: &str) -> Option<&str> {
    let function_start = source.find("function workflow")?;
    let open = source[function_start..].find('{')? + function_start;
    let close = matching_delimiter(source, open, '{', '}')?;
    Some(&source[open + 1..close])
}

fn collect_executable_calls(
    source: &str,
    calls: &mut Vec<WorkflowV2HostCall>,
    conditions: &[String],
) -> Result<(), WorkflowV2HarnessError> {
    let mut idx = 0usize;
    while idx < source.len() {
        idx = skip_ws(source, idx);
        if idx >= source.len() {
            break;
        }
        if starts_keyword(source, idx, "function") {
            idx = skip_function_declaration(source, idx).unwrap_or(idx + 1);
            continue;
        }
        if starts_keyword(source, idx, "if") {
            if let Some(next) = collect_if_calls(source, idx, calls, conditions)? {
                idx = next;
                continue;
            }
        }
        if starts_keyword(source, idx, "while") {
            if let Some(next) = collect_while_calls(source, idx, calls, conditions)? {
                idx = next;
                continue;
            }
        }
        if starts_keyword(source, idx, "for") {
            if let Some(next) = collect_for_calls(source, idx, calls, conditions)? {
                idx = next;
                continue;
            }
        }
        let Some(ch) = source[idx..].chars().next() else {
            break;
        };
        if matches!(ch, '"' | '\'' | '`') {
            idx = skip_quoted(source, idx, ch);
            continue;
        }
        if source[idx..].starts_with("w.") && is_boundary_before(source, idx) {
            let mut parsed = parse_host_call_at(source, idx)?;
            annotate_conditions(&mut parsed.call, conditions);
            calls.push(parsed.call);
            idx = parsed.end;
            continue;
        }
        idx += ch.len_utf8();
    }
    Ok(())
}

fn collect_if_calls(
    source: &str,
    if_start: usize,
    calls: &mut Vec<WorkflowV2HostCall>,
    conditions: &[String],
) -> Result<Option<usize>, WorkflowV2HarnessError> {
    let mut idx = skip_ws(source, if_start + "if".len());
    if !source[idx..].starts_with('(') {
        return Ok(None);
    }
    let Some(condition_close) = matching_delimiter(source, idx, '(', ')') else {
        return Ok(None);
    };
    let condition = source[idx + 1..condition_close].trim();
    idx = skip_ws(source, condition_close + 1);
    if !source[idx..].starts_with('{') {
        return Ok(None);
    }
    let Some(then_close) = matching_delimiter(source, idx, '{', '}') else {
        return Ok(None);
    };
    let then_body = &source[idx + 1..then_close];
    let mut next = skip_ws(source, then_close + 1);
    let else_body = if starts_keyword(source, next, "else") {
        next = skip_ws(source, next + "else".len());
        if source[next..].starts_with('{') {
            let Some(else_close) = matching_delimiter(source, next, '{', '}') else {
                return Ok(None);
            };
            let body = Some(&source[next + 1..else_close]);
            next = else_close + 1;
            body
        } else {
            None
        }
    } else {
        None
    };
    match condition {
        "true" => collect_executable_calls(then_body, calls, conditions)?,
        "false" => {
            if let Some(body) = else_body {
                collect_executable_calls(body, calls, conditions)?;
            }
        }
        _ => {
            let mut then_conditions = conditions.to_vec();
            then_conditions.push(condition.to_string());
            collect_executable_calls(then_body, calls, &then_conditions)?;
            if let Some(body) = else_body {
                let mut else_conditions = conditions.to_vec();
                else_conditions.push(format!("!({condition})"));
                collect_executable_calls(body, calls, &else_conditions)?;
            }
        }
    }
    Ok(Some(next))
}

fn collect_for_calls(
    source: &str,
    for_start: usize,
    calls: &mut Vec<WorkflowV2HostCall>,
    conditions: &[String],
) -> Result<Option<usize>, WorkflowV2HarnessError> {
    let mut idx = skip_ws(source, for_start + "for".len());
    if !source[idx..].starts_with('(') {
        return Ok(None);
    }
    let Some(header_close) = matching_delimiter(source, idx, '(', ')') else {
        return Ok(None);
    };
    let header = source[idx + 1..header_close].trim();
    idx = skip_ws(source, header_close + 1);
    if !source[idx..].starts_with('{') {
        return Ok(None);
    }
    let Some(body_close) = matching_delimiter(source, idx, '{', '}') else {
        return Ok(None);
    };
    let body = &source[idx + 1..body_close];
    let Some(source_expr) = for_of_source_expr(header) else {
        let mut loop_calls = Vec::new();
        collect_executable_calls(body, &mut loop_calls, conditions)?;
        if loop_calls.is_empty() {
            return Ok(Some(body_close + 1));
        }
        require_dynamic_loop_host_ids("for", header, &loop_calls)?;
        for mut call in loop_calls {
            annotate_runtime_loop(&mut call, "for", header);
            calls.push(call);
        }
        return Ok(Some(body_close + 1));
    };
    let mut loop_calls = Vec::new();
    collect_executable_calls(body, &mut loop_calls, conditions)?;
    for mut call in loop_calls {
        if matches!(
            call.method,
            WorkflowV2HostMethod::Agent | WorkflowV2HostMethod::Implementation
        ) {
            call.method = WorkflowV2HostMethod::Fanout;
        }
        if call.options.source.is_none() {
            call.options.source = Some(source_expr.clone());
        }
        call.options.extra.insert(
            "loop_source".to_string(),
            serde_json::Value::String(source_expr.clone()),
        );
        calls.push(call);
    }
    Ok(Some(body_close + 1))
}

fn collect_while_calls(
    source: &str,
    while_start: usize,
    calls: &mut Vec<WorkflowV2HostCall>,
    conditions: &[String],
) -> Result<Option<usize>, WorkflowV2HarnessError> {
    let mut idx = skip_ws(source, while_start + "while".len());
    if !source[idx..].starts_with('(') {
        return Ok(None);
    }
    let Some(condition_close) = matching_delimiter(source, idx, '(', ')') else {
        return Ok(None);
    };
    let condition = source[idx + 1..condition_close].trim();
    idx = skip_ws(source, condition_close + 1);
    if !source[idx..].starts_with('{') {
        return Ok(None);
    }
    let Some(body_close) = matching_delimiter(source, idx, '{', '}') else {
        return Ok(None);
    };
    let body = &source[idx + 1..body_close];
    let mut loop_calls = Vec::new();
    collect_executable_calls(body, &mut loop_calls, conditions)?;
    if loop_calls.is_empty() {
        return Ok(Some(body_close + 1));
    }
    require_dynamic_loop_host_ids("while", condition, &loop_calls)?;
    for mut call in loop_calls {
        annotate_runtime_loop(&mut call, "while", condition);
        calls.push(call);
    }
    Ok(Some(body_close + 1))
}

fn for_of_source_expr(header: &str) -> Option<String> {
    let (_, source) = header.split_once(" of ")?;
    let source = source.trim();
    if source.is_empty()
        || source.starts_with('{')
        || source.starts_with('[')
        || source.starts_with('"')
        || source.starts_with('\'')
        || source.starts_with(char::is_numeric)
    {
        return None;
    }
    Some(source.to_string())
}

fn require_dynamic_loop_host_ids(
    loop_kind: &str,
    loop_expr: &str,
    calls: &[WorkflowV2HostCall],
) -> Result<(), WorkflowV2HarnessError> {
    let static_call = calls.iter().find(|call| {
        !call
            .options
            .extra
            .get("dynamic_id")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    });
    if let Some(call) = static_call {
        return Err(WorkflowV2HarnessError::UnsupportedLoop(format!(
            "{loop_kind} loop `{loop_expr}` contains host call w.{}('{}') with a static id; adaptive runtime loops must use deterministic dynamic ids with a literal prefix, for example \"{}-\" + iteration",
            call.method.as_str(),
            call.id,
            call.id
        )));
    }
    Ok(())
}

fn annotate_runtime_loop(call: &mut WorkflowV2HostCall, loop_kind: &str, loop_expr: &str) {
    call.options.extra.insert(
        "runtime_loop".to_string(),
        serde_json::Value::String(loop_kind.to_string()),
    );
    let key = match loop_kind {
        "while" => "loop_condition",
        "for" => "loop_header",
        _ => "loop_expression",
    };
    call.options.extra.insert(
        key.to_string(),
        serde_json::Value::String(loop_expr.to_string()),
    );
}

fn annotate_conditions(call: &mut WorkflowV2HostCall, conditions: &[String]) {
    if conditions.is_empty() {
        return;
    }
    let expression = conditions.join(" && ");
    call.options.extra.insert(
        "condition".to_string(),
        serde_json::Value::String(expression),
    );
}
