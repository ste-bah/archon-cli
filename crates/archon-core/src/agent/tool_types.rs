use std::sync::Arc;

pub(super) struct PreflightResult {
    pub(super) tool_name: String,
    pub(super) tool_id: String,
    pub(super) input: serde_json::Value,
    pub(super) tool_arc: Arc<dyn archon_tools::tool::Tool>,
    pub(super) file_path: Option<String>,
}
