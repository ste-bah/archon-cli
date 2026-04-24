// TASK-AGS-300: Canonical JSON Schema for agent metadata documents.
//
// Provides compile-once validation against Draft 2020-12 schema used by
// both local and remote discovery sources. Invalid files become
// AgentState::Invalid(reason) per EC-DISCOVERY-001.

use serde_json::Value;

/// Canonical JSON Schema (Draft 2020-12) for agent metadata documents.
///
/// Required fields: name (regex), version (SemVer string), description,
/// resource_requirements (cpu, memory_mb, timeout_sec).
/// Optional: tags, capabilities, input_schema, output_schema, dependencies.
pub const CANONICAL_AGENT_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "required": ["name", "version", "description", "resource_requirements"],
  "properties": {
    "name": {
      "type": "string",
      "pattern": "^[a-z0-9][a-z0-9-]*$"
    },
    "version": {
      "type": "string"
    },
    "description": {
      "type": "string",
      "minLength": 1
    },
    "tags": {
      "type": "array",
      "items": { "type": "string" },
      "default": []
    },
    "capabilities": {
      "type": "array",
      "items": { "type": "string" },
      "default": []
    },
    "input_schema": {
      "type": "object",
      "default": {}
    },
    "output_schema": {
      "type": "object",
      "default": {}
    },
    "resource_requirements": {
      "type": "object",
      "required": ["cpu", "memory_mb", "timeout_sec"],
      "properties": {
        "cpu": { "type": "number" },
        "memory_mb": { "type": "integer" },
        "timeout_sec": { "type": "integer" }
      }
    },
    "dependencies": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["name"],
        "properties": {
          "name": { "type": "string" },
          "version": { "type": "string" }
        }
      }
    },
    "category": {
      "type": "string"
    }
  },
  "additionalProperties": true
}"#;

/// Error compiling the canonical schema (should never happen at runtime
/// unless the embedded schema JSON is malformed — programming error).
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("failed to compile agent metadata schema: {0}")]
    Compile(String),
}

/// Report from validating a document against the canonical agent schema.
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub missing_fields: Vec<String>,
    pub errors: Vec<String>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.missing_fields.is_empty() && self.errors.is_empty()
    }

    /// Flatten all issues into a single human-readable reason string.
    pub fn reason(&self) -> String {
        let mut parts = Vec::new();
        if !self.missing_fields.is_empty() {
            parts.push(format!(
                "missing fields: {}",
                self.missing_fields.join(", ")
            ));
        }
        if !self.errors.is_empty() {
            parts.push(format!("errors: {}", self.errors.join("; ")));
        }
        parts.join("; ")
    }
}

/// Validates agent metadata documents against the canonical JSON Schema.
///
/// Construct once via `new()`, then call `validate()` on each document.
/// Thread-safe and cheaply shareable via `Arc<AgentSchemaValidator>`.
pub struct AgentSchemaValidator {
    schema: jsonschema::Validator,
}

impl AgentSchemaValidator {
    /// Compile the canonical schema. Returns `Err` only on programming
    /// errors (malformed embedded schema).
    pub fn new() -> Result<Self, SchemaError> {
        let schema_value: Value = serde_json::from_str(CANONICAL_AGENT_SCHEMA)
            .map_err(|e| SchemaError::Compile(e.to_string()))?;
        let schema = jsonschema::validator_for(&schema_value)
            .map_err(|e| SchemaError::Compile(e.to_string()))?;
        Ok(Self { schema })
    }

    /// Validate a JSON value against the canonical agent metadata schema.
    pub fn validate(&self, value: &Value) -> Result<(), ValidationReport> {
        let errors: Vec<String> = self
            .schema
            .iter_errors(value)
            .map(|e| e.to_string())
            .collect();

        if errors.is_empty() {
            return Ok(());
        }

        let mut report = ValidationReport::default();
        let required_fields = ["name", "version", "description", "resource_requirements"];

        for msg in errors {
            let mut found_missing = false;
            for field in &required_fields {
                if msg.contains(&format!("\"{field}\"")) && msg.contains("required") {
                    if !report.missing_fields.contains(&field.to_string()) {
                        report.missing_fields.push(field.to_string());
                    }
                    found_missing = true;
                }
            }
            if !found_missing {
                report.errors.push(msg);
            }
        }

        Err(report)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_fixture() -> Value {
        serde_json::json!({
            "name": "my-agent",
            "version": "1.0.0",
            "description": "A test agent",
            "tags": ["rust", "test"],
            "capabilities": ["code-review"],
            "input_schema": {},
            "output_schema": {},
            "resource_requirements": {
                "cpu": 1.0,
                "memory_mb": 512,
                "timeout_sec": 300
            }
        })
    }

    #[test]
    fn valid_fixture_passes() {
        let validator = AgentSchemaValidator::new().unwrap();
        assert!(validator.validate(&valid_fixture()).is_ok());
    }

    #[test]
    fn missing_name_detected() {
        let validator = AgentSchemaValidator::new().unwrap();
        let mut doc = valid_fixture();
        doc.as_object_mut().unwrap().remove("name");
        let err = validator.validate(&doc).unwrap_err();
        assert!(
            err.missing_fields.contains(&"name".to_string()),
            "expected 'name' in missing_fields, got: {:?}",
            err.missing_fields
        );
    }

    #[test]
    fn missing_version_detected() {
        let validator = AgentSchemaValidator::new().unwrap();
        let mut doc = valid_fixture();
        doc.as_object_mut().unwrap().remove("version");
        let err = validator.validate(&doc).unwrap_err();
        assert!(
            err.missing_fields.contains(&"version".to_string()),
            "expected 'version' in missing_fields, got: {:?}",
            err.missing_fields
        );
    }

    #[test]
    fn invalid_name_regex_rejected() {
        let validator = AgentSchemaValidator::new().unwrap();
        let mut doc = valid_fixture();
        doc["name"] = serde_json::json!("Bad Name!");
        let err = validator.validate(&doc).unwrap_err();
        assert!(
            !err.errors.is_empty(),
            "expected errors for regex violation, got: {:?}",
            err
        );
    }

    #[test]
    fn missing_resource_requirements_detected() {
        let validator = AgentSchemaValidator::new().unwrap();
        let mut doc = valid_fixture();
        doc.as_object_mut().unwrap().remove("resource_requirements");
        let err = validator.validate(&doc).unwrap_err();
        assert!(
            err.missing_fields
                .contains(&"resource_requirements".to_string()),
            "expected 'resource_requirements' in missing_fields, got: {:?}",
            err.missing_fields
        );
    }
}
