//! Explicit strict-rule assertions; historical recordings stay unchanged.
use serde_json::{Value, json};

pub fn expected(name: &str, recorded: &Value) -> Value {
    let invalid_payload = match name {
        "series_nonfinite_infinity" | "series_nonfinite_nan" => {
            Some(("expected value at line 1 column 7", "x"))
        }
        "series_nonfinite_negative_infinity" => Some(("invalid number at line 1 column 8", "x")),
        "steps_repeated_infinity" | "steps_repeated_nan" => {
            Some(("expected value at line 1 column 7", "y"))
        }
        "steps_repeated_negative_infinity" => Some(("invalid number at line 1 column 8", "y")),
        _ => None,
    };
    if let Some((error, field)) = invalid_payload {
        return json!({"verdict":"failed", "findings":[
            format!("payload cannot be parsed as json: <root-0>/payload.json: {error}"),
            "a payload has no records",
            format!("a payload field is constant or absent: {field}")
        ]});
    }
    let findings = match name {
        "identity_bool_numeric" => vec![
            "missing required cell: [1]",
            "extra undeclared cell: [true]",
        ],
        "integer_bool" | "integer_float" => vec![
            "a positive count field failed: v",
            "a minimum count field is not numeric: v",
        ],
        "minimum_zero_empty_object" | "minimum_zero_missing" | "minimum_zero_null" => {
            vec!["a minimum count field is not numeric: n"]
        }
        "json_nan" | "json_infinity" | "json_negative_infinity" => {
            vec!["declared deliverable is not valid JSON"]
        }
        "nonempty_object" => vec!["a required non-empty field failed: v"],
        "registry_null" => vec!["declared registry contains a null value"],
        "registry_null_key" => vec!["null registry key contains a null or missing value"],
        "request_bool" | "request_float" => vec!["a requested count is not numeric"],
        _ => return recorded["native"].clone(),
    };
    json!({"verdict":"failed", "findings":findings})
}
