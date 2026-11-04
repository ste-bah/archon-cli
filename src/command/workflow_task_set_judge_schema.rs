//! Main acceptance judge wire schema, also used by offline judge fixtures.
use archon_workflow::task_set_contract::JudgeDecision;

#[derive(Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum RefutationKind {
    State,
    Duplicate,
    Coverage,
    Other,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(super) struct BatchedJudgeResponse {
    pub(super) decisions: Vec<JudgeResponse>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(super) struct JudgeResponse {
    pub(super) kind: RefutationKind,
    pub(super) cited_paths: Vec<String>,
    pub(super) id: String,
    pub(super) verdict: JudgeDecision,
    pub(super) counterexample: String,
    pub(super) reason: String,
}
