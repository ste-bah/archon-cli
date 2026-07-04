use regex::Regex;
use thiserror::Error;

use super::harness_safety::{code_without_string_literals, reject_unsafe_source, strip_comments};
use super::host_api::{
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode,
};

#[derive(Debug, Default, Clone)]
pub struct WorkflowV2HarnessValidator;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2HarnessPlan {
    pub calls: Vec<WorkflowV2HostCall>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2HarnessError {
    #[error("workflow harness contains forbidden token `{0}`")]
    ForbiddenToken(&'static str),
    #[error("workflow harness uses unsupported host method w.{0}")]
    UnsupportedHostMethod(String),
    #[error("workflow harness declares no executable host calls")]
    NoHostCalls,
    #[error("host call w.{0}(...) must pass a non-empty string id as its first argument")]
    HostCallRequiresLiteralId(String),
    #[error("host call w.{method}('{id}') has invalid write mode `{value}`")]
    InvalidWriteMode {
        method: String,
        id: String,
        value: String,
    },
    #[error(
        "write-capable host call w.{method}('{id}') requires write: \"serial\", \"coordinated\", or \"worktree\""
    )]
    MissingWriteMode { method: String, id: String },
    #[error("fanout host call w.fanout('{0}') must include a typed item source argument")]
    UntypedFanout(String),
    #[error("workflow harness loop is unsupported: {0}")]
    UnsupportedLoop(String),
    #[error("workflow harness has duplicate host call metadata id `{0}`")]
    DuplicateHostCallId(String),
}

impl WorkflowV2HarnessValidator {
    pub fn validate(&self, source: &str) -> Result<WorkflowV2HarnessPlan, WorkflowV2HarnessError> {
        let executable = strip_comments(source);
        reject_unsafe_source(&executable)?;
        reject_opaque_host_api_usage(&executable)?;
        let calls = extract_host_calls(&executable)?;
        if calls.is_empty() {
            return Err(WorkflowV2HarnessError::NoHostCalls);
        }
        Ok(WorkflowV2HarnessPlan { calls })
    }
}

include!("harness_collect.rs");

include!("harness_parse_call.rs");

include!("harness_lex.rs");

include!("harness_sources.rs");

include!("harness_options.rs");
