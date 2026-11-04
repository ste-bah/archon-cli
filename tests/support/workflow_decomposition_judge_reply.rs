//! Offline replies use the production judge wire type so required fields cannot drift.
#[path = "../../src/command/workflow_task_set_judge_schema.rs"]
#[allow(dead_code)]
mod schema;

pub(crate) fn judge_decision(id: &str, refuted: bool) -> serde_json::Value {
    use archon_workflow::task_set_contract::JudgeDecision;
    let decision = schema::JudgeResponse {
        id: id.into(),
        verdict: if refuted {
            JudgeDecision::Refuted
        } else {
            JudgeDecision::Accepted
        },
        kind: if refuted {
            schema::RefutationKind::State
        } else {
            schema::RefutationKind::Other
        },
        cited_paths: Vec::new(),
        counterexample: if refuted {
            "a passing state where the criterion is false"
        } else {
            "None found: a missing or false outcome makes the command exit non-zero."
        }
        .into(),
        reason: if refuted {
            "the check admits that state"
        } else {
            "The command directly checks the declared outcome and exits non-zero when it is false."
        }
        .into(),
    };
    serde_json::to_value(decision).unwrap()
}
