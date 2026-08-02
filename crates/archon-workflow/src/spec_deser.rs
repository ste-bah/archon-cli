use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

pub fn deserialize_learning_hooks<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(input) = Option::<Value>::deserialize(deserializer)? else {
        return Ok(Vec::new());
    };
    let mut hooks = Vec::new();
    collect_learning_hooks(&input, &mut hooks);
    hooks.sort();
    hooks.dedup();
    Ok(hooks)
}

pub fn deserialize_permissions<'de, D>(deserializer: D) -> Result<BTreeMap<String, Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(input) = Option::<Value>::deserialize(deserializer)? else {
        return Ok(BTreeMap::new());
    };
    Ok(match input {
        Value::Object(values) => values.into_iter().collect(),
        _ => BTreeMap::new(),
    })
}

fn collect_learning_hooks(value: &Value, hooks: &mut Vec<String>) {
    match value {
        Value::String(value) => hooks.extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        ),
        Value::Array(values) => {
            for value in values {
                collect_learning_hooks(value, hooks);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if learning_hook_enabled(value) {
                    hooks.push(key.clone());
                }
            }
        }
        _ => {}
    }
}

fn learning_hook_enabled(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::Null => false,
        Value::String(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "" | "false" | "off" | "no"
        ),
        Value::Object(values) => values
            .get("enabled")
            .map(learning_hook_enabled)
            .unwrap_or(true),
        _ => true,
    }
}
