use super::declarative_floor::{field, normalized};
use super::declarative_floor_consistency::internal_consistency;
use super::declarative_floor_facts::{DeclarativeFloorFacts, FileState};
use super::declarative_floor_values::{
    canonical, identity_equal, integer, json_file, scalar_text, timestamp,
};
use crate::task_universe::WorkflowV2DeliverableContract;
use chrono::Datelike;
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn evaluate_records(
    contract: &WorkflowV2DeliverableContract,
    facts: &DeclarativeFloorFacts,
    cell: &Value,
    record: &Value,
    label: &str,
    findings: &mut Vec<String>,
) -> Vec<Value> {
    if let Some(path) = &contract.validation_path_field {
        if let Some(reference) = reference(record, path) {
            if let Some(validation) = json_file(
                facts.files.get(reference),
                &format!("{label} validation"),
                findings,
            ) {
                internal_consistency(
                    contract,
                    &validation,
                    &format!("{label} validation"),
                    findings,
                );
                let overall = field(&validation, contract.validation_status_field.as_deref())
                    .map(normalized)
                    .unwrap_or_default();
                if validation.is_object()
                    && !contract.validation_passed_values.is_empty()
                    && !contract
                        .validation_passed_values
                        .iter()
                        .any(|v| v.trim().to_lowercase() == overall)
                {
                    findings.push(format!("{label} validation overall status is not passed"));
                }
            }
        } else {
            findings.push(format!("{label} registry has no validation path"));
        }
    }
    let rows = payload(contract, facts, record, label, findings);
    for (index, row) in rows.iter().enumerate() {
        if !row.is_object() {
            findings.push(format!("{label} payload row {index} is not an object"));
            continue;
        }
        for path in &contract.required_fields {
            if field(row, Some(path)).is_none_or(Value::is_null) {
                findings.push(format!("{label} payload row {index} lacks {path}"));
            }
        }
    }
    for path in &contract.non_constant_fields {
        let values: BTreeSet<_> = rows
            .iter()
            .filter(|row| row.is_object())
            .map(|row| canonical(field(row, Some(path))))
            .collect();
        if values.len() <= 1 {
            findings.push(format!(
                "{label} payload field is constant or absent: {path}"
            ));
        }
    }
    temporal(contract, facts, &rows, label, findings);
    if let (Some(path), Some(count)) = (
        &contract.request_path_field,
        &contract.requested_count_field,
    ) {
        let request = reference(record, path).and_then(|reference| {
            json_file(
                facts.files.get(reference),
                &format!("{label} request"),
                findings,
            )
        });
        let requested = request
            .as_ref()
            .and_then(|request| field(request, Some(count)));
        match integer(requested) {
            None => findings.push(format!("{label} requested count is not numeric")),
            Some(value) if value <= 0 => findings.push(format!(
                "{label} requested count is not positive: {}",
                requested.map(scalar_text).unwrap_or_default()
            )),
            Some(value) if (rows.len() as i128) < value => findings.push(format!(
                "{label} delivered count {} is below requested count {}",
                rows.len(),
                requested.map(scalar_text).unwrap_or_default()
            )),
            _ => {}
        }
    }
    if let Some(path) = &contract.response_path_field {
        let response = match reference(record, path) {
            Some(reference) => json_file(
                facts.files.get(reference),
                &format!("{label} response"),
                findings,
            ),
            None => {
                findings.push(format!("{label} registry has no response path"));
                None
            }
        };
        if let Some(response) = response.filter(Value::is_object) {
            for (cell_path, response_path) in &contract.response_identity_fields {
                if !identity_equal(
                    field(cell, Some(cell_path)),
                    field(&response, Some(response_path)),
                ) {
                    findings.push(format!(
                        "{label} response identity mismatch: {cell_path}!={response_path}"
                    ));
                }
            }
        }
    }
    rows
}

pub(super) fn reference<'a>(record: &'a Value, path: &str) -> Option<&'a str> {
    field(record, Some(path))?
        .as_str()
        .filter(|value| !value.is_empty())
}

