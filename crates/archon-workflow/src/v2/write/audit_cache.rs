//! Latch current source before deciding whether a stored branch grants credit.
use super::*;
use crate::repository_audit::runtime::Snapshot;

pub(super) async fn refresh(
    branches: &mut [WorkflowV2FanoutItem],
    items: &[WorkflowV2WriteItem],
    root: Option<&str>,
    call_id: &str,
    store: &WorkflowV2ResultStore,
    dispatch: &dyn WorkflowAgentDispatch,
) -> WorkflowResult<()> {
    let Some(audit) = dispatch.repository_audit() else {
        return Ok(());
    };
    let mut has_cached_candidate = false;
    for branch in branches {
        if let Some(item) = items.iter().find(|item| item.id == branch.id) {
            branch.call.options.target_files = item.owned_targets.clone();
        }
        has_cached_candidate |= store.load_branch_outcome(call_id, &branch.id)?.is_some();
    }
    if !has_cached_candidate {
        return Ok(());
    }
    let Some(root) = root else {
        return Ok(());
    };
    let mut paths = audit.state()?.declared_paths;
    paths.extend(
        items
            .iter()
            .flat_map(|item| item.owned_targets.iter().cloned()),
    );
    let paths = paths.into_iter().collect::<Vec<_>>();
    let snapshot = Snapshot::capture(Path::new(root), &paths, store)?;
    audit.assess(&snapshot, &paths, "cache", dispatch).await
}
