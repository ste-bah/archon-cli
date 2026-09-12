use super::SubagentRunner;
impl SubagentRunner {
    pub(super) fn evidence_recovery_message(&self) -> serde_json::Value {
        let mut hint = "Your evidence was summarised. This is not task completion: return the full contracted artifact, not a narration or a handover. Preserve established findings and re-establish only missing evidence.\n".to_string();
        if let Some(guard) = &self.tool_context.workflow_read_guard { hint.push_str(&guard.orientation()); }
        if let Some(landing) = &self.tool_context.audit_landing {
            hint.push_str(&landing.hint().unwrap_or_else(|e|format!("Host audit record recovery failed: {e}")));
        }
        if let (Some(store),Some(id)) = (&self.transcript_store,&self.transcript_agent_id) {
            hint.push_str(&format!("\nOriginal evidence remains in {}. Retrieve only needed transcript ranges, not the whole log.",store.transcript_path(id).display()));
        }
        serde_json::json!({"role":"user","content":hint})
    }
}
