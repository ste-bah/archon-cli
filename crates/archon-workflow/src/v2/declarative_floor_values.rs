//! Shared value conversions for declared predicates.
use super::declarative_floor_facts::{FileFact, FileState};
use crate::task_universe::WorkflowV2DeliverableContract;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use serde_json::Value;

pub fn artifact_format(contract: &WorkflowV2DeliverableContract, path: &str) -> String {
    contract.normalized_artifact_format().unwrap_or_else(|| {
        let path = path.to_ascii_lowercase();
        if path.ends_with(".json") || path.ends_with(".jsonl") {
            "json"
        } else {
            "text"
        }
        .into()
    })
}

pub(super) fn json_file(
    fact: Option<&FileFact>,
    label: &str,
    findings: &mut Vec<String>,
) -> Option<Value> {
    let Some(fact) = fact else {
        findings.push(format!("{label} evidence facts are missing"));
        return None;
    };
    match &fact.state {
        FileState::Bytes(bytes) => match super::declarative_floor_json::parse(bytes) {
            Ok(Value::Null) => {
                findings.push(format!("{label} contains a null value: {}", fact.shown));
                None
            }
            Ok(value) => Some(value),
            Err(error) => {
                findings.push(format!(
                    "{label} is not valid JSON: {}: {error}",
                    fact.shown
                ));
                None
            }
        },
        FileState::Unreadable(error) => {
            findings.push(format!("{label} cannot be read: {}: {error}", fact.shown));
            None
        }
        _ => {
            findings.push(format!("{label} missing or empty: {}", fact.shown));
            None
        }
    }
}

pub(super) fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(mut seconds) = value.as_f64() {
        if seconds > 1e11 {
            seconds /= 1000.0;
        }
        if !seconds.is_finite() || seconds.abs() > i64::MAX as f64 {
            return None;
        }
        let whole = seconds.floor();
        return DateTime::from_timestamp(whole as i64, ((seconds - whole) * 1e9) as u32);
    }
    let text = value.as_str()?.trim();
    if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
        return Some(instant.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(instant) = NaiveDateTime::parse_from_str(text, format) {
            return Some(instant.and_utc());
        }
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)
        .map(|date| date.and_utc())
}

pub(super) fn integer(value: Option<&Value>) -> Option<i128> {
    match value? {
        Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))
            .or_else(|| {
                let f = n.as_f64()?;
                (f.is_finite() && f.fract() == 0.0 && f.abs() < i128::MAX as f64)
                    .then_some(f as i128)
            }),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub(super) fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        value => value.to_string(),
    }
}

pub(super) fn canonical(value: Option<&Value>) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let ordered: std::collections::BTreeMap<_, _> =
                    map.iter().map(|(k, v)| (k.clone(), sorted(v))).collect();
                Value::Object(ordered.into_iter().collect())
            }
            Value::Array(values) => Value::Array(values.iter().map(sorted).collect()),
            value => value.clone(),
        }
    }
    sorted(value.unwrap_or(&Value::Null)).to_string()
}

pub(super) fn identity_key(value: &Value) -> String {
    match value {
        Value::Number(number)
            if number.is_f64() && number.as_f64().is_some_and(|value| value.fract() == 0.0) =>
        {
            format!("{:.0}", number.as_f64().unwrap() + 0.0)
        }
        value => value.to_string(),
    }
}

pub(super) fn identity_text(identity: &[String]) -> String {
    format!("[{}]", identity.join(", "))
}

/// Identity equality preserves types and compares numbers numerically.
pub(super) fn identity_equal(left: Option<&Value>, right: Option<&Value>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) if !left.is_null() && !right.is_null() => {
            identity_key(left) == identity_key(right)
        }
        _ => false,
    }
}
