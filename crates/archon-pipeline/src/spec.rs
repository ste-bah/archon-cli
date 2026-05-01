//! Pipeline specification types — declarative definition of a pipeline.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Top-level pipeline specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PipelineSpec {
    /// Human-readable pipeline name.
    pub name: String,

    /// Spec format version string.
    #[serde(default = "default_version")]
    pub version: String,

    /// Maximum wall-clock seconds for the entire pipeline run.
    #[serde(default = "default_global_timeout_secs")]
    pub global_timeout_secs: u64,

    /// Maximum number of steps that may execute concurrently.
    #[serde(default = "default_max_parallelism")]
    pub max_parallelism: u32,

    /// Ordered list of step specifications.
    pub steps: Vec<StepSpec>,
}

impl PipelineSpec {
    /// Returns the global timeout as a `Duration`.
    pub fn global_timeout(&self) -> Duration {
        Duration::from_secs(self.global_timeout_secs)
    }
}

fn default_version() -> String {
    "1.0".to_string()
}

fn default_global_timeout_secs() -> u64 {
    3600
}

fn default_max_parallelism() -> u32 {
    5
}

/// Specification for a single pipeline step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepSpec {
    /// Unique identifier within the pipeline.
    pub id: String,

    /// Agent key that executes this step.
    pub agent: String,

    /// Input payload forwarded to the agent.
    #[serde(default = "default_input")]
    pub input: serde_json::Value,

    /// Step IDs that must complete before this step starts.
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Retry policy for this step.
    #[serde(default)]
    pub retry: RetrySpec,

    /// Maximum wall-clock seconds for this step.
    #[serde(default = "default_step_timeout_secs")]
    pub timeout_secs: u64,

    /// Optional CEL condition; step is skipped when it evaluates to false.
    #[serde(default)]
    pub condition: Option<String>,

    /// Policy applied when this step fails.
    #[serde(default)]
    pub on_failure: OnFailurePolicy,
}

impl StepSpec {
    /// Returns the step timeout as a `Duration`.
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }
}

fn default_input() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

fn default_step_timeout_secs() -> u64 {
    1800
}

/// Retry policy for a step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrySpec {
    /// Maximum number of attempts (1 = no retry).
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,

    /// Backoff strategy between retries.
    #[serde(default)]
    pub backoff: BackoffKind,

    /// Base delay in milliseconds before first retry.
    #[serde(default = "default_base_delay_ms")]
    pub base_delay_ms: u64,
}

impl Default for RetrySpec {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            backoff: BackoffKind::default(),
            base_delay_ms: default_base_delay_ms(),
        }
    }
}

impl RetrySpec {
    /// Returns the base delay as a `Duration`.
    pub fn base_delay(&self) -> Duration {
        Duration::from_millis(self.base_delay_ms)
    }
}

fn default_max_attempts() -> u32 {
    1
}

fn default_base_delay_ms() -> u64 {
    1000
}

/// Backoff strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum BackoffKind {
    Fixed,
    Linear,
    #[default]
    Exponential,
}


/// Policy applied when a step fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum OnFailurePolicy {
    Retry,
    #[default]
    Rollback,
    Skip,
    Fail,
}


/// Supported pipeline definition formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineFormat {
    Yaml,
    Json,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_serde_roundtrip() {
        let spec = PipelineSpec {
            name: "roundtrip-test".to_string(),
            version: "2.0".to_string(),
            global_timeout_secs: 7200,
            max_parallelism: 10,
            steps: vec![StepSpec {
                id: "step-a".to_string(),
                agent: "analyzer".to_string(),
                input: serde_json::json!({"key": "value"}),
                depends_on: vec!["step-b".to_string()],
                retry: RetrySpec {
                    max_attempts: 3,
                    backoff: BackoffKind::Linear,
                    base_delay_ms: 500,
                },
                timeout_secs: 900,
                condition: Some("result.ok == true".to_string()),
                on_failure: OnFailurePolicy::Fail,
            }],
        };

        let json = serde_json::to_string(&spec).expect("serialize");
        let deserialized: PipelineSpec = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(spec, deserialized);
    }

    #[test]
    fn spec_defaults() {
        let json = r#"{"name":"test","steps":[{"id":"a","agent":"x"}]}"#;
        let spec: PipelineSpec = serde_json::from_str(json).expect("deserialize");

        assert_eq!(spec.name, "test");
        assert_eq!(spec.version, "1.0");
        assert_eq!(spec.global_timeout_secs, 3600);
        assert_eq!(spec.max_parallelism, 5);
        assert_eq!(spec.steps.len(), 1);

        let step = &spec.steps[0];
        assert_eq!(step.id, "a");
        assert_eq!(step.agent, "x");
        assert_eq!(
            step.input,
            serde_json::Value::Object(serde_json::Map::new())
        );
        assert!(step.depends_on.is_empty());
        assert_eq!(step.timeout_secs, 1800);
        assert_eq!(step.on_failure, OnFailurePolicy::Rollback);
        assert_eq!(step.retry.max_attempts, 1);
        assert_eq!(step.retry.backoff, BackoffKind::Exponential);
        assert_eq!(step.retry.base_delay_ms, 1000);
        assert!(step.condition.is_none());
    }

    #[test]
    fn on_failure_variants() {
        for variant in [
            OnFailurePolicy::Retry,
            OnFailurePolicy::Rollback,
            OnFailurePolicy::Skip,
            OnFailurePolicy::Fail,
        ] {
            let json = serde_json::to_string(&variant).expect("serialize");
            let deserialized: OnFailurePolicy = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(variant, deserialized);
        }
    }

    #[test]
    fn backoff_variants() {
        for variant in [
            BackoffKind::Fixed,
            BackoffKind::Linear,
            BackoffKind::Exponential,
        ] {
            let json = serde_json::to_string(&variant).expect("serialize");
            let deserialized: BackoffKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(variant, deserialized);
        }
    }

    #[test]
    fn pipeline_format_variants() {
        for variant in [PipelineFormat::Yaml, PipelineFormat::Json] {
            let json = serde_json::to_string(&variant).expect("serialize");
            let deserialized: PipelineFormat = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(variant, deserialized);
        }
    }
}
