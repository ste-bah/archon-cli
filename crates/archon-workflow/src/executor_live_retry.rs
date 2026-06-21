use serde_json::json;

use crate::runner::StageRunRequest;

pub(super) fn confirmation_retry_request(
    request: &StageRunRequest,
    previous_body: &str,
) -> Option<StageRunRequest> {
    if !output_asks_for_confirmation(previous_body) {
        return None;
    }
    let mut retry = request.clone();
    retry.attempt = retry.attempt.saturating_add(1);
    retry.task = format!(
        "{}\n\nWorkflow corrective retry: the previous response asked for confirmation or returned a plan-only answer. Do not ask whether to proceed. Execute this stage now using the available tools. Return the required stage artifact directly, or return `status: blocked` only with concrete missing evidence.",
        request.task
    );
    let marker = json!({
        "reason": "previous_output_asked_for_confirmation",
        "previous_output_excerpt": one_line_excerpt(previous_body, 400),
    });
    if let Some(obj) = retry.input.as_object_mut() {
        obj.insert("workflow_retry".into(), marker);
    } else {
        retry.input = json!({ "workflow_retry": marker });
    }
    Some(retry)
}

fn output_asks_for_confirmation(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    [
        "would you like me to proceed",
        "do you want me to proceed",
        "should i proceed",
        "shall i proceed",
        "would you like me to continue",
        "do you want me to continue",
        "let me know if you want me to proceed",
        "let me know if you'd like me to proceed",
        "if you want me to proceed",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn one_line_excerpt(text: &str, max: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::spec::{ProviderTier, StageKind};

    fn request(input: serde_json::Value) -> StageRunRequest {
        StageRunRequest {
            run_id: "run".into(),
            stage_id: "discovery".into(),
            stage_kind: StageKind::Agent,
            agent: None,
            task: "Inspect evidence.".into(),
            attempt: 1,
            provider_tier: ProviderTier::Planner,
            depends_on: Vec::new(),
            input,
        }
    }

    #[test]
    fn confirmation_output_builds_corrective_retry_request() {
        let retry = confirmation_retry_request(
            &request(json!({ "existing": true })),
            "I inspected the files. Would you like me to proceed?",
        )
        .expect("confirmation response should retry");

        assert_eq!(retry.attempt, 2);
        assert!(retry.task.contains("Do not ask whether to proceed"));
        assert_eq!(retry.input["existing"], true);
        assert_eq!(
            retry.input["workflow_retry"]["reason"],
            "previous_output_asked_for_confirmation"
        );
    }

    #[test]
    fn non_confirmation_output_does_not_retry() {
        assert!(confirmation_retry_request(&request(json!({})), "Discovery complete.").is_none());
    }
}
