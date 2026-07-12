use serde_json::Value;

use crate::command::workflow_live::workflow_live_generated_lifecycle_support as support;

pub(super) fn prepare_verification_items(
    items: Vec<Value>,
    project_artifact_root: Option<&str>,
) -> Vec<Value> {
    items
        .into_iter()
        .map(|mut item| {
            if let (Some(root), Some(object)) = (project_artifact_root, item.as_object_mut()) {
                object.insert(
                    "project_artifact_root".to_string(),
                    Value::String(root.to_string()),
                );
            }
            item
        })
        .collect()
}

pub(super) fn verification_options(items: &[Value], task: &str, focused: bool) -> Value {
    let mut options = serde_json::json!({ "tier": "coder", "task": task });
    if focused {
        options["itemKind"] = Value::String("focused_verification".to_string());
    }
    if items_have_cargo_commands(items) {
        options["maxParallelism"] = serde_json::json!(1);
    }
    options
}

fn items_have_cargo_commands(items: &[Value]) -> bool {
    items.iter().any(|item| {
        support::raw_strings(
            item,
            &[
                "focused_verification",
                "commands",
                "command",
                "expected_evidence",
            ],
        )
        .iter()
        .any(|text| text.to_ascii_lowercase().contains("cargo "))
    })
}
