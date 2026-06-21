use serde_json::Value;

use crate::spec::{StageKind, StageSpec, WorkflowSpec};

pub(crate) fn ensure_remediation_contracts(spec: &mut WorkflowSpec) {
    for stage in &mut spec.stages {
        strip_unresolved_verify_command(stage);
        let text = stage_text(stage);
        if is_remediation_inventory(&text) {
            declare_items_output(stage);
        }
        if is_remediation_impl(&text) {
            stage
                .extra
                .entry("allow_empty_items".into())
                .or_insert(Value::Bool(true));
        }
        if is_post_remediation(&text) {
            stage
                .extra
                .entry("allow_empty_remediation_noop".into())
                .or_insert(Value::Bool(true));
            stage
                .extra
                .entry("failure_aware".into())
                .or_insert(Value::Bool(true));
            if stage.kind == StageKind::Fanout {
                stage
                    .extra
                    .entry("allow_empty_items".into())
                    .or_insert(Value::Bool(true));
            }
        }
    }
}

fn strip_unresolved_verify_command(stage: &mut StageSpec) {
    let Some(command) = stage.verify_command.as_deref() else {
        return;
    };
    if !contains_workflow_template(command) {
        return;
    }
    let command = stage.verify_command.take().unwrap_or_default();
    stage.extra.insert(
        "removed_unresolved_verify_command".into(),
        Value::String(command),
    );
}

fn declare_items_output(stage: &mut StageSpec) {
    let mut outputs = match stage.extra.remove("outputs") {
        Some(Value::Array(values)) => values,
        Some(Value::String(value)) => vec![Value::String(value)],
        Some(other) => vec![other],
        None => Vec::new(),
    };
    if !outputs.iter().any(|value| {
        value
            .as_str()
            .is_some_and(|text| text.eq_ignore_ascii_case("items"))
    }) {
        outputs.push(Value::String("items".into()));
    }
    stage.extra.insert("outputs".into(), Value::Array(outputs));
}

fn contains_workflow_template(command: &str) -> bool {
    let mut rest = command;
    while let Some(start) = rest.find("${") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find('}') else {
            return false;
        };
        if rest[..end].contains('.') {
            return true;
        }
        rest = &rest[end + 1..];
    }
    false
}

fn stage_text(stage: &StageSpec) -> String {
    format!(
        "{} {}",
        stage.id.to_ascii_lowercase(),
        stage
            .task
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
    )
}

fn is_remediation_inventory(text: &str) -> bool {
    text.contains("remediation_inventory")
        || text.contains("remediation-inventory")
        || text.contains("remediation inventory")
}

fn is_remediation_impl(text: &str) -> bool {
    (text.contains("remediation_impl")
        || text.contains("remediation-impl")
        || text.contains("remediation implementation")
        || text.contains("remediation fixes"))
        && !is_post_remediation(text)
}

fn is_post_remediation(text: &str) -> bool {
    text.contains("post_remediation")
        || text.contains("post-remediation")
        || text.contains("post remediation")
}