fn payload(
    contract: &WorkflowV2DeliverableContract,
    facts: &DeclarativeFloorFacts,
    record: &Value,
    label: &str,
    findings: &mut Vec<String>,
) -> Vec<Value> {
    let Some(path) = &contract.payload_path_field else {
        return Vec::new();
    };
    let format = contract.payload_format.as_deref().unwrap_or("json");
    let fact = reference(record, path).and_then(|reference| facts.files.get(reference));
    let rows = fact
        .and_then(|fact| match &fact.state {
            FileState::Bytes(bytes) => {
                let parsed = if format == "jsonl" {
                    std::str::from_utf8(bytes)
                        .map_err(|e| e.to_string())
                        .and_then(|text| {
                            text.split([
                                '\n', '\r', '\u{000b}', '\u{000c}', '\u{001c}', '\u{001d}',
                                '\u{001e}', '\u{0085}', '\u{2028}', '\u{2029}',
                            ])
                            .filter(|line| !line.trim().is_empty())
                            .map(|line| {
                                super::declarative_floor_json::parse(line.as_bytes())
                                    .map_err(|e| e.to_string())
                            })
                            .collect::<Result<Vec<Value>, _>>()
                        })
                } else {
                    super::declarative_floor_json::parse(bytes)
                        .map(|value| match value {
                            Value::Array(rows) => rows,
                            value => vec![value],
                        })
                        .map_err(|e| e.to_string())
                };
                match parsed {
                    Ok(rows) => Some(rows),
                    Err(error) => {
                        findings.push(format!(
                            "payload cannot be parsed as {format}: {}: {error}",
                            fact.shown
                        ));
                        None
                    }
                }
            }
            FileState::Unreadable(error) => {
                findings.push(format!("payload cannot be read: {}: {error}", fact.shown));
                None
            }
            _ => {
                findings.push(format!("payload missing or empty: {}", fact.shown));
                None
            }
        })
        .unwrap_or_default();
    if rows.is_empty() {
        findings.push(format!("{label} payload has no records"));
    }
    rows
}

fn temporal(
    contract: &WorkflowV2DeliverableContract,
    facts: &DeclarativeFloorFacts,
    rows: &[Value],
    label: &str,
    findings: &mut Vec<String>,
) {
    let Some(path) = &contract.observed_time_field else {
        return;
    };
    let now = facts
        .verified_at
        .as_deref()
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok());
    for (index, row) in rows.iter().enumerate().filter(|(_, row)| row.is_object()) {
        let Some(raw) = field(row, Some(path)).filter(|v| !v.is_null() && v.as_str() != Some(""))
        else {
            continue;
        };
        match timestamp(raw) {
            None => findings.push(format!(
                "{label} payload row {index} {path} is not a parseable timestamp: {}",
                scalar_text(raw)
            )),
            Some(parsed) => match now {
                Some(now) if parsed > now => {
                    findings.push(format!("{label} payload row {index} {path} is in the future ({}); an observed series cannot contain future records", scalar_text(raw)));
                    break;
                }
                None => {
                    findings.push("verification instant is missing or invalid".into());
                    break;
                }
                _ => {}
            },
        }
    }
    let dates: BTreeSet<_> = contract
        .closed_dates
        .iter()
        .map(|date| date.trim().chars().take(10).collect::<String>())
        .collect();
    let mut hits = Vec::new();
    for (index, row) in rows.iter().enumerate().filter(|(_, row)| row.is_object()) {
        let Some(parsed) = field(row, Some(path)).and_then(timestamp) else {
            continue;
        };
        let day = parsed.weekday().num_days_from_monday() as u8;
        let date = parsed.date_naive().to_string();
        if contract
            .closed_weekdays
            .as_ref()
            .is_some_and(|days| days.contains(&day))
        {
            hits.push(format!("row {index} falls on weekday {day}"));
        } else if dates.contains(&date) {
            hits.push(format!("row {index} falls on closed date {date}"));
        }
    }
    if !hits.is_empty() {
        findings.push(format!("{label} payload has {} record(s) dated when the source was closed ({}); an observed series cannot contain them", hits.len(), hits.iter().take(3).cloned().collect::<Vec<_>>().join("; ")));
    }
}
