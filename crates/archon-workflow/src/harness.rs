//! Legacy JavaScript-to-`WorkflowSpec` compiler.
//!
//! This module is retained for saved templates and explicit legacy spec/harness
//! flows. Generated `/workflow run <objective>` must use `crate::v2::harness`
//! plus the V2 result store/runtime instead of compiling dynamic harnesses back
//! into YAML-stage execution.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::spec::{
    ArtifactPolicy, ProviderTier, ReducerKind, StageKind, StageSpec, WORKFLOW_SCHEMA, WorkflowSpec,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessPhase {
    pub id: String,
    pub method: String,
    pub kind: StageKind,
    pub depends_on: Vec<String>,
    pub write_capable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct HarnessCompiler;

impl HarnessCompiler {
    pub fn validate(&self, source: &str) -> WorkflowResult<Vec<HarnessPhase>> {
        let executable = executable_source(source);
        reject_unsafe_source(&executable)?;
        let calls = host_calls(&executable)?;
        let mut phases = Vec::new();
        let mut prior_stage = None::<String>;
        let mut variables = BTreeMap::<String, String>::new();
        let mut artifacts = BTreeMap::<String, String>::new();
        for call in calls {
            if matches!(call.method.as_str(), "saveArtifact" | "requireArtifact") {
                continue;
            }
            if call.method == "runCompiledSpec" {
                phases.push(HarnessPhase {
                    id: call.id,
                    method: call.method,
                    kind: StageKind::Checkpoint,
                    depends_on: Vec::new(),
                    write_capable: false,
                });
                continue;
            }
            let kind = method_stage_kind(&call.method)?;
            let stage_kind = call.item_kind.unwrap_or(kind);
            let depends_on = call_depends_on(&call, prior_stage.as_deref(), &variables, &artifacts);
            let phase = HarnessPhase {
                id: call.id.clone(),
                method: call.method.clone(),
                kind,
                depends_on,
                write_capable: stage_kind == StageKind::Implementation,
            };
            prior_stage = Some(phase.id.clone());
            if let Some(variable) = call.variable.clone() {
                variables.insert(variable, phase.id.clone());
            }
            if let Some(artifact) = call.output_artifact.clone() {
                artifacts.insert(artifact, phase.id.clone());
            }
            phases.push(phase);
        }
        if phases.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "workflow harness declares no executable host calls".to_string(),
            ));
        }
        Ok(phases)
    }

    pub fn compile(&self, source: &str, name: &str, task: &str) -> WorkflowResult<WorkflowSpec> {
        self.validate(source)?;
        let executable = executable_source(source);
        let calls = host_calls(&executable)?;
        let required_artifacts = calls
            .iter()
            .filter(|call| call.method == "requireArtifact")
            .map(|call| call.id.clone())
            .collect::<Vec<_>>();
        let mut prior_stage = None::<String>;
        let mut provider_tiers = BTreeMap::new();
        provider_tiers.insert(ProviderTier::Planner, "auto".to_string());
        provider_tiers.insert(ProviderTier::Researcher, "auto".to_string());
        provider_tiers.insert(ProviderTier::Coder, "auto".to_string());
        provider_tiers.insert(ProviderTier::Critic, "auto".to_string());
        provider_tiers.insert(ProviderTier::Reducer, "auto".to_string());
        let mut stages = Vec::new();
        let mut variables = BTreeMap::<String, String>::new();
        let mut artifacts = BTreeMap::<String, String>::new();
        for call in calls {
            if matches!(
                call.method.as_str(),
                "runCompiledSpec" | "saveArtifact" | "requireArtifact"
            ) {
                continue;
            }
            let kind = method_stage_kind(&call.method)?;
            let depends_on = call_depends_on(&call, prior_stage.as_deref(), &variables, &artifacts);
            if (kind == StageKind::Implementation
                || call.item_kind == Some(StageKind::Implementation))
                && !task_allows_repository_edits(task)
            {
                return Err(WorkflowError::SpecInvalid(format!(
                    "workflow harness declares write-capable stage '{}' for a non-editing task",
                    call.id
                )));
            }
            let stage = stage_for_call(&call, kind, depends_on, task)?;
            prior_stage = Some(stage.id.clone());
            if let Some(variable) = call.variable.clone() {
                variables.insert(variable, stage.id.clone());
            }
            if let Some(artifact) = call.output_artifact.clone() {
                artifacts.insert(artifact, stage.id.clone());
            }
            stages.push(stage);
        }
        if stages.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "imported compiled-spec wrappers cannot be recompiled without the compiled spec"
                    .to_string(),
            ));
        }
        if !required_artifacts.is_empty() {
            attach_required_artifacts(&mut stages, required_artifacts)?;
        }
        let mut spec = WorkflowSpec {
            schema: WORKFLOW_SCHEMA.to_string(),
            name: sanitize_name(name),
            task: task.to_string(),
            target_repository_root: None,
            max_parallelism: 8,
            max_agents: 200,
            provider_tiers,
            stages,
            artifact_policy: ArtifactPolicy::default(),
            permissions: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            learning_hooks: Vec::new(),
        };
        ensure_fanout_sources(&mut spec);
        spec.validate()?;
        Ok(spec)
    }
}

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

