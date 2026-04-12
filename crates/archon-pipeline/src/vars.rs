//! Variable substitution for `${step.output}` references.
//!
//! Provides extraction, validation, and substitution of inter-step variable
//! references in pipeline step inputs. References follow the pattern
//! `${step_id.output}` or `${step_id.output.path.segments}`.

use std::collections::HashMap;

use regex::Regex;
use serde_json::Value;

use crate::dag::Dag;
use crate::error::PipelineError;
use crate::spec::PipelineSpec;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A parsed variable reference from `${step_id.output.path...}`.
#[derive(Debug, Clone, PartialEq)]
pub struct VarRef {
    /// The step ID being referenced.
    pub step: String,
    /// Dot-separated path segments after `output` (may be empty).
    pub path: Vec<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the shared regex. The pattern captures:
///   group 1 — step id (`[a-zA-Z0-9_-]+`)
///   group 2 — optional dot-separated path after `.output`
fn var_regex() -> Regex {
    Regex::new(r"\$\{([a-zA-Z0-9_-]+)\.output(?:\.([a-zA-Z0-9_.\-]+))?\}").unwrap()
}

/// Navigate into a JSON value using dot-separated path segments.
fn resolve_path<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut current = root;
    for segment in path {
        match current {
            Value::Object(map) => {
                current = map.get(segment.as_str())?;
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Convert a JSON value to its string representation for interpolation.
fn value_to_interpolation_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        // Arrays and objects get their JSON representation.
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Extract all `${step.output...}` variable references from a JSON value.
///
/// Walks the value recursively and returns every reference found in string
/// leaves, including strings nested inside arrays and objects.
pub fn extract_refs(value: &Value) -> Vec<VarRef> {
    let mut refs = Vec::new();
    let re = var_regex();
    collect_refs(value, &re, &mut refs);
    refs
}

fn collect_refs(value: &Value, re: &Regex, out: &mut Vec<VarRef>) {
    match value {
        Value::String(s) => {
            for caps in re.captures_iter(s) {
                let step = caps[1].to_string();
                let path = caps
                    .get(2)
                    .map(|m| m.as_str().split('.').map(|p| p.to_string()).collect())
                    .unwrap_or_default();
                out.push(VarRef { step, path });
            }
        }
        Value::Array(arr) => {
            for v in arr {
                collect_refs(v, re, out);
            }
        }
        Value::Object(map) => {
            for v in map.values() {
                collect_refs(v, re, out);
            }
        }
        _ => {}
    }
}

/// Validate that all variable references in the pipeline spec are legal.
///
/// For every step, each `${step_id.output...}` reference must satisfy:
/// 1. The referenced step exists in the spec.
/// 2. The referenced step is a transitive predecessor (its DAG level is strictly
///    less than the current step's level).
///
/// # Errors
/// * [`PipelineError::MissingStep`] — referenced step does not exist.
/// * [`PipelineError::ValidationError`] — referenced step is not a dependency.
pub fn validate_refs(spec: &PipelineSpec, dag: &Dag) -> Result<(), PipelineError> {
    let known_ids: HashMap<&str, ()> = spec.steps.iter().map(|s| (s.id.as_str(), ())).collect();

    for step in &spec.steps {
        let refs = extract_refs(&step.input);
        let current_level = dag
            .step_index
            .get(&step.id)
            .copied()
            .unwrap_or(usize::MAX);

        for var_ref in &refs {
            // 1. Referenced step must exist.
            if !known_ids.contains_key(var_ref.step.as_str()) {
                return Err(PipelineError::MissingStep(var_ref.step.clone()));
            }

            // 2. Referenced step must be at a strictly lower level.
            let ref_level = dag
                .step_index
                .get(&var_ref.step)
                .copied()
                .unwrap_or(usize::MAX);

            if ref_level >= current_level {
                return Err(PipelineError::ValidationError(format!(
                    "step '{}' references '{}' which is not a dependency",
                    step.id, var_ref.step
                )));
            }
        }
    }

    Ok(())
}

/// Substitute `${step.output...}` references in a JSON value using resolved
/// step outputs.
///
/// Substitution rules:
/// - If an entire string is exactly one reference, the resolved value replaces
///   it preserving its JSON type (number, object, etc.).
/// - If a string contains references mixed with other text, each reference is
///   replaced with its string representation (interpolation).
/// - If the resolved value is an object with key `__archon_file_ref__`, it is
///   returned as-is (file-ref preservation).
///
/// # Errors
/// * [`PipelineError::ValidationError`] — a referenced step has no output yet,
///   or a path segment cannot be resolved.
pub fn substitute(
    input: &Value,
    outputs: &HashMap<String, Value>,
) -> Result<Value, PipelineError> {
    let re = var_regex();
    substitute_value(input, outputs, &re)
}

fn substitute_value(
    value: &Value,
    outputs: &HashMap<String, Value>,
    re: &Regex,
) -> Result<Value, PipelineError> {
    match value {
        Value::String(s) => substitute_string(s, outputs, re),
        Value::Array(arr) => {
            let items: Result<Vec<Value>, PipelineError> = arr
                .iter()
                .map(|v| substitute_value(v, outputs, re))
                .collect();
            Ok(Value::Array(items?))
        }
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(k.clone(), substitute_value(v, outputs, re)?);
            }
            Ok(Value::Object(new_map))
        }
        // Numbers, booleans, null — pass through.
        other => Ok(other.clone()),
    }
}

fn substitute_string(
    s: &str,
    outputs: &HashMap<String, Value>,
    re: &Regex,
) -> Result<Value, PipelineError> {
    // Check if the ENTIRE string is exactly one reference.
    let exact_re =
        Regex::new(r"^\$\{([a-zA-Z0-9_-]+)\.output(?:\.([a-zA-Z0-9_.\-]+))?\}$").unwrap();

    if let Some(caps) = exact_re.captures(s) {
        // Ensure there is exactly one match in the original string as well.
        if re.find_iter(s).count() == 1 {
            let step_id = &caps[1];
            let path: Vec<String> = caps
                .get(2)
                .map(|m| m.as_str().split('.').map(|p| p.to_string()).collect())
                .unwrap_or_default();

            let resolved = resolve_ref(step_id, &path, outputs)?;

            // File-ref preservation: return as-is.
            if let Value::Object(map) = &resolved {
                if map.contains_key("__archon_file_ref__") {
                    return Ok(resolved);
                }
            }

            return Ok(resolved);
        }
    }

    // Interpolation mode: replace each reference with its string form.
    if !re.is_match(s) {
        return Ok(Value::String(s.to_string()));
    }

    let mut result = String::new();
    let mut last_end = 0;

    for caps in re.captures_iter(s) {
        let whole_match = caps.get(0).unwrap();
        result.push_str(&s[last_end..whole_match.start()]);

        let step_id = &caps[1];
        let path: Vec<String> = caps
            .get(2)
            .map(|m| m.as_str().split('.').map(|p| p.to_string()).collect())
            .unwrap_or_default();

        let resolved = resolve_ref(step_id, &path, outputs)?;
        result.push_str(&value_to_interpolation_string(&resolved));

        last_end = whole_match.end();
    }
    result.push_str(&s[last_end..]);

    Ok(Value::String(result))
}

fn resolve_ref(
    step_id: &str,
    path: &[String],
    outputs: &HashMap<String, Value>,
) -> Result<Value, PipelineError> {
    // When a step has no output (e.g. it was skipped due to a condition),
    // resolve to JSON null rather than erroring.  This allows downstream
    // steps to reference a potentially-skipped step without hard failure.
    let output = match outputs.get(step_id) {
        Some(o) => o,
        None => return Ok(Value::Null),
    };

    if path.is_empty() {
        return Ok(output.clone());
    }

    resolve_path(output, path).cloned().ok_or_else(|| {
        PipelineError::ValidationError(format!(
            "cannot resolve path '{}' in output of step '{}'",
            path.join("."),
            step_id
        ))
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dag::DagBuilder;
    use crate::spec::{BackoffKind, OnFailurePolicy, PipelineSpec, RetrySpec, StepSpec};
    use serde_json::json;

    /// Helper to build a [`PipelineSpec`] from `(id, agent, depends_on, input)` tuples.
    fn make_spec_with_input(
        steps: Vec<(&str, &str, Vec<&str>, Value)>,
    ) -> PipelineSpec {
        PipelineSpec {
            name: "test".to_string(),
            version: "1.0".to_string(),
            global_timeout_secs: 3600,
            max_parallelism: 5,
            steps: steps
                .into_iter()
                .map(|(id, agent, deps, input)| StepSpec {
                    id: id.to_string(),
                    agent: agent.to_string(),
                    input,
                    depends_on: deps.into_iter().map(|d| d.to_string()).collect(),
                    retry: RetrySpec {
                        max_attempts: 1,
                        backoff: BackoffKind::Exponential,
                        base_delay_ms: 1000,
                    },
                    timeout_secs: 1800,
                    condition: None,
                    on_failure: OnFailurePolicy::Rollback,
                })
                .collect(),
        }
    }

    // 1. extract_refs_simple
    #[test]
    fn extract_refs_simple() {
        let value = json!("${a.output}");
        let refs = extract_refs(&value);
        assert_eq!(refs.len(), 1);
        assert_eq!(
            refs[0],
            VarRef {
                step: "a".to_string(),
                path: vec![],
            }
        );
    }

    // 2. extract_refs_nested_path
    #[test]
    fn extract_refs_nested_path() {
        let value = json!("${a.output.data.items}");
        let refs = extract_refs(&value);
        assert_eq!(refs.len(), 1);
        assert_eq!(
            refs[0],
            VarRef {
                step: "a".to_string(),
                path: vec!["data".to_string(), "items".to_string()],
            }
        );
    }

    // 3. extract_refs_in_nested_json
    #[test]
    fn extract_refs_in_nested_json() {
        let value = json!({
            "top": "${x.output}",
            "nested": {
                "inner": "${y.output.foo}"
            },
            "list": ["${z.output}", "plain"]
        });
        let refs = extract_refs(&value);
        assert_eq!(refs.len(), 3);

        let step_ids: Vec<&str> = refs.iter().map(|r| r.step.as_str()).collect();
        assert!(step_ids.contains(&"x"));
        assert!(step_ids.contains(&"y"));
        assert!(step_ids.contains(&"z"));
    }

    // 4. substitute_exact_preserves_type
    #[test]
    fn substitute_exact_preserves_type() {
        let input = json!("${a.output}");
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), json!(42));

        let result = substitute(&input, &outputs).unwrap();
        assert_eq!(result, json!(42));
        assert!(result.is_number(), "result should be a number, not a string");
    }

    // 5. substitute_interpolation
    #[test]
    fn substitute_interpolation() {
        let input = json!("value=${a.output}");
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), json!(42));

        let result = substitute(&input, &outputs).unwrap();
        assert_eq!(result, json!("value=42"));
    }

