use super::declarative_floor::field;
use super::declarative_floor_values::canonical;
use crate::task_universe::WorkflowV2DeliverableContract;
use serde_json::Value;
use std::collections::BTreeSet;

const STEP_VARIETY_MIN_ROWS: usize = 20;
const STEP_VARIETY_MIN_PERCENT: u32 = 20;

type Tokens = Vec<Vec<String>>;
type Steps = Vec<(String, Vec<String>)>;
#[derive(Default)]
pub(super) struct Series {
    values: Vec<(String, Tokens)>,
    steps: Vec<(String, Steps)>,
}

impl Series {
    pub(super) fn cell(
        &mut self,
        contract: &WorkflowV2DeliverableContract,
        rows: &[Value],
        label: &str,
        findings: &mut Vec<String>,
    ) {
        if rows.is_empty() || contract.series_value_fields.is_empty() {
            return;
        }
        let objects: Vec<_> = rows.iter().filter(|row| row.is_object()).collect();
        let tokens = objects
            .iter()
            .map(|row| {
                contract
                    .series_value_fields
                    .iter()
                    .map(|path| canonical(field(row, Some(path))))
                    .collect()
            })
            .collect();
        self.values.push((label.to_owned(), tokens));
        let mut field_steps = Vec::new();
        for path in &contract.series_value_fields {
            let values: Option<Vec<_>> = objects
                .iter()
                .map(|row| number(field(row, Some(path))?))
                .collect();
            let Some(values) = values.filter(|values| values.len() >= 3) else {
                continue;
            };
            let steps: Vec<_> = values
                .windows(2)
                .map(|pair| {
                    let difference = pair[1] - pair[0];
                    if !difference.is_finite() {
                        findings.push(format!(
                            "{label} payload field has a non-finite first difference: {path}"
                        ));
                    }
                    let step = format!("{difference:.12}");
                    if step == "-0.000000000000" {
                        "0.000000000000".into()
                    } else {
                        step
                    }
                })
                .collect();
            let distinct = steps.iter().collect::<BTreeSet<_>>().len();
            if distinct == 1 {
                findings.push(format!(
                    "{label} payload field has a constant first difference: {path}"
                ));
            } else if steps.len()
                >= contract
                    .step_variety_min_rows
                    .filter(|n| *n > 0)
                    .unwrap_or(STEP_VARIETY_MIN_ROWS)
            {
                let ratio = distinct as f64 / steps.len() as f64;
                let minimum = f64::from(
                    contract
                        .step_variety_min_percent
                        .filter(|n| *n > 0)
                        .unwrap_or(STEP_VARIETY_MIN_PERCENT),
                ) / 100.0;
                if ratio < minimum {
                    findings.push(format!("{label} payload field has only {distinct} distinct first differences across {} steps (ratio {ratio:.3} < {minimum}): {path} is synthetic, not observed", steps.len()));
                }
            }
            field_steps.push((path.clone(), steps));
        }
        if !field_steps.is_empty() {
            self.steps.push((label.to_owned(), field_steps));
        }
    }

    pub(super) fn finish(
        self,
        contract: &WorkflowV2DeliverableContract,
        findings: &mut Vec<String>,
    ) {
        for (index, (label, tokens)) in self.values.iter().enumerate() {
            for (other, other_tokens) in &self.values[..index] {
                if tokens == other_tokens {
                    findings.push(format!(
                        "distinct cells {other} and {label} have identical payload series"
                    ));
                    continue;
                }
                let n = contract.series_overlap_min_rows;
                if n > 0 && tokens.len() >= n && other_tokens.len() >= n {
                    let windows: BTreeSet<_> = tokens.windows(n).collect();
                    if other_tokens
                        .windows(n)
                        .any(|window| windows.contains(window))
                    {
                        findings.push(format!("distinct cells {other} and {label} share a declared payload-series window"));
                    }
                }
            }
        }
        for (index, (label, steps)) in self.steps.iter().enumerate() {
            for (other, other_steps) in &self.steps[..index] {
                if steps == other_steps {
                    findings.push(format!("distinct cells {other} and {label} have identical declared field-step series"));
                }
            }
        }
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(value) => Some(u8::from(*value) as f64),
        Value::String(value) => value.trim().parse().ok(),
        Value::Number(number) => number.to_string().parse().ok(),
        _ => None,
    }
}
