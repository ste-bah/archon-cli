use tracing::warn;

use crate::subagent_request::SubagentRequest;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AgentToolError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

pub(super) fn validate_and_build(
    input: &serde_json::Value,
) -> Result<SubagentRequest, AgentToolError> {
    let prompt = input
        .get("prompt")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or(AgentToolError::MissingField("prompt"))?
        .to_string();

    let model = input
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let allowed_tools = match input.get("allowed_tools") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    };

    let max_turns = input
        .get("max_turns")
        .and_then(|v| v.as_u64())
        .map(|n| {
            let cap = u64::from(SubagentRequest::MAX_TURNS_HARD_CAP);
            if n == 0 || n > cap {
                Err(AgentToolError::InvalidInput(format!(
                    "max_turns must be between 1 and {cap}"
                )))
            } else {
                warn!(
                    value = n,
                    tool = "Agent",
                    "max_turns emitted by model despite schema removal -- investigate"
                );
                Ok(n as u32)
            }
        })
        .transpose()?
        .unwrap_or(SubagentRequest::DEFAULT_MAX_TURNS);

    let timeout_secs = input
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(SubagentRequest::DEFAULT_TIMEOUT_SECS);

    let subagent_type = input
        .get("subagent_type")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());

    let run_in_background = input
        .get("run_in_background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let cwd = input
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());

    let isolation = match input
        .get("isolation")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some("none") => None,
        Some("worktree") => Some("worktree".to_string()),
        Some(other) => {
            return Err(AgentToolError::InvalidInput(format!(
                "isolation must be 'none' or 'worktree', got '{other}'"
            )));
        }
        None => None,
    };

    Ok(SubagentRequest {
        prompt,
        model,
        allowed_tools,
        max_turns,
        timeout_secs,
        subagent_type,
        run_in_background,
        cwd,
        isolation,
        provider_env: None,
    })
}

pub(super) fn expected_target_files(
    input: &serde_json::Value,
) -> Result<Vec<String>, AgentToolError> {
    string_array(input, "expected_target_files")
}

/// Paths or globs this subagent says it intends to write (#184 M2).
///
/// Kept beside `SubagentRequest` rather than inside it, following
/// `expected_target_files`: the struct is built by ~24 literal expressions with
/// no `Default`, so a new field is a compile break at every one of them for a
/// value only `AgentTool::execute` reads.
pub(super) fn intended_writes(input: &serde_json::Value) -> Result<Vec<String>, AgentToolError> {
    string_array(input, "intended_writes")
}

/// The existing task this subagent is taking on, if it named one.
pub(super) fn task_id(input: &serde_json::Value) -> Option<String> {
    input
        .get("task_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned)
}

/// Shared reader for the tool's array-of-non-empty-strings fields.
fn string_array(
    input: &serde_json::Value,
    field: &'static str,
) -> Result<Vec<String>, AgentToolError> {
    let Some(value) = input.get(field) else {
        return Ok(Vec::new());
    };
    let Some(array) = value.as_array() else {
        return Err(AgentToolError::InvalidInput(format!(
            "{field} must be an array of strings"
        )));
    };
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    AgentToolError::InvalidInput(format!(
                        "{field} must contain only non-empty strings"
                    ))
                })
        })
        .collect()
}
