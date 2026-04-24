//! CEL-based conditional execution for pipeline steps.
//!
//! Evaluates condition expressions attached to [`StepSpec`] entries before the
//! step's retry loop runs.  When the expression yields `false` the step is
//! marked [`StepRunState::Skipped`] and no task is submitted.
//!
//! Expressions may reference upstream step outputs using the pipeline variable
//! syntax `${step_id.output.path}`.  The preprocessor rewrites these to
//! `steps.step_id.output.path` so that the CEL engine can resolve them against
//! a `steps` map variable in the execution context.

use std::collections::HashMap;

use regex::Regex;
use serde_json::Value as JsonValue;

use crate::error::PipelineError;

/// Evaluates CEL conditions for step execution decisions.
pub struct ConditionEvaluator;

impl ConditionEvaluator {
    /// Evaluate a condition expression against completed step outputs.
    ///
    /// The expression uses `${step_id.output.path}` syntax.  Preprocessing
    /// strips `${}` and rewrites variable references to `steps.step_id.output.path`
    /// for CEL resolution.
    ///
    /// Returns `true` if the step should execute, `false` to skip.
    pub fn evaluate(
        expr: &str,
        outputs: &HashMap<String, JsonValue>,
    ) -> Result<bool, PipelineError> {
        // 1. Preprocess: strip outer ${...} and rewrite variable refs.
        let processed = Self::preprocess(expr)?;

        // 2. Build CEL context with `steps` map.
        let context = Self::build_context(outputs)?;

        // 3. Compile and execute.
        let program = cel_interpreter::Program::compile(&processed).map_err(|e| {
            PipelineError::ValidationError(format!("invalid condition '{}': {}", expr, e))
        })?;

        let result = program.execute(&context).map_err(|e| {
            PipelineError::ValidationError(format!(
                "condition evaluation failed for '{}': {}",
                expr, e
            ))
        })?;

        // 4. Coerce to bool.
        match result {
            cel_interpreter::Value::Bool(b) => Ok(b),
            other => Err(PipelineError::ValidationError(format!(
                "condition '{}' did not yield bool, got: {:?}",
                expr, other
            ))),
        }
    }

    /// Preprocess the user expression:
    /// - Strip outer `${...}` wrapper if the entire expression is wrapped
    /// - Rewrite `step_id.output.path` references to `steps.step_id.output.path`
    fn preprocess(expr: &str) -> Result<String, PipelineError> {
        let trimmed = expr.trim();

        // Strip ${...} wrapper if present.
        let inner = if trimmed.starts_with("${") && trimmed.ends_with('}') {
            &trimmed[2..trimmed.len() - 1]
        } else {
            trimmed
        };

        // Rewrite variable references:
        // `step_id.output.path` -> `steps.step_id.output.path`
        //
        // We capture an optional preceding character (non-word boundary) and the
        // identifier.  If the preceding char is a dot or word char, this is
        // already qualified (e.g. `steps.a.output`) and we leave it alone.
        //
        // The Rust regex crate does not support look-behind, so we match an
        // optional preceding character and conditionally rewrite.
        let re = Regex::new(r"(^|[^.\w])([a-zA-Z_][a-zA-Z0-9_-]*)\.output\b")
            .map_err(|e| PipelineError::ValidationError(format!("regex error: {}", e)))?;

        let result = re.replace_all(inner, "${1}steps.${2}.output").to_string();
        Ok(result)
    }

    /// Build a CEL execution context from step outputs.
    ///
    /// Creates a `steps` variable that is a map of `step_id -> { output: <value> }`.
    fn build_context(
        outputs: &HashMap<String, JsonValue>,
    ) -> Result<cel_interpreter::Context<'static>, PipelineError> {
        let mut context = cel_interpreter::Context::default();

        // Build a nested map: steps -> { step_id -> { output -> value } }
        let mut steps_map: HashMap<String, JsonValue> = HashMap::new();
        for (step_id, output) in outputs {
            let wrapper = serde_json::json!({ "output": output });
            steps_map.insert(step_id.clone(), wrapper);
        }

