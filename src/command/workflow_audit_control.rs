//! CLI inspection is read-only. Mutation requires the interactive host channel.
use crate::cli_args::workflow_audit::AuditAction;
use archon_workflow::{WorkflowStore, WorkflowV2ResultStore};
use std::path::Path;

pub(crate) async fn handle_cli(project: &Path, action: &AuditAction) -> anyhow::Result<()> {
    let run_id = action.run_id();
    validate_run_id(run_id)?;
    let store = WorkflowStore::project(project);
    let state = read_state(&store, run_id)?;
    match action {
        AuditAction::Status { .. } => {
            println!("{}", serde_json::to_string_pretty(&state.status()?)?)
        }
        _ => {
            submit_request(project, action).await?;
            println!(
                "Audit request queued for interactive host confirmation; no mutation applied."
            );
        }
    }
    Ok(())
}

async fn submit_request(project: &Path, action: &AuditAction) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let address: std::net::SocketAddr = std::fs::read_to_string(
        project.join(".archon/audit-control-endpoint"),
    )
    .map_err(|_| {
        anyhow::anyhow!(
            "no interactive audit control host; open the project in interactive archon and retry"
        )
    })?
    .parse()?;
    if !address.ip().is_loopback() {
        anyhow::bail!("audit control endpoint is not local");
    }
    let body = serde_json::to_vec(action)?;
    if body.len() > 16384 {
        anyhow::bail!("audit request exceeds 16KiB");
    }
    tokio::time::timeout(std::time::Duration::from_secs(4), async {
        let mut stream = tokio::net::TcpStream::connect(address).await?;
        stream.write_u32(body.len() as u32).await?;
        stream.write_all(&body).await?;
        let mut reply = Vec::new();
        stream.take(64).read_to_end(&mut reply).await?;
        if reply != b"queued" {
            anyhow::bail!("interactive host rejected audit request; no mutation applied");
        }
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}

pub(crate) fn validate_run_id(run_id: &str) -> archon_workflow::WorkflowResult<()> {
    if run_id.is_empty()
        || run_id.len() > 128
        || !run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(archon_workflow::WorkflowError::PolicyDenied(
            "invalid audit run ID".into(),
        ));
    }
    Ok(())
}
pub(crate) fn read_state(
    store: &WorkflowStore,
    run_id: &str,
) -> archon_workflow::WorkflowResult<archon_workflow::repository_audit::runtime::AuditState> {
    validate_run_id(run_id)?;
    let v2 = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    archon_workflow::repository_audit::reuse::load_state(&v2)?.ok_or_else(|| {
        archon_workflow::WorkflowError::StateCorrupt("run has no repository audit".into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::repository_audit::{
        budget::{AuditPolicy, Limit},
        runtime::AuditRuntime,
    };

    #[tokio::test]
    async fn repository_audit_cli_mutation_without_host_confirmation_changes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store
            .create_run(archon_workflow::WorkflowSpec {
                schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
                name: "operator".into(),
                task: "audit".into(),
                target_repository_root: None,
                max_agents: 1,
                max_parallelism: 1,
                stages: vec![],
                permissions: Default::default(),
                learning_hooks: vec![],
            })
            .unwrap();
        let audit = AuditRuntime::initialize(
            store.clone(),
            run.id.clone(),
            AuditPolicy {
                attempt_timeout_secs: Limit::Finite(10),
                total_time_secs: Limit::Finite(20),
                unexpected_change_refreshes: Limit::Finite(3),
            },
        )
        .unwrap();
        let before = std::fs::read(
            store
                .run_dir(&run.id)
                .join(archon_workflow::repository_audit::runtime::STATE_PATH),
        )
        .unwrap();
        let action = AuditAction::ExtendBudget {
            run_id: run.id.clone(),
            extra_refreshes: Some(2),
            extra_seconds: None,
            reason: "more time".into(),
        };
        assert!(handle_cli(temp.path(), &action).await.is_err());
        assert_eq!(
            before,
            std::fs::read(
                store
                    .run_dir(&run.id)
                    .join(archon_workflow::repository_audit::runtime::STATE_PATH)
            )
            .unwrap()
        );
        assert_eq!(
            audit
                .state()
                .unwrap()
                .budget
                .policy
                .unexpected_change_refreshes,
            Limit::Finite(3)
        );
    }
}
