struct ParsedHostCall {
    call: WorkflowV2HostCall,
    end: usize,
}

fn parse_host_call_at(
    source: &str,
    call_start: usize,
) -> Result<ParsedHostCall, WorkflowV2HarnessError> {
    let method_start = call_start + "w.".len();
    let method_end = take_ident(source, method_start);
    let raw_method = &source[method_start..method_end];
    let method = WorkflowV2HostMethod::parse(raw_method)
        .ok_or_else(|| WorkflowV2HarnessError::UnsupportedHostMethod(raw_method.to_string()))?;
    let open = skip_ws(source, method_end);
    if !source[open..].starts_with('(') {
        return Err(WorkflowV2HarnessError::UnsupportedHostMethod(
            raw_method.to_string(),
        ));
    }
    let Some(close) = matching_delimiter(source, open, '(', ')') else {
        return Err(WorkflowV2HarnessError::HostCallRequiresLiteralId(
            method.as_str().to_string(),
        ));
    };
    let call_args = &source[open + 1..close];
    let (id_expr, args) = split_first_argument(call_args);
    let id = host_call_id_from_expr(method, id_expr, call_start, call_args)?;
    let dynamic_id = host_call_id_expr_is_dynamic(id_expr);
    validate_fanout_source(method, &id, args)?;
    let write_mode = parse_write_mode(method, &id, args)?;
    if is_write_intent(method, args) && write_mode.is_none() {
        return Err(WorkflowV2HarnessError::MissingWriteMode {
            method: method.as_str().to_string(),
            id,
        });
    }
    let mut options = parse_options(method, args);
    if dynamic_id {
        annotate_dynamic_host_id(&mut options, id_expr);
    }
    options.binding = binding_before_call(source, call_start);
    Ok(ParsedHostCall {
        call: WorkflowV2HostCall {
            id,
            method,
            write_mode,
            options,
        },
        end: close + 1,
    })
}

fn annotate_dynamic_host_id(options: &mut WorkflowV2HostOptions, id_expr: &str) {
    let id_expr = id_expr.trim();
    options
        .extra
        .insert("dynamic_id".to_string(), serde_json::Value::Bool(true));
    options.extra.insert(
        "dynamic_template".to_string(),
        serde_json::Value::Bool(true),
    );
    options.extra.insert(
        "dynamic_id_expr".to_string(),
        serde_json::Value::String(id_expr.to_string()),
    );
    if let Some((prefix, _)) = leading_string_literal(id_expr) {
        options.extra.insert(
            "dynamic_id_prefix".to_string(),
            serde_json::Value::String(sanitize_dynamic_id_prefix(&prefix)),
        );
    }
}

fn split_first_argument(args: &str) -> (&str, &str) {
    let mut depth = 0usize;
    let mut quote = None::<char>;
    let mut escaped = false;
    for (idx, ch) in args.char_indices() {
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
            ',' if depth == 0 => return (&args[..idx], &args[idx..]),
            _ => {}
        }
    }
    (args, "")
}

fn host_call_id_from_expr(
    method: WorkflowV2HostMethod,
    expr: &str,
    call_start: usize,
    call_args: &str,
) -> Result<String, WorkflowV2HarnessError> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(WorkflowV2HarnessError::HostCallRequiresLiteralId(
            method.as_str().to_string(),
        ));
    }
    let Some((prefix, remainder)) = leading_string_literal(expr) else {
        return Err(WorkflowV2HarnessError::HostCallRequiresLiteralId(
            method.as_str().to_string(),
        ));
    };
    if prefix.trim().is_empty() {
        return Err(WorkflowV2HarnessError::HostCallRequiresLiteralId(
            method.as_str().to_string(),
        ));
    }
    let remainder = remainder.trim();
    if remainder.is_empty() {
        return Ok(prefix);
    }
    if !remainder.starts_with('+') && !expr.trim_start().starts_with('`') {
        return Err(WorkflowV2HarnessError::HostCallRequiresLiteralId(
            method.as_str().to_string(),
        ));
    }
    let call_site_fingerprint = format!(
        "{}\n{}\n{}\n{}",
        method.as_str(),
        call_start,
        expr,
        normalized_call_site_args(call_args)
    );
    Ok(format!(
        "{}dynamic-{:08x}",
        sanitize_dynamic_id_prefix(&prefix),
        stable_expr_hash(&call_site_fingerprint)
    ))
}

fn normalized_call_site_args(call_args: &str) -> String {
    call_args
        .trim()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn host_call_id_expr_is_dynamic(expr: &str) -> bool {
    let expr = expr.trim();
    let Some((_prefix, remainder)) = leading_string_literal(expr) else {
        return false;
    };
    let remainder = remainder.trim();
    !remainder.is_empty() && (remainder.starts_with('+') || expr.starts_with('`'))
}

fn leading_string_literal(expr: &str) -> Option<(String, &str)> {
    let mut chars = expr.char_indices();
    let (_, quote) = chars.next()?;
    if !matches!(quote, '"' | '\'' | '`') {
        return None;
    }
    let mut escaped = false;
    let mut value = String::new();
    for (idx, ch) in chars {
        if escaped {
            value.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if quote == '`' && ch == '$' && expr[idx + ch.len_utf8()..].starts_with('{') {
            return Some((value, &expr[idx..]));
        }
        if ch == quote {
            return Some((value, &expr[idx + ch.len_utf8()..]));
        }
        value.push(ch);
    }
    None
}

fn sanitize_dynamic_id_prefix(prefix: &str) -> String {
    let mut output = prefix
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    while output.ends_with('-') || output.ends_with('_') {
        output.pop();
    }
    if output.is_empty() {
        "dynamic".to_string()
    } else {
        format!("{output}-")
    }
}

fn stable_expr_hash(expr: &str) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    for byte in expr.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}
