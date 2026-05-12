#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCorrectionEventPayload {
    pub correction_type: String,
    pub top_rule_id: Option<String>,
    pub user_input_excerpt: String,
    pub session_context: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningEvidenceEventPayload {
    pub evidence_id: String,
    pub kind: String,
    pub entity_key: Option<String>,
    pub output_hash: Option<String>,
    pub redacted_excerpt: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningTurnEventPayload {
    pub session_id: String,
    pub turn_number: u64,
    pub assistant_text: String,
    pub evidence_refs: Vec<ReasoningEvidenceEventPayload>,
    pub cwd: Option<String>,
    pub workspace_root: Option<String>,
}
