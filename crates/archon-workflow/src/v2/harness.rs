use std::collections::BTreeSet;

use regex::Regex;
use thiserror::Error;

use super::harness_safety::{code_without_string_literals, reject_unsafe_source, strip_comments};
use super::host_api::{
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode,
};

#[derive(Debug, Default, Clone)]
pub struct WorkflowV2HarnessValidator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2HarnessPlan {
    pub calls: Vec<WorkflowV2HostCall>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2HarnessError {
    #[error("workflow harness contains forbidden token `{0}`")]
    ForbiddenToken(&'static str),
    #[error("workflow harness uses unsupported host method w.{0}")]
    UnsupportedHostMethod(String),
    #[error("workflow harness declares no executable host calls")]
    NoHostCalls,
    #[error("host call w.{0}(...) must pass a non-empty string id as its first argument")]
    HostCallRequiresLiteralId(String),
    #[error("host call w.{method}('{id}') has invalid write mode `{value}`")]
    InvalidWriteMode {
        method: String,
        id: String,
        value: String,
    },
    #[error(
        "write-capable host call w.{method}('{id}') requires write: \"serial\", \"coordinated\", or \"worktree\""
    )]
    MissingWriteMode { method: String, id: String },
    #[error("fanout host call w.fanout('{0}') must include a typed item source argument")]
    UntypedFanout(String),
    #[error(
        "host call w.{method}('{id}') source `{source_id}` does not reference an earlier host call or declared script variable"
    )]
    UnknownSource {
        method: String,
        id: String,
        source_id: String,
    },
    #[error("workflow harness loop is unsupported: {0}")]
    UnsupportedLoop(String),
}

impl WorkflowV2HarnessValidator {
    pub fn validate(&self, source: &str) -> Result<WorkflowV2HarnessPlan, WorkflowV2HarnessError> {
        let executable = strip_comments(source);
        reject_unsafe_source(&executable)?;
        reject_opaque_host_api_usage(&executable)?;
        let calls = extract_host_calls(&executable)?;
        if calls.is_empty() {
            return Err(WorkflowV2HarnessError::NoHostCalls);
        }
        Ok(WorkflowV2HarnessPlan { calls })
    }
}

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
    let script_variables = declared_script_variables(body);
    collect_executable_calls(body, &mut calls, &[])?;
    rewrite_aliases(&mut calls);
    validate_source_references(&calls, &script_variables)?;
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
    let id = host_call_id_from_expr(method, id_expr)?;
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
    Ok(format!(
        "{}dynamic-{:08x}",
        sanitize_dynamic_id_prefix(&prefix),
        stable_expr_hash(expr)
    ))
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

fn skip_function_declaration(source: &str, start: usize) -> Option<usize> {
    let open = source[start..].find('{')? + start;
    matching_delimiter(source, open, '{', '}').map(|idx| idx + 1)
}

fn matching_delimiter(source: &str, open_idx: usize, open: char, close: char) -> Option<usize> {
    let mut idx = open_idx;
    let mut depth = 0usize;
    while idx < source.len() {
        let ch = source[idx..].chars().next()?;
        if matches!(ch, '"' | '\'' | '`') {
            idx = skip_quoted(source, idx, ch);
            continue;
        }
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(idx);
            }
        }
        idx += ch.len_utf8();
    }
    None
}

fn skip_quoted(source: &str, start: usize, quote: char) -> usize {
    let mut idx = start + quote.len_utf8();
    let mut escaped = false;
    while idx < source.len() {
        let ch = source[idx..]
            .chars()
            .next()
            .expect("quote index on char boundary");
        idx += ch.len_utf8();
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            break;
        }
    }
    idx
}

fn skip_ws(source: &str, mut idx: usize) -> usize {
    while idx < source.len() {
        let ch = source[idx..]
            .chars()
            .next()
            .expect("whitespace index on char boundary");
        if !ch.is_whitespace() {
            break;
        }
        idx += ch.len_utf8();
    }
    idx
}

fn take_ident(source: &str, mut idx: usize) -> usize {
    while idx < source.len() {
        let ch = source[idx..]
            .chars()
            .next()
            .expect("identifier index on char boundary");
        if !(ch == '_' || ch.is_ascii_alphanumeric()) {
            break;
        }
        idx += ch.len_utf8();
    }
    idx
}

fn starts_keyword(source: &str, idx: usize, keyword: &str) -> bool {
    source[idx..].starts_with(keyword)
        && source[idx + keyword.len()..]
            .chars()
            .next()
            .is_none_or(|ch| !(ch == '_' || ch.is_ascii_alphanumeric()))
        && is_boundary_before(source, idx)
}

fn is_boundary_before(source: &str, idx: usize) -> bool {
    if idx == 0 {
        return true;
    }
    source[..idx]
        .chars()
        .next_back()
        .is_none_or(|ch| !(ch == '_' || ch.is_ascii_alphanumeric()))
}

fn binding_before_call(source: &str, call_start: usize) -> Option<String> {
    let prefix = &source[..call_start];
    let statement_start = prefix
        .rfind(';')
        .or_else(|| prefix.rfind('\n'))
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let statement = prefix[statement_start..].trim();
    let re = Regex::new(r#"(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:await\s*)?$"#)
        .expect("binding regex compiles");
    re.captures(statement)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_string())
}