fn parse_depends_on(args: &str) -> Option<Vec<String>> {
    static DEPS_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"depends_on["']?\s*:\s*\[(?P<deps>[^\]]*)\]"#)
            .expect("depends_on regex compiles")
    });
    let captures = DEPS_RE.captures(args)?;
    let deps = captures
        .name("deps")?
        .as_str()
        .split(',')
        .map(|value| value.trim().trim_matches('"').trim_matches('\''))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!deps.is_empty()).then_some(deps)
}

fn parse_source_var_arg(args: &str) -> Option<String> {
    static SOURCE_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^\s*,\s*([A-Za-z_][A-Za-z0-9_]*)\s*,"#)
            .expect("source argument regex compiles")
    });
    SOURCE_RE
        .captures(args)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_string())
}

fn parse_string_prop(args: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let regex = Regex::new(&format!(
            r#"(?s)(?:{}|["']{}["'])\s*:\s*["']([^"']+)["']"#,
            regex::escape(key),
            regex::escape(key)
        ))
        .ok()?;
        regex
            .captures(args)
            .and_then(|captures| captures.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn parse_identifier_prop(args: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let regex = Regex::new(&format!(
            r#"(?s)(?:{}|["']{}["'])\s*:\s*([A-Za-z_][A-Za-z0-9_]*)"#,
            regex::escape(key),
            regex::escape(key)
        ))
        .ok()?;
        regex
            .captures(args)
            .and_then(|captures| captures.get(1))
            .map(|m| m.as_str().trim().to_string())
            .filter(|value| !matches!(value.as_str(), "true" | "false" | "null" | "undefined"))
    })
}

fn parse_u32_prop(args: &str, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| {
        let regex = Regex::new(&format!(
            r#"(?s)(?:{}|["']{}["'])\s*:\s*(\d+)"#,
            regex::escape(key),
            regex::escape(key)
        ))
        .ok()?;
        regex
            .captures(args)
            .and_then(|captures| captures.get(1))
            .and_then(|m| m.as_str().parse::<u32>().ok())
    })
}

fn parse_bool_prop(args: &str, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| {
        let regex = Regex::new(&format!(
            r#"(?s)(?:{}|["']{}["'])\s*:\s*(true|false)"#,
            regex::escape(key),
            regex::escape(key)
        ))
        .ok()?;
        regex
            .captures(args)
            .and_then(|captures| captures.get(1))
            .and_then(|m| m.as_str().parse::<bool>().ok())
    })
}