    // 6. validate_rejects_unknown_step
    #[test]
    fn validate_rejects_unknown_step() {
        let spec = make_spec_with_input(vec![(
            "A",
            "agent",
            vec![],
            json!({"ref": "${ghost.output}"}),
        )]);
        let dag = DagBuilder::build(&spec).unwrap();

        let err = validate_refs(&spec, &dag).expect_err("should reject unknown step");
        match err {
            PipelineError::MissingStep(id) => assert_eq!(id, "ghost"),
            other => panic!("expected MissingStep, got: {other:?}"),
        }
    }

    // 7. validate_rejects_non_dependency
    #[test]
    fn validate_rejects_non_dependency() {
        // B references A's output but does NOT depend_on A.
        // Both end up at level 0, so A is NOT a predecessor of B.
        let spec = make_spec_with_input(vec![
            ("A", "agent", vec![], json!({})),
            ("B", "agent", vec![], json!({"ref": "${A.output}"})),
        ]);
        let dag = DagBuilder::build(&spec).unwrap();

        let err = validate_refs(&spec, &dag).expect_err("should reject non-dependency ref");
        match err {
            PipelineError::ValidationError(msg) => {
                assert!(
                    msg.contains("step 'B' references 'A' which is not a dependency"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected ValidationError, got: {other:?}"),
        }
    }

    // 8. substitute_preserves_file_ref_wrapper
    #[test]
    fn substitute_preserves_file_ref_wrapper() {
        let input = json!("${a.output}");
        let file_ref = json!({"__archon_file_ref__": "/tmp/x"});
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), file_ref.clone());

        let result = substitute(&input, &outputs).unwrap();
        assert_eq!(result, file_ref);
        assert!(
            result.as_object().unwrap().contains_key("__archon_file_ref__"),
            "file ref wrapper should be preserved"
        );
    }

    // 9. substitute_missing_output_resolves_to_null
    //
    // When a step has no output (e.g. it was skipped by a condition), the
    // reference resolves to JSON null so downstream steps can continue.
    #[test]
    fn substitute_missing_output_resolves_to_null() {
        let input = json!("${missing.output}");
        let outputs = HashMap::new();

        let result = substitute(&input, &outputs).expect("should resolve to null");
        assert_eq!(result, json!(null));
    }
}
