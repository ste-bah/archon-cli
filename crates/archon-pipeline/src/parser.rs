//! YAML/JSON pipeline parser.

use std::path::Path;

use crate::error::PipelineError;
use crate::spec::{PipelineFormat, PipelineSpec};

/// Parses pipeline definitions from YAML or JSON.
pub struct PipelineParser;

impl PipelineParser {
    /// Parse a pipeline spec from a string in the given format.
    pub fn parse_str(src: &str, fmt: PipelineFormat) -> Result<PipelineSpec, PipelineError> {
        let spec = match fmt {
            PipelineFormat::Yaml => {
                serde_yml::from_str::<PipelineSpec>(src).map_err(|e| PipelineError::ParseError {
                    path: "<string>".to_string(),
                    line: e.location().map(|l| l.line()),
                    msg: e.to_string(),
                })?
            }
            PipelineFormat::Json => serde_json::from_str::<PipelineSpec>(src).map_err(|e| {
                PipelineError::ParseError {
                    path: "<string>".to_string(),
                    line: Some(e.line()),
                    msg: e.to_string(),
                }
            })?,
        };
        Self::validate_structure(&spec)?;
        Ok(spec)
    }

    /// Parse a pipeline spec from a file, auto-detecting format.
    pub fn parse_file(path: &Path) -> Result<PipelineSpec, PipelineError> {
        let src = std::fs::read_to_string(path).map_err(PipelineError::Io)?;
        let fmt = Self::detect_format(path, &src);
        let spec = match fmt {
            PipelineFormat::Yaml => serde_yml::from_str::<PipelineSpec>(&src).map_err(|e| {
                PipelineError::ParseError {
                    path: path.display().to_string(),
                    line: e.location().map(|l| l.line()),
                    msg: e.to_string(),
                }
            })?,
            PipelineFormat::Json => serde_json::from_str::<PipelineSpec>(&src).map_err(|e| {
                PipelineError::ParseError {
                    path: path.display().to_string(),
                    line: Some(e.line()),
                    msg: e.to_string(),
                }
            })?,
        };
        Self::validate_structure(&spec)?;
        Ok(spec)
    }

    /// Detect format by file extension, falling back to content sniffing.
    fn detect_format(path: &Path, content: &str) -> PipelineFormat {
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") => PipelineFormat::Json,
            Some("yaml" | "yml") => PipelineFormat::Yaml,
            _ => {
                // Content sniffing: first non-whitespace char
                let first = content.trim_start().chars().next();
                if first == Some('{') || first == Some('[') {
                    PipelineFormat::Json
                } else {
                    PipelineFormat::Yaml
                }
            }
        }
    }

    /// Structural validation after parse.
    fn validate_structure(spec: &PipelineSpec) -> Result<(), PipelineError> {
        if spec.steps.is_empty() {
            return Err(PipelineError::ValidationError(
                "pipeline has no steps".to_string(),
            ));
        }

        let mut seen = std::collections::HashSet::new();
        for step in &spec.steps {
            if step.id.is_empty() {
                return Err(PipelineError::ValidationError(
                    "step id must not be empty".to_string(),
                ));
            }
            if step.id.contains('.') {
                return Err(PipelineError::ValidationError(format!(
                    "step id '{}' must not contain '.' (reserved for variable substitution)",
                    step.id
                )));
            }
            if !seen.insert(&step.id) {
                return Err(PipelineError::ValidationError(format!(
                    "duplicate step id: {}",
                    step.id
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn yaml_and_json_parse_to_equivalent_spec() {
        let yaml_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/example.yaml");
        let json_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/example.json");

        let yaml_spec = PipelineParser::parse_file(&yaml_path).expect("YAML fixture should parse");
        let json_spec = PipelineParser::parse_file(&json_path).expect("JSON fixture should parse");

        assert_eq!(yaml_spec, json_spec);
    }

    #[test]
    fn empty_steps_rejected() {
        let input = r#"{"name":"test","steps":[]}"#;
        let err = PipelineParser::parse_str(input, PipelineFormat::Json)
            .expect_err("empty steps should fail validation");
        let msg = err.to_string();
        assert!(
            msg.contains("no steps"),
            "expected 'no steps' in error, got: {msg}"
        );
    }

    #[test]
    fn duplicate_step_id_rejected() {
        let input = r#"{
            "name": "dup-test",
            "steps": [
                {"id": "alpha", "agent": "a"},
                {"id": "alpha", "agent": "b"}
            ]
        }"#;
        let err = PipelineParser::parse_str(input, PipelineFormat::Json)
            .expect_err("duplicate step ids should fail validation");
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate step id"),
            "expected 'duplicate step id' in error, got: {msg}"
        );
    }

    #[test]
    fn malformed_yaml_yields_line_number() {
        // Invalid YAML: a mapping value where a scalar is expected
        let bad_yaml = "name: test\nsteps:\n  - id: ok\n    agent: :\n";
        let err = PipelineParser::parse_str(bad_yaml, PipelineFormat::Yaml)
            .expect_err("malformed YAML should fail");
        match err {
            PipelineError::ParseError { line, .. } => {
                assert!(
                    line.is_some(),
                    "YAML parse error should include a line number"
                );
            }
            other => panic!("expected ParseError, got: {other:?}"),
        }
    }

    #[test]
    fn malformed_json_yields_line_number() {
        let bad_json = "{\n  \"name\": \"test\",\n  \"steps\": [\n    { bad }\n  ]\n}";
        let err = PipelineParser::parse_str(bad_json, PipelineFormat::Json)
            .expect_err("malformed JSON should fail");
        match err {
            PipelineError::ParseError { line, .. } => {
                assert!(
                    line.is_some(),
                    "JSON parse error should include a line number"
                );
            }
            other => panic!("expected ParseError, got: {other:?}"),
        }
    }

    #[test]
    fn auto_detect_json_by_content() {
        // File with no extension — content starts with '{', so JSON is detected
        let json_content = r#"{"name":"sniff-test","steps":[{"id":"s1","agent":"x"}]}"#;
        let tmp = tempfile::NamedTempFile::new().expect("create temp file");
        std::fs::write(tmp.path(), json_content).expect("write temp");

        let spec = PipelineParser::parse_file(tmp.path()).expect("auto-detected JSON should parse");
        assert_eq!(spec.name, "sniff-test");
    }

    #[test]
    fn dot_in_step_id_rejected() {
        let input = r#"{"name":"dot-test","steps":[{"id":"foo.bar","agent":"a"}]}"#;
        let err = PipelineParser::parse_str(input, PipelineFormat::Json)
            .expect_err("dot in step id should fail validation");
        let msg = err.to_string();
        assert!(
            msg.contains("must not contain '.'"),
            "expected dot error in: {msg}"
        );
    }

    #[test]
    fn empty_step_id_rejected() {
        let input = r#"{"name":"empty-id-test","steps":[{"id":"","agent":"a"}]}"#;
        let err = PipelineParser::parse_str(input, PipelineFormat::Json)
            .expect_err("empty step id should fail validation");
        let msg = err.to_string();
        assert!(
            msg.contains("must not be empty"),
            "expected empty-id error in: {msg}"
        );
    }
}