        let steps_json = serde_json::to_value(&steps_map).map_err(|e| {
            PipelineError::ValidationError(format!("failed to serialize step outputs: {}", e))
        })?;

        // Use `add_variable` with serde serialization to convert
        // serde_json::Value -> cel_interpreter::Value automatically.
        context.add_variable("steps", steps_json).map_err(|e| {
            PipelineError::ValidationError(format!("failed to build CEL context: {}", e))
        })?;

        Ok(context)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preprocess_strips_dollar_brace_wrapper() {
        let result = ConditionEvaluator::preprocess("${a.output.flag}").unwrap();
        assert_eq!(result, "steps.a.output.flag");
    }

    #[test]
    fn preprocess_rewrites_bare_reference() {
        let result = ConditionEvaluator::preprocess("a.output.flag == true").unwrap();
        assert_eq!(result, "steps.a.output.flag == true");
    }

    #[test]
    fn preprocess_handles_multiple_references() {
        let result =
            ConditionEvaluator::preprocess("a.output.x > 0 && b.output.y == true").unwrap();
        assert_eq!(result, "steps.a.output.x > 0 && steps.b.output.y == true");
    }

    #[test]
    fn preprocess_does_not_double_prefix() {
        // If already prefixed with steps., should not double-prefix.
        let result = ConditionEvaluator::preprocess("steps.a.output.flag").unwrap();
        // The `steps` part is preceded by nothing (start of string), but `a` is
        // preceded by `.` so it should NOT match the rewrite regex.
        assert_eq!(result, "steps.a.output.flag");
    }

    #[test]
    fn evaluate_bool_true() {
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), json!({"should_run": true}));

        let result = ConditionEvaluator::evaluate("${a.output.should_run}", &outputs).unwrap();
        assert!(result);
    }

    #[test]
    fn evaluate_bool_false() {
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), json!({"should_run": false}));

        let result = ConditionEvaluator::evaluate("${a.output.should_run}", &outputs).unwrap();
        assert!(!result);
    }

    #[test]
    fn evaluate_comparison() {
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), json!({"count": 5}));

        let result = ConditionEvaluator::evaluate("a.output.count > 3", &outputs).unwrap();
        assert!(result);

        let result = ConditionEvaluator::evaluate("a.output.count > 10", &outputs).unwrap();
        assert!(!result);
    }

    #[test]
    fn evaluate_string_equality() {
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), json!({"status": "ok"}));

        let result = ConditionEvaluator::evaluate("a.output.status == \"ok\"", &outputs).unwrap();
        assert!(result);

        let result = ConditionEvaluator::evaluate("a.output.status == \"fail\"", &outputs).unwrap();
        assert!(!result);
    }

    #[test]
    fn evaluate_non_bool_result_errors() {
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), json!({"count": 5}));

        let err = ConditionEvaluator::evaluate("a.output.count", &outputs)
            .expect_err("should fail on non-bool");
        let msg = format!("{err}");
        assert!(
            msg.contains("did not yield bool"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn evaluate_missing_step_errors() {
        let outputs = HashMap::new();

        let err = ConditionEvaluator::evaluate("${missing.output.flag}", &outputs)
            .expect_err("should fail on missing step");
        let msg = format!("{err}");
        assert!(
            msg.contains("condition evaluation failed"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn evaluate_logical_operators() {
        let mut outputs = HashMap::new();
        outputs.insert("a".to_string(), json!({"x": true, "y": false}));

        let result = ConditionEvaluator::evaluate("a.output.x && !a.output.y", &outputs).unwrap();
        assert!(result);

        let result = ConditionEvaluator::evaluate("a.output.x || a.output.y", &outputs).unwrap();
        assert!(result);

        let result = ConditionEvaluator::evaluate("a.output.x && a.output.y", &outputs).unwrap();
        assert!(!result);
    }
}