fn rewrite_aliases(calls: &mut [WorkflowV2HostCall]) {
    let aliases = calls
        .iter()
        .filter_map(|call| {
            call.options
                .binding
                .as_ref()
                .map(|binding| (binding.clone(), call.id.clone()))
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    if aliases.is_empty() {
        return;
    }
    for call in calls {
        if let Some(source) = call.options.source.as_mut() {
            *source = rewrite_source_expr(source, &aliases);
        }
        let rewritten_condition = call
            .options
            .extra
            .get("condition")
            .and_then(serde_json::Value::as_str)
            .map(|condition| rewrite_condition_expr(condition, &aliases));
        if let Some(condition) = rewritten_condition {
            call.options.extra.insert(
                "condition".to_string(),
                serde_json::Value::String(condition),
            );
        }
    }
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

fn rewrite_condition_expr(
    condition: &str,
    aliases: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut out = String::with_capacity(condition.len());
    let mut idx = 0usize;
    let mut quote = None::<char>;
    let mut escaped = false;
    while idx < condition.len() {
        let ch = condition[idx..]
            .chars()
            .next()
            .expect("condition index on char boundary");
        if let Some(active) = quote {
            out.push(ch);
            idx += ch.len_utf8();
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '"' | '\'' | '`') {
            quote = Some(ch);
            out.push(ch);
            idx += ch.len_utf8();
            continue;
        }
        if ch == '_' || ch.is_ascii_alphabetic() {
            let end = take_ident(condition, idx);
            let ident = &condition[idx..end];
            let previous = condition[..idx].chars().rev().find(|c| !c.is_whitespace());
            if previous != Some('.') {
                out.push_str(aliases.get(ident).map(String::as_str).unwrap_or(ident));
            } else {
                out.push_str(ident);
            }
            idx = end;
            continue;
        }
        out.push(ch);
        idx += ch.len_utf8();
    }
    out
}

fn validate_source_references(
    calls: &[WorkflowV2HostCall],
    script_variables: &BTreeSet<String>,
) -> Result<(), WorkflowV2HarnessError> {
    let mut seen = BTreeSet::<String>::new();
    for call in calls {
        if let Some(source) = call.options.source.as_deref() {
            if !source.trim_start().starts_with('{') {
                for source_id in source_call_ids(source) {
                    if !seen.contains(&source_id) && !script_variables.contains(&source_id) {
                        return Err(WorkflowV2HarnessError::UnknownSource {
                            method: call.method.as_str().to_string(),
                            id: call.id.clone(),
                            source_id,
                        });
                    }
                }
            }
        }
        if let Some(condition) = call
            .options
            .extra
            .get("condition")
            .and_then(serde_json::Value::as_str)
        {
            for source_id in condition_call_ids(condition) {
                if !seen.contains(&source_id) && !script_variables.contains(&source_id) {
                    return Err(WorkflowV2HarnessError::UnknownSource {
                        method: call.method.as_str().to_string(),
                        id: call.id.clone(),
                        source_id,
                    });
                }
            }
        }
        seen.insert(call.id.clone());
    }
    Ok(())
}

fn declared_script_variables(source: &str) -> BTreeSet<String> {
    let code = code_without_string_literals(source);
    let re = Regex::new(r#"\b(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\b"#)
        .expect("script variable regex compiles");
    re.captures_iter(&code)
        .filter_map(|captures| captures.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

fn source_call_ids(source: &str) -> Vec<String> {
    let trimmed = source.trim();
    let body = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    body.split(',')
        .filter_map(|part| {
            let head = part
                .trim()
                .split_once('.')
                .map(|(head, _)| head)
                .unwrap_or(part);
            let id = head.trim().trim_matches(|ch| ch == '"' || ch == '\'');
            if id.is_empty() || id.starts_with('{') {
                None
            } else {
                Some(id.to_string())
            }
        })
        .collect()
}

fn condition_call_ids(condition: &str) -> Vec<String> {
    let mut ids = BTreeSet::<String>::new();
    let mut idx = 0usize;
    let mut quote = None::<char>;
    let mut escaped = false;
    while idx < condition.len() {
        let ch = condition[idx..]
            .chars()
            .next()
            .expect("condition index on char boundary");
        if let Some(active) = quote {
            idx += ch.len_utf8();
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '"' | '\'' | '`') {
            quote = Some(ch);
            idx += ch.len_utf8();
            continue;
        }
        if ch == '_' || ch.is_ascii_alphabetic() {
            let end = take_ident(condition, idx);
            let ident = &condition[idx..end];
            let previous = condition[..idx].chars().rev().find(|c| !c.is_whitespace());
            let next = condition[end..].chars().find(|c| !c.is_whitespace());
            if previous != Some('.') && next == Some('.') {
                ids.insert(ident.to_string());
            }
            idx = end;
            continue;
        }
        idx += ch.len_utf8();
    }
    ids.into_iter().collect()
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
    if invalid_direct_item_source(value) {
        return None;
    }
    if value.starts_with('{') {
        if !string_prop(value, "type").is_some_and(|kind| kind == "static_items") {
            return None;
        }
        let items = expr_prop(value, "items")?;
        let items = normalize_fanout_item_source(&items)?;
        return Some(items);
    }
    Some(value.to_string())
}

fn invalid_direct_item_source(value: &str) -> bool {
    value.is_empty()
        || value.starts_with('[')
        || value.starts_with('"')
        || value.starts_with('\'')
        || value.starts_with(char::is_numeric)
        || value.starts_with("null")
        || value.starts_with("undefined")
        || value.starts_with("true")
        || value.starts_with("false")
}

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
    WorkflowV2HostOptions {
        binding: None,
        role: string_prop(args, "role").or_else(|| string_prop(args, "tier")),
        task: string_prop(args, "task"),
        source: source_expr(method, args),
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
    }
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
        WorkflowV2HostMethod::Checkpoint
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
        WorkflowV2HostMethod::Checkpoint
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