fn parse_string_array_prop(args: &str, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| {
            let regex = Regex::new(&format!(
                r#"(?s)(?:{}|["']{}["'])\s*:\s*\[(?P<items>[^\]]*)\]"#,
                regex::escape(key),
                regex::escape(key)
            ))
            .ok()?;
            let captures = regex.captures(args)?;
            Some(
                captures
                    .name("items")?
                    .as_str()
                    .split(',')
                    .map(|value| value.trim().trim_matches('"').trim_matches('\''))
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap_or_default()
}

fn parse_inline_items(args: &str) -> Vec<serde_json::Value> {
    let Some(body) = extract_array_prop(args, "items") else {
        return Vec::new();
    };
    split_object_literals(&body)
        .into_iter()
        .filter_map(|object| {
            let mut item = serde_json::Map::new();
            if let Some(name) = parse_string_prop(&object, &["name", "id", "task_id"]) {
                item.insert("id".into(), serde_json::Value::String(name.clone()));
                item.insert("task_id".into(), serde_json::Value::String(name));
            }
            if let Some(task) = parse_string_prop(&object, &["task"]) {
                item.insert("task".into(), serde_json::Value::String(task));
            }
            let target_files = parse_string_array_prop(
                &object,
                &[
                    "targetFiles",
                    "target_files",
                    "expectedTargetFiles",
                    "expected_target_files",
                ],
            );
            if !target_files.is_empty() {
                item.insert(
                    "target_files".into(),
                    serde_json::Value::Array(
                        target_files
                            .into_iter()
                            .map(serde_json::Value::String)
                            .collect(),
                    ),
                );
            }
            (!item.is_empty()).then_some(serde_json::Value::Object(item))
        })
        .collect()
}

fn parse_item_filter(args: &str) -> Option<String> {
    let body = extract_object_prop(args, "itemFilter")?;
    let (field, values) = [
        ("phase", vec!["phases", "phase"]),
        (
            "task_id",
            vec!["tasks", "task", "task_ids", "taskIds", "task_id", "taskId"],
        ),
        ("id", vec!["ids", "id"]),
    ]
    .into_iter()
    .find_map(|(field, keys)| {
        let values = parse_string_array_prop(&body, &keys);
        (!values.is_empty()).then_some((field, values))
    })?;
    let quoted = values
        .into_iter()
        .map(|value| format!("'{}'", value.replace('\'', "")))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("item.{field} in [{quoted}]"))
}

fn extract_array_prop(args: &str, key: &str) -> Option<String> {
    extract_balanced_prop(args, key, '[', ']')
}

fn extract_object_prop(args: &str, key: &str) -> Option<String> {
    extract_balanced_prop(args, key, '{', '}')
}

fn extract_balanced_prop(args: &str, key: &str, open: char, close: char) -> Option<String> {
    let regex = Regex::new(&format!(
        r#"(?s)(?:{}|["']{}["'])\s*:\s*{}"#,
        regex::escape(key),
        regex::escape(key),
        regex::escape(&open.to_string())
    ))
    .ok()?;
    let hit = regex.find(args)?;
    let start = hit.end() - open.len_utf8();
    balanced_slice(args, start, open, close)
}

fn balanced_slice(args: &str, start: usize, open: char, close: char) -> Option<String> {
    let mut depth = 0usize;
    let mut quote = None::<char>;
    let mut escaped = false;
    let mut body_start = None;
    for (offset, ch) in args[start..].char_indices() {
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
            ch if ch == open => {
                depth += 1;
                if depth == 1 {
                    body_start = Some(start + offset + ch.len_utf8());
                }
            }
            ch if ch == close => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return body_start.map(|inner| args[inner..start + offset].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn split_object_literals(array_body: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut depth = 0usize;
    let mut quote = None::<char>;
    let mut escaped = false;
    let mut start = None::<usize>;
    for (idx, ch) in array_body.char_indices() {
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
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0
                    && let Some(start) = start.take()
                {
                    objects.push(array_body[start..=idx].to_string());
                }
            }
            _ => {}
        }
    }
    objects
}

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
