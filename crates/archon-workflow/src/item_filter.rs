use serde_json::Value;

use crate::error::{WorkflowError, WorkflowResult};
use crate::fanout::FanoutItem;
use crate::spec::StageSpec;

#[derive(Clone, Copy)]
enum Operator {
    Eq,
    Ne,
    In,
    Contains,
}

struct Predicate {
    path: Vec<String>,
    op: Operator,
    values: Vec<String>,
}

pub(crate) fn validate_filter(stage_id: &str, raw: &str) -> WorkflowResult<()> {
    parse_predicate(stage_id, raw).map(|_| ())
}

pub(crate) fn apply_stage_filter(
    stage: &StageSpec,
    items: Vec<FanoutItem>,
    allow_empty: bool,
) -> WorkflowResult<Vec<FanoutItem>> {
    let Some(raw) = stage
        .filter
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Ok(items);
    };
    let before = items.len();
    let predicate = parse_predicate(&stage.id, raw)?;
    let filtered = items
        .into_iter()
        .filter(|item| predicate.matches(&item.payload))
        .collect::<Vec<_>>();
    if filtered.is_empty() && before > 0 && !allow_empty {
        return Err(WorkflowError::InvalidFanout(format!(
            "stage '{}' filter '{raw}' matched 0 of {before} fanout item(s)",
            stage.id
        )));
    }
    Ok(filtered)
}

pub(crate) fn stage_filter_matches(stage: &StageSpec, payload: &Value) -> WorkflowResult<bool> {
    let Some(raw) = stage
        .filter
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Ok(true);
    };
    parse_predicate(&stage.id, raw).map(|predicate| predicate.matches(payload))
}

fn parse_predicate(stage_id: &str, raw: &str) -> WorkflowResult<Predicate> {
    let (lhs, op, rhs) = split_operator(raw).ok_or_else(|| invalid_filter(stage_id, raw))?;
    let path = parse_item_path(lhs).ok_or_else(|| invalid_filter(stage_id, raw))?;
    let values = match op {
        Operator::In => parse_list(rhs).ok_or_else(|| invalid_filter(stage_id, raw))?,
        Operator::Eq | Operator::Ne | Operator::Contains => {
            vec![parse_scalar(rhs).ok_or_else(|| invalid_filter(stage_id, raw))?]
        }
    };
    if values.is_empty() {
        return Err(invalid_filter(stage_id, raw));
    }
    Ok(Predicate { path, op, values })
}

fn split_operator(raw: &str) -> Option<(&str, Operator, &str)> {
    if let Some((lhs, rhs)) = raw.split_once(" contains ") {
        return Some((lhs, Operator::Contains, rhs));
    }
    if let Some((lhs, rhs)) = raw.split_once(" in ") {
        return Some((lhs, Operator::In, rhs));
    }
    if let Some((lhs, rhs)) = raw.split_once("==") {
        return Some((lhs, Operator::Eq, rhs));
    }
    raw.split_once("!=")
        .map(|(lhs, rhs)| (lhs, Operator::Ne, rhs))
}

fn parse_item_path(lhs: &str) -> Option<Vec<String>> {
    let path = lhs.trim().strip_prefix("item.")?;
    let parts = path
        .split('.')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!parts.is_empty() && parts.iter().all(|part| valid_path_part(part))).then_some(parts)
}

fn valid_path_part(part: &str) -> bool {
    part.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn parse_scalar(raw: &str) -> Option<String> {
    scalar_to_string(&serde_yaml_ng::from_str::<Value>(raw.trim()).ok()?)
}

fn parse_list(raw: &str) -> Option<Vec<String>> {
    let Value::Array(values) = serde_yaml_ng::from_str::<Value>(raw.trim()).ok()? else {
        return None;
    };
    values.iter().map(scalar_to_string).collect()
}

fn scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

impl Predicate {
    fn matches(&self, item: &Value) -> bool {
        let Some(value) = value_at_path(item, &self.path).or_else(|| field_alias(item, &self.path))
        else {
            return false;
        };
        match self.op {
            Operator::Eq => value_matches_any(value, &self.values),
            Operator::Ne => !value_matches_any(value, &self.values),
            Operator::In => value_contains_any(value, &self.values),
            Operator::Contains => value_contains_any(value, &self.values),
        }
    }
}

fn field_alias<'a>(item: &'a Value, path: &[String]) -> Option<&'a Value> {
    let [field] = path else {
        return None;
    };
    let aliases: &[&str] = match field.as_str() {
        "phase" => &["phases"],
        "phases" => &["phase"],
        "task_id" => &["task_ids", "task", "tasks", "phase", "phases", "id"],
        "task_ids" => &["task_id", "task", "tasks", "phase", "phases", "id"],
        "task" => &["tasks", "task_id", "task_ids", "id"],
        "tasks" => &["task", "task_id", "task_ids", "id"],
        "id" => &["task_id", "task_ids"],
        _ => &[],
    };
    aliases.iter().find_map(|alias| item.get(*alias))
}

fn value_at_path<'a>(item: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut current = item;
    for part in path {
        current = current.get(part)?;
    }
    Some(current)
}

fn value_matches_any(value: &Value, expected: &[String]) -> bool {
    expected.iter().any(|needle| scalar_eq(value, needle))
}

fn value_contains_any(value: &Value, expected: &[String]) -> bool {
    match value {
        Value::Array(values) => values
            .iter()
            .any(|value| value_matches_any(value, expected)),
        Value::String(text) => expected.iter().any(|needle| text.contains(needle)),
        _ => value_matches_any(value, expected),
    }
}

fn scalar_eq(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(value) => value == expected,
        Value::Bool(value) => value.to_string() == expected,
        Value::Number(value) => value.to_string() == expected,
        _ => false,
    }
}

fn invalid_filter(stage_id: &str, raw: &str) -> WorkflowError {
    WorkflowError::InvalidFanout(format!(
        "stage '{stage_id}' has unsupported fanout filter '{raw}'; supported forms are `item.field == 'value'`, `item.field != 'value'`, `item.field in ['a', 'b']`, and `item.array contains 'value'`"
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn matches_scalar_equality() {
        let predicate = parse_predicate("s", "item.wave_id == 'wave1'").unwrap();
        assert!(predicate.matches(&json!({"wave_id": "wave1"})));
        assert!(!predicate.matches(&json!({"wave_id": "wave2"})));
    }

    #[test]
    fn matches_array_contains() {
        let predicate = parse_predicate("s", "item.task_ids contains 'T001'").unwrap();
        assert!(predicate.matches(&json!({"task_ids": ["T001", "T002"]})));
        assert!(!predicate.matches(&json!({"task_ids": ["T010"]})));
    }

    #[test]
    fn phase_in_filter_matches_phase_or_phases() {
        let predicate = parse_predicate("s", "item.phase in ['T001', 'T010']").unwrap();
        assert!(predicate.matches(&json!({"phase": "T001"})));
        assert!(predicate.matches(&json!({"phases": ["T010", "T020"]})));
        assert!(!predicate.matches(&json!({"phases": ["T030"]})));
    }

    #[test]
    fn task_id_filter_matches_common_task_fields() {
        let predicate = parse_predicate("s", "item.task_id in ['T001', 'T010']").unwrap();
        assert!(predicate.matches(&json!({"task_id": "T001"})));
        assert!(predicate.matches(&json!({"task_ids": ["T010", "T020"]})));
        assert!(predicate.matches(&json!({"tasks": ["T010"]})));
        assert!(predicate.matches(&json!({"phase": "T010"})));
        assert!(!predicate.matches(&json!({"task_ids": ["T030"]})));
    }
}
