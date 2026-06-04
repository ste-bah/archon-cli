use std::path::PathBuf;

use thiserror::Error;

pub type WorkflowResult<T> = Result<T, WorkflowError>;

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("invalid workflow schema: expected archon.workflow.v1, got {0}")]
    InvalidSchema(String),
    #[error("invalid workflow spec: {0}")]
    SpecInvalid(String),
    #[error("unknown dependency '{dependency}' referenced by stage '{stage}'")]
    UnknownDependency { stage: String, dependency: String },
    #[error("dependency cycle detected: {0:?}")]
    DependencyCycle(Vec<String>),
    #[error("stage '{0}' requires a reducer")]
    MissingReducer(String),
    #[error("hard-coded provider/model is forbidden on stage '{0}'")]
    HardcodedModel(String),
    #[error("invalid fan-out contract: {0}")]
    InvalidFanout(String),
    #[error("stage '{stage}' requires field '{field}'")]
    MissingStageField { stage: String, field: &'static str },
    #[error("duplicate stage id '{0}'")]
    DuplicateStage(String),
    #[error("workflow run already exists: {0}")]
    RunAlreadyExists(String),
    #[error("workflow run not found: {0}")]
    RunNotFound(String),
    #[error("workflow state is corrupt: {0}")]
    StateCorrupt(String),
    #[error("artifact cannot be reused: {0}")]
    ArtifactInvalid(String),
    #[error("forbidden provider-private payload field stripped: {0}")]
    ForbiddenPayload(String),
    #[error("policy denied workflow action: {0}")]
    PolicyDenied(String),
    #[error("provider tier '{0}' could not be resolved")]
    ProviderTierUnresolved(String),
    #[error("workflow stage failed: {0}")]
    StageFailed(String),
    #[error("workflow template is unsafe: {0}")]
    UnsafeTemplate(String),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml_ng::Error),
    #[error(transparent)]
    TomlSerialize(#[from] toml::ser::Error),
    #[error(transparent)]
    TomlDeserialize(#[from] toml::de::Error),
}

impl WorkflowError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
