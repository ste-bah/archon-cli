use super::*;
use crate::command::workflow_task_set::reauthor::AuthorScope;
use archon_workflow::WorkflowLlmClient;
use std::sync::Arc;

pub(super) async fn build(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    project_root: &Path,
    scope: &AuthorScope,
) -> Result<Arc<dyn WorkflowLlmClient>> {
    #[cfg(test)]
    if let Some(client) = CLIENTS.lock().unwrap().get(project_root).cloned() {
        return Ok(client);
    }
    crate::command::workflow::reverify_acceptance_client(
        config,
        env_vars,
        project_root,
        "workflow-reverify-acceptance",
        scope,
    )
    .await
}

#[cfg(test)]
static CLIENTS: std::sync::LazyLock<
    std::sync::Mutex<BTreeMap<std::path::PathBuf, Arc<dyn WorkflowLlmClient>>>,
> = std::sync::LazyLock::new(Default::default);

#[cfg(test)]
pub(crate) struct Injection(std::path::PathBuf);
#[cfg(test)]
impl Drop for Injection {
    fn drop(&mut self) {
        CLIENTS.lock().unwrap().remove(&self.0);
    }
}
#[cfg(test)]
pub(crate) fn inject(project: &Path, client: Arc<dyn WorkflowLlmClient>) -> Injection {
    CLIENTS
        .lock()
        .unwrap()
        .insert(project.to_path_buf(), client);
    Injection(project.to_path_buf())
}
