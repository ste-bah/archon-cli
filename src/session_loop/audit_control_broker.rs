//! External callers can request controls, never approve them.
use crate::cli_args::workflow_audit::AuditAction;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct RequestBroker {
    receiver: tokio::sync::mpsc::Receiver<AuditAction>,
    task: tokio::task::JoinHandle<()>,
    endpoint: PathBuf,
    address: String,
}
impl RequestBroker {
    pub(super) async fn start(project: &Path) -> anyhow::Result<Self> {
        use std::io::Write;
        let endpoint = project.join(".archon/audit-control-endpoint");
        std::fs::create_dir_all(endpoint.parent().unwrap())?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?.to_string();
        // Never overwrite another host's endpoint. A stale registration is a
        // visible refusal, not permission to replace another session's host.
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&endpoint)?;
        file.write_all(address.as_bytes())?;
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        let task = archon_observability::spawn_named("audit-operator-requests", async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    let length = stream.read_u32().await? as usize;
                    if length > 16384 { anyhow::bail!("audit request exceeds 16KiB"); }
                    let mut bytes = vec![0; length];
                    stream.read_exact(&mut bytes).await?;
                    let action: AuditAction = serde_json::from_slice(&bytes)?;
                    crate::command::workflow_audit_control::validate_run_id(action.run_id())?;
                    sender.try_send(action).map_err(|_| anyhow::anyhow!("operator request queue is full"))?;
                    Ok::<_, anyhow::Error>(())
                }).await;
                let reply = if matches!(result, Ok(Ok(()))) { b"queued".as_slice() } else { b"rejected".as_slice() };
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), stream.write_all(reply)).await;
            }
        });
        Ok(Self { receiver, task, endpoint, address })
    }
    pub(super) fn try_receive(&mut self) -> Option<AuditAction> { self.receiver.try_recv().ok() }
    #[cfg(test)]
    pub(super) async fn receive(&mut self) -> Option<AuditAction> { self.receiver.recv().await }
}
impl Drop for RequestBroker {
    fn drop(&mut self) {
        self.task.abort();
        if std::fs::read_to_string(&self.endpoint).ok().as_deref() == Some(&self.address) {
            let _ = std::fs::remove_file(&self.endpoint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn repository_audit_cli_request_reaches_host_without_mutating_state() {
        use archon_workflow::repository_audit::{budget::{AuditPolicy,Limit},runtime::AuditRuntime};
        let temp=tempfile::tempdir().unwrap();
        let store=archon_workflow::WorkflowStore::project(temp.path());
        let run=store.create_run(archon_workflow::WorkflowSpec{schema:archon_workflow::spec::WORKFLOW_SCHEMA.into(),name:"control".into(),task:"audit".into(),target_repository_root:None,max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![]}).unwrap();
        let audit=AuditRuntime::initialize(store,run.id.clone(),AuditPolicy{attempt_timeout_secs:Limit::Finite(10),total_time_secs:Limit::Finite(20),unexpected_change_refreshes:Limit::Finite(3)}).unwrap();
        let mut broker=RequestBroker::start(temp.path()).await.unwrap();
        let action=AuditAction::ExtendBudget{run_id:run.id,extra_refreshes:Some(2),extra_seconds:None,reason:"more source".into()};
        crate::command::workflow_audit_control::handle_cli(temp.path(),&action).await.unwrap();
        let queued=tokio::time::timeout(std::time::Duration::from_secs(2),broker.receive()).await.unwrap().unwrap();
        assert_eq!(queued.run_id(),audit.run_id);
        assert_eq!(audit.state().unwrap().budget.policy.unexpected_change_refreshes,Limit::Finite(3));
        assert!(audit.state().unwrap().operator_controls.is_empty());
    }
}
