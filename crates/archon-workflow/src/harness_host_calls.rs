#[derive(Debug, Clone)]
struct HostCall {
    variable: Option<String>,
    method: String,
    id: String,
    depends_on: Option<Vec<String>>,
    source_var: Option<String>,
    output_artifact: Option<String>,
    items_from_artifact: Option<String>,
    items_from_var: Option<String>,
    inline_items: Vec<serde_json::Value>,
    filter: Option<String>,
    task: Option<String>,
    tier: Option<ProviderTier>,
    max_parallelism: Option<u32>,
    target_files: Vec<String>,
    verify_command: Option<String>,
    item_kind: Option<StageKind>,
    tool: Option<String>,
    allow_empty_items: bool,
    requires_item_target_files: bool,
}

fn host_calls(source: &str) -> WorkflowResult<Vec<HostCall>> {
    static CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)(?:(?:const|let|var)\s+(?P<var>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:await\s*)?)?\bw\.([A-Za-z_][A-Za-z0-9_]*)\s*\(\s*["']([^"']+)["'](?P<args>.*?)\)"#)
            .expect("host call regex compiles")
    });
    let mut calls = Vec::new();
    for captures in CALL_RE.captures_iter(source) {
        let variable = captures.name("var").map(|m| m.as_str().to_string());
        let method = captures.get(2).unwrap().as_str().to_string();
        if !allowed_method(&method) {
            return Err(WorkflowError::SpecInvalid(format!(
                "workflow harness uses unsupported host method w.{method}"
            )));
        }
        let id = sanitize_stage_id(captures.get(3).unwrap().as_str())?;
        let args = captures
            .name("args")
            .map(|m| m.as_str())
            .unwrap_or_default();
        calls.push(HostCall {
            variable,
            method,
            id,
            depends_on: parse_depends_on(args),
            source_var: parse_source_var_arg(args),
            output_artifact: parse_string_prop(args, &["outputArtifact", "output_artifact"]),
            items_from_artifact: parse_string_prop(
                args,
                &["itemsFromArtifact", "items_from_artifact"],
            ),
            items_from_var: parse_identifier_prop(
                args,
                &["itemsFromArtifact", "items_from_artifact"],
            ),
            inline_items: parse_inline_items(args),
            filter: parse_item_filter(args),
            task: parse_string_prop(args, &["task"]),
            tier: parse_tier(args),
            max_parallelism: parse_u32_prop(args, &["maxParallelism", "max_parallelism"]),
            target_files: parse_string_array_prop(
                args,
                &[
                    "targetFiles",
                    "target_files",
                    "expectedTargetFiles",
                    "expected_target_files",
                ],
            ),
            verify_command: parse_string_prop(args, &["verifyCommand", "verify_command"]),
            item_kind: parse_item_kind(args),
            tool: parse_string_prop(args, &["tool", "name"]),
            allow_empty_items: parse_bool_prop(args, &["allowEmptyItems", "allow_empty_items"])
                .unwrap_or(false),
            requires_item_target_files: parse_bool_prop(
                args,
                &[
                    "targetFilesFromItem",
                    "target_files_from_item",
                    "requiresItemTargetFiles",
                    "requires_item_target_files",
                ],
            )
            .unwrap_or(false),
        });
    }

    static ANY_HOST_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"\bw\.([A-Za-z_][A-Za-z0-9_]*)\s*\("#).expect("host method regex compiles")
    });
    for captures in ANY_HOST_RE.captures_iter(source) {
        let method = captures.get(1).unwrap().as_str();
        if !allowed_method(method) {
            return Err(WorkflowError::SpecInvalid(format!(
                "workflow harness uses unsupported host method w.{method}"
            )));
        }
    }
    Ok(calls)
}

fn executable_source(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut quote = None::<char>;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if let Some(active) = quote {
            out.push(ch);
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
            '"' | '\'' | '`' => {
                quote = Some(ch);
                out.push(ch);
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.next();
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                    }
                    if prev == '*' && next == '/' {
                        break;
                    }
                    prev = next;
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

fn reject_unsafe_source(source: &str) -> WorkflowResult<()> {
    let lower = source.to_ascii_lowercase();
    let blocked = [
        "import ",
        "export *",
        "require(",
        "eval(",
        "function(",
        "new function",
        "fs.",
        "node:fs",
        "child_process",
        "process.",
        "deno.",
        "bun.",
        "fetch(",
        "xmlhttprequest",
        "websocket",
        "net.",
        "tls.",
        "http.",
        "https.",
        "anthropic",
        "openai",
        "claude-",
        "gpt-",
        "gemini",
        "provider:",
        "model:",
    ];
    if let Some(hit) = blocked.iter().find(|needle| lower.contains(**needle)) {
        return Err(WorkflowError::SpecInvalid(format!(
            "workflow harness contains forbidden token `{hit}`"
        )));
    }
    static BLOCKED_RE: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
        vec![
            (
                "dynamic import",
                Regex::new(r#"\bimport\s*(?:\(|[{"'*A-Za-z_])"#).expect("blocked regex compiles"),
            ),
            (
                "require",
                Regex::new(r#"\brequire\s*\("#).expect("blocked regex compiles"),
            ),
            (
                "dynamic eval",
                Regex::new(r#"\beval\s*\("#).expect("blocked regex compiles"),
            ),
            (
                "provider literal",
                Regex::new(r#"\bprovider\s*:"#).expect("blocked regex compiles"),
            ),
            (
                "model literal",
                Regex::new(r#"\bmodel\s*:"#).expect("blocked regex compiles"),
            ),
        ]
    });
    if let Some((label, _)) = BLOCKED_RE.iter().find(|(_, regex)| regex.is_match(&lower)) {
        return Err(WorkflowError::SpecInvalid(format!(
            "workflow harness contains forbidden {label}"
        )));
    }
    Ok(())
}

fn allowed_method(method: &str) -> bool {
    matches!(
        method,
        "agent"
            | "fanout"
            | "reduce"
            | "tool"
            | "implementation"
            | "qualityGate"
            | "humanGate"
            | "checkpoint"
            | "saveArtifact"
            | "requireArtifact"
            | "runCompiledSpec"
    )
}

fn method_stage_kind(method: &str) -> WorkflowResult<StageKind> {
    match method {
        "agent" => Ok(StageKind::Agent),
        "fanout" => Ok(StageKind::Fanout),
        "reduce" => Ok(StageKind::Reduce),
        "tool" => Ok(StageKind::Tool),
        "implementation" => Ok(StageKind::Implementation),
        "qualityGate" => Ok(StageKind::QualityGate),
        "humanGate" => Ok(StageKind::HumanGate),
        "checkpoint" => Ok(StageKind::Checkpoint),
        _ => Err(WorkflowError::SpecInvalid(format!(
            "workflow harness method w.{method} is not executable"
        ))),
    }
}
