use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct SpoofConfig {
    pub originator: String,
    pub user_agent: String,
    pub client_id: String,
    pub openai_beta: String,
    pub extra_headers: BTreeMap<String, String>,
}

impl Default for SpoofConfig {
    fn default() -> Self {
        Self {
            originator: "openclaw".into(),
            user_agent: format!("openclaw/{}", env!("CARGO_PKG_VERSION")),
            client_id: "app_EMoamEEZ73f0CkXaXp7hrann".into(),
            openai_beta: "responses=experimental".into(),
            extra_headers: BTreeMap::new(),
        }
    }
}
