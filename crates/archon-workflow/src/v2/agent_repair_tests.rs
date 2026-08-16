use super::WorkflowV2AgentError;

/// The live sequence that exhausted the budget in one step: a validation
/// error (Contract) followed by a parse failure (Malformed) are different
/// mistakes, so the second earns its own repair attempt.
#[test]
fn a_parse_failure_after_a_validation_failure_earns_another_repair() {
    let validation = WorkflowV2AgentError::InvalidResult(
        "agent result failed validation: task coverage requires evidence".to_string(),
    );
    let parse = WorkflowV2AgentError::MalformedOutput(
        "agent output must be one JSON WorkflowV2Result object".to_string(),
    );
    assert!(parse.differs_from(&validation));
}

/// Two parse failures in a row are the same mistake repeated; the budget
/// stays bounded.
#[test]
fn repeated_parse_failures_do_not_extend_the_budget() {
    let first = WorkflowV2AgentError::MalformedOutput("first".to_string());
    let second = WorkflowV2AgentError::MalformedOutput("second".to_string());
    assert!(!second.differs_from(&first));
}

/// Two schema violations in a row likewise share one class.
#[test]
fn repeated_validation_failures_do_not_extend_the_budget() {
    let first = WorkflowV2AgentError::InvalidResult("evidence missing".to_string());
    let second = WorkflowV2AgentError::ConfirmationQuestion;
    assert!(!second.differs_from(&first));
}
