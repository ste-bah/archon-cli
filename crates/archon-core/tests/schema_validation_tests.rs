use archon_core::schema_validation::{extract_json, validate_json_schema};

#[test]
fn validate_valid_json() {
    let json = r#"{"name":"test"}"#;
    let schema = r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}"#;
    let result = validate_json_schema(json, schema);
    assert!(result.is_ok(), "valid JSON should pass: {:?}", result.err());
}

#[test]
fn validate_invalid_json() {
    let json = r#"{"name":123}"#;
    let schema = r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}"#;
    let result = validate_json_schema(json, schema);
    assert!(result.is_err(), "type mismatch should fail");
    let errors = result.unwrap_err();
    assert!(!errors.is_empty());
}

#[test]
fn validate_missing_required() {
    let json = r#"{}"#;
    let schema = r#"{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}"#;
    let result = validate_json_schema(json, schema);
    assert!(result.is_err(), "missing required field should fail");
}

#[test]
fn extract_json_raw() {
    let input = r#"{"a":1}"#;
    let result = extract_json(input);
    assert!(result.is_some());
    assert_eq!(result.as_deref(), Some(r#"{"a":1}"#));
}

#[test]
fn extract_json_from_code_block() {
    let input = "```json\n{\"a\":1}\n```";
    let result = extract_json(input);
    assert!(result.is_some());
    // Parse both to compare structurally
    let parsed: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    assert_eq!(parsed, serde_json::json!({"a": 1}));
}

#[test]
fn extract_json_no_json() {
    let input = "just text with no json";
    let result = extract_json(input);
    assert!(result.is_none());
}

#[test]
fn validate_empty_schema() {
    let json = r#"{"anything":"goes"}"#;
    let schema = r#"{}"#;
    let result = validate_json_schema(json, schema);
    assert!(
        result.is_ok(),
        "empty schema should accept any JSON: {:?}",
        result.err()
    );
}

#[test]
fn validate_malformed_schema() {
    let json = r#"{"a":1}"#;
    let schema = "not valid json {{{";
    let result = validate_json_schema(json, schema);
    assert!(result.is_err(), "malformed schema should return error");
}

#[test]
fn validate_malformed_json() {
    let json = "not json at all";
    let schema = r#"{"type":"object"}"#;
    let result = validate_json_schema(json, schema);
    assert!(result.is_err(), "malformed JSON input should return error");
}
