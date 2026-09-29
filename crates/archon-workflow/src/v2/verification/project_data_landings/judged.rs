//! Whether a remediation verdict judged every project-data landing it was
//! shown (Batch K, I2): the gate an accepted verdict must pass.

use serde_json::Value;

use super::{ANSWER_KEY, stamped};
use crate::v2::WorkflowV2Status;

/// Whether `answer` (a path, `dir/` or `*`) covers `required`.
pub(crate) fn covers(answer: &str, required: &str) -> bool {
    let answer = answer.trim().trim_start_matches("./");
    answer == "*"
        || answer == required
        || (answer.ends_with('/') && required.starts_with(answer))
        || (!answer.is_empty() && required.starts_with(&format!("{answer}/")))
}

/// Refuse an accepted verdict on a stamped item that does not judge every
/// landing, or judges one not legitimate.
pub(crate) fn judged(
    request: &crate::v2::agent_adapter::WorkflowV2AgentRequest,
    result: &crate::WorkflowV2Result,
) -> Result<(), crate::v2::agent_adapter::WorkflowV2AgentError> {
    use crate::v2::agent_adapter::WorkflowV2AgentError;
    let Some(stamp) = stamped(&request.input) else {
        return Ok(());
    };
    if !matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        return Ok(());
    }
    let answers: Vec<(String, Option<bool>, bool)> = result
        .data
        .get(ANSWER_KEY)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let path = entry.get("path")?.as_str()?.to_string();
            let legitimate = entry.get("legitimate").and_then(Value::as_bool);
            let reasoned = entry
                .get("provenance")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty());
            Some((path, legitimate, reasoned))
        })
        .collect();
    let mut unjudged = Vec::new();
    let mut refused = Vec::new();
    for (required, flagged) in stamp.required() {
        let covering: Vec<_> = answers
            .iter()
            .filter(|(path, _, _)| {
                if flagged {
                    path.trim().trim_start_matches("./") == required
                } else {
                    covers(path, &required)
                }
            })
            .collect();
        if covering
            .iter()
            .any(|(_, legitimate, _)| *legitimate == Some(false))
        {
            refused.push(required);
        } else if !covering
            .iter()
            .any(|(_, legitimate, reasoned)| *legitimate == Some(true) && *reasoned)
        {
            unjudged.push(required);
        }
    }
    let mut violations = Vec::new();
    if !unjudged.is_empty() {
        violations.push(WorkflowV2AgentError::ProjectDataLandingsUnjudged(unjudged));
    }
    if !refused.is_empty() {
        violations.push(WorkflowV2AgentError::AcceptedWithIllegitimateProjectData(
            refused,
        ));
    }
    WorkflowV2AgentError::all_of(violations)
}
