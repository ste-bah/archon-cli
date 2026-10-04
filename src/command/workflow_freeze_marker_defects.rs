//! Complete marker diagnostics; identities use structural slots, never rejected values.
use archon_workflow::defect::ValidationDefect;
use serde_json::Value;

pub(crate) fn marker_defects(candidate: &Value, skeleton: bool) -> Vec<ValidationDefect> {
    let mut defects = Vec::new();
    let lists: &[&str] = if skeleton {
        &["tasks"]
    } else {
        &["entries", "supplementary", "acceptance"]
    };
    for list in lists {
        for (index, entry) in candidate
            .get(*list)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let id_field = if skeleton { "task_id" } else { "id" };
            let id = entry.get(id_field).and_then(Value::as_str).unwrap_or("?");
            let subject = format!("{list}/{index}");
            let fields: &[&str] = if skeleton {
                &["file_name", "depends_on", "deliverable_contracts"]
            } else {
                &["check"]
            };
            for field in fields {
                if let Some(value) = entry.get(*field) {
                    collect(
                        value,
                        &subject,
                        &format!("/{list}/{index}/{field}"),
                        field,
                        id,
                        skeleton,
                        &mut defects,
                    );
                }
            }
        }
    }
    defects
}

fn collect(
    value: &Value,
    subject: &str,
    display: &str,
    location: &str,
    id: &str,
    skeleton: bool,
    defects: &mut Vec<ValidationDefect>,
) {
    match value {
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                collect(
                    value,
                    subject,
                    &format!("{display}/{index}"),
                    &format!("{location}/{index}"),
                    id,
                    skeleton,
                    defects,
                );
            }
        }
        Value::Object(items) => {
            for (slot, (key, value)) in items.iter().enumerate() {
                // Display retains the JSON pointer, but identity never retains
                // submitted property names (which a malformed shape can control).
                collect(
                    value,
                    subject,
                    &format!("{display}/{key}"),
                    &format!("{location}/field/{slot}"),
                    id,
                    skeleton,
                    defects,
                );
            }
        }
        Value::String(_) if archon_workflow::events::redaction_marker_path(value).is_some() => {
            let kind = if skeleton { "task" } else { "check" };
            defects.push(ValidationDefect::new(
                "redacted_executable_value",
                subject,
                location,
                format!("{kind} '{id}': {}", super::marker_guidance(display)),
            ));
        }
        _ => {}
    }
}
