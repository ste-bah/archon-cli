use archon_tui::state::AppState;
use archon_tui::task_dispatch::AgentRouter;
use archon_tui::theme::intj_theme;
use std::sync::Arc;

/// A no-op router for testing AppState construction.
struct MockRouter;

impl AgentRouter for MockRouter {
    fn switch(&self, _agent_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[test]
fn app_state_new_compiles() {
    let router = Arc::new(MockRouter);
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let theme = intj_theme();
    let _state = AppState::new(router, tx, theme);
}
