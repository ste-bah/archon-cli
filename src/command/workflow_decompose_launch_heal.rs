//! Launch repair shares the availability loop with recorded capabilities.
use super::*;
use crate::command::workflow_host_retry;

pub(super) async fn repair(
    factory: &dyn archon_workflow::WorkflowLlmClientFactory,
    decision: &LaunchAcceptance,
    (store, run_id): (&WorkflowStore, &str),
    (project_root, prd_path, task_root, repository_root): (&Path, &Path, &Path, &Path),
    cargo_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
) -> Result<(String, serde_json::Value)> {
    let generation = store.load_state(run_id)?.generation;
    let remaining = std::cell::RefCell::new(decision.reauthor.clone());
    workflow_host_retry::retry("launch-repair", (store, run_id, generation, None), || async {
        let selected = remaining.borrow().clone();
        let client = factory.build_client(archon_workflow::WorkflowLlmClientRequest {
            cwd: project_root.to_path_buf(),
            origin: "workflow-decompose-launch-reauthor".into(),
            session_id: format!("workflow-decompose-launch-reauthor-{}", uuid::Uuid::new_v4()),
            read_roots: vec![repository_root.to_path_buf()],
        }).await.context("building the client that re-authors refused frozen checks")?;
        let probe = crate::command::workflow_task_set::executability::HostProbe::for_task_set_with_limits(
            project_root, task_root, cargo_limits.clone(),
        );
        let attempted = if selected.len() == decision.reauthor.len() {
            Box::pin(reauthor_refused(client.as_ref(), &probe, (project_root, prd_path, task_root), decision)).await
        } else {
            Box::pin(reauthor_selected(client.as_ref(), &probe, (project_root, prd_path, task_root), decision, &selected)).await
        };
        let repaired = match attempted {
            Ok(repaired) => repaired,
            Err(error) => {
                let Some(committed) = error.downcast_ref::<crate::command::workflow_task_set::republish::CommittedRepair>() else { return Err(error); };
                observe((store, run_id), || committed.settle(project_root, task_root)).await?;
                repair_report(committed.result.clone(), decision)?
            }
        };
        let pending = pending::Pending::of(&repaired.1);
        if !pending.host.is_empty() && pending.authored.is_empty() && pending.host.keys().all(|id| repaired.1["host_faults"][id] == "transient") {
            // Published siblings keep their committed identity; retry only pending entries.
            remaining.borrow_mut().retain(|id, _| pending.host.contains_key(id));
            return Err(crate::command::workflow_host_fault::transient(pending::Pending::reasons(&pending.host)));
        }
        Ok(repaired)
    }).await
}

#[cfg(test)]
thread_local! {
    pub(crate) static READ_FAILURE: std::cell::Cell<Option<std::io::ErrorKind>> = const { std::cell::Cell::new(None) };
}

type Bodies = std::collections::BTreeMap<String, Vec<u8>>;

pub(super) async fn observe<T>(
    (store, run_id): (&WorkflowStore, &str),
    mut read: impl FnMut() -> Result<T>,
) -> Result<T> {
    let generation = store.load_state(run_id)?.generation;
    workflow_host_retry::retry(
        "launch-observation",
        (store, run_id, generation, None),
        || std::future::ready(read()),
    )
    .await
}

fn contents(
    task_root: &Path,
    snapshot: &FrozenChainSnapshot,
    skeleton: bool,
) -> Result<(Option<Vec<u8>>, Bodies)> {
    if !skeleton {
        return Ok((None, Default::default()));
    }
    let path = task_root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading frozen task skeleton at {}", path.display()))?;
    Ok((Some(bytes), body_contents(task_root, snapshot)?))
}

pub(super) async fn before(
    control: (&WorkflowStore, &str),
    task_root: &Path,
    snapshot: &FrozenChainSnapshot,
) -> Result<(Option<Vec<u8>>, Bodies)> {
    observe(control, || contents(task_root, snapshot, snapshot.skeleton)).await
}

pub(super) struct Observation {
    pub(super) snapshot: FrozenChainSnapshot,
    pub(super) skeleton: Option<Vec<u8>>,
    pub(super) bodies: Bodies,
    pub(super) acceptance_digest: String,
}

pub(super) async fn after(
    control: (&WorkflowStore, &str),
    (project_root, prd_path, task_root): (&Path, &Path, &Path),
    skeleton: bool,
) -> Result<Observation> {
    observe(control, || {
        #[cfg(test)]
        if let Some(kind) = READ_FAILURE.take() {
            return Err(std::io::Error::from(kind).into());
        }
        let snapshot = frozen_chain_snapshot(project_root, prd_path, task_root)
            .context("reading the frozen chain the launch republished")?;
        let (skeleton, bodies) = contents(task_root, &snapshot, skeleton)?;
        let path = task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
        let bytes = std::fs::read(&path).with_context(|| {
            format!(
                "reading republished acceptance contract at {}",
                path.display()
            )
        })?;
        Ok(Observation {
            snapshot,
            skeleton,
            bodies,
            acceptance_digest: archon_workflow::task_set_contract::content_digest(&bytes),
        })
    })
    .await
}

#[cfg(test)]
#[path = "workflow_decompose_launch_partial_heal_tests.rs"]
mod tests;
