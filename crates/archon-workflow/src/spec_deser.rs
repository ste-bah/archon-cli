use std::collections::BTreeMap;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::spec::ProviderTier;

pub fn deserialize_provider_tiers<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<ProviderTier, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(input) = Option::<Value>::deserialize(deserializer)? else {
        return Ok(BTreeMap::new());
    };
    provider_tiers_from_value(input).map_err(D::Error::custom)
}

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

pub fn deserialize_quality_gates<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, Value>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(input) = Option::<Value>::deserialize(deserializer)? else {
        return Ok(BTreeMap::new());
    };
    Ok(normalize_quality_gates(input))
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

fn provider_tiers_from_value(input: Value) -> Result<BTreeMap<ProviderTier, String>, String> {
    let mut tiers = BTreeMap::new();
    match input {
        Value::Null => {}
        Value::Object(values) => {
            for (key, value) in values {
                // Skip unrecognized tier keys rather than aborting the whole
                // plan. Generated plans occasionally emit advisory keys (e.g.
                // `hint`) that are not real tiers; the provider_tiers map is
                // advisory and not consulted at runtime, so dropping unknown
                // keys is safe and keeps recoverable input usable.
                let Ok(tier) = parse_provider_tier(&key) else {
                    continue;
                };
                tiers.insert(tier, normalize_provider_tier(&value));
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_provider_tier_entry(value, &mut tiers)?;
            }
        }
        other => {
            return Err(format!(
                "provider_tiers must be a map or sequence, got {other:?}"
            ));
        }
    }
    Ok(tiers)
}

fn collect_provider_tier_entry(
    value: Value,
    tiers: &mut BTreeMap<ProviderTier, String>,
) -> Result<(), String> {
    match value {
        Value::String(name) => {
            if let Ok(tier) = parse_provider_tier(&name) {
                tiers.insert(tier, "auto".into());
            }
        }
        Value::Object(values) => {
            if let Some(name) = named_tier(&values) {
                if let Ok(tier) = parse_provider_tier(name) {
                    tiers.insert(tier, normalize_provider_tier_map(&values));
                }
                return Ok(());
            }
            for (key, value) in values {
                let Ok(tier) = parse_provider_tier(&key) else {
                    continue;
                };
                tiers.insert(tier, normalize_provider_tier(&value));
            }
        }
        other => {
            return Err(format!(
                "provider_tiers sequence entry is invalid: {other:?}"
            ));
        }
    }
    Ok(())
}

fn named_tier(values: &serde_json::Map<String, Value>) -> Option<&str> {
    ["tier", "name", "id"]
        .into_iter()
        .find_map(|key| values.get(key).and_then(Value::as_str))
}

fn parse_provider_tier(value: &str) -> Result<ProviderTier, String> {
    serde_json::from_value(Value::String(value.to_string()))
        .map_err(|_| format!("unknown provider tier '{value}'"))
}

fn normalize_provider_tier(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Object(values) => normalize_provider_tier_map(values),
        _ => "auto".into(),
    }
}

fn normalize_provider_tier_map(values: &serde_json::Map<String, Value>) -> String {
    for key in ["provider", "model", "family"] {
        let Some(value) = values.get(key).and_then(Value::as_str) else {
            continue;
        };
        if has_text(value) && !is_neutral_tier_hint(value) {
            return format!("hardcoded:{key}:{value}");
        }
    }
    values
        .get("value")
        .or_else(|| values.get("alias"))
        .and_then(Value::as_str)
        .filter(|value| is_neutral_tier_hint(value))
        .unwrap_or("auto")
        .to_string()
}

fn normalize_quality_gates(input: Value) -> BTreeMap<String, Value> {
    match input {
        Value::Object(values) => values.into_iter().collect(),
        Value::Array(values) => values
            .into_iter()
            .enumerate()
            .map(|(idx, value)| (quality_gate_key(idx, &value), value))
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn quality_gate_key(idx: usize, value: &Value) -> String {
    value
        .as_object()
        .and_then(|object| {
            ["id", "name", "stage", "gate"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(Value::as_str))
        })
        .filter(|value| has_text(value))
        .map(str::to_string)
        .unwrap_or_else(|| format!("gate-{}", idx + 1))
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

fn has_text(value: &str) -> bool {
    !value.trim().is_empty()
}

fn is_neutral_tier_hint(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "auto" | "default" | "inherit" | "active"
    )
}
