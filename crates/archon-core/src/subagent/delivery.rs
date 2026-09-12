use super::SubagentManager;

impl SubagentManager {
    pub(crate) fn set_parent(&mut self, id: &str, parent: Option<&str>) {
        if let Some(parent) = parent {
            self.parent_ids.insert(id.to_string(), parent.to_string());
        } else {
            self.parent_ids.entry(id.to_string())
                .or_insert_with(|| crate::message_router::LEAD_QUEUE_ID.to_string());
        }
    }

    pub(crate) fn parent_id(&self, id: &str) -> &str {
        self.parent_ids.get(id).map(String::as_str)
            .unwrap_or(crate::message_router::LEAD_QUEUE_ID)
    }

    /// Completed names remain discoverable without changing the running-name registry.
    pub(crate) fn result_id(&self, name: &str) -> Option<&str> {
        if let Some(info) = self.agents.get(name) { return Some(&info.id); }
        if let Some(id) = self.resolve_name(name) { return Some(id); }
        self.agents.values().filter(|info| info.request.subagent_type.as_deref() == Some(name))
            .max_by_key(|info| info.generation).map(|info| info.id.as_str())
    }
}
