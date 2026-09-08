//! Cache admission uses current host assessment, not the cached agent verdict.
use super::runtime::{AuditState, STATE_PATH};
use crate::{WorkflowError, WorkflowResult, WorkflowV2ResultStore};

pub fn load_state(store: &WorkflowV2ResultStore) -> WorkflowResult<Option<AuditState>> {
    let run_root = store.root().parent().ok_or_else(||
        WorkflowError::StateCorrupt("audit run root missing".into()))?;
    let path = run_root.join(STATE_PATH);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let state: AuditState = serde_json::from_slice(&bytes)?;
            if state.schema_version != 1 {
                return Err(WorkflowError::StateCorrupt("unsupported audit state schema".into()));
            }
            Ok(Some(state))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if store.root().join("repository-audit/required.json").exists() {
                Err(WorkflowError::StateCorrupt("mandatory repository audit state is missing".into()))
            } else {
                Ok(None)
            }
        }
        Err(error) => Err(WorkflowError::io(path, error)),
    }
}

/// The caller latches/assesses the boundary snapshot before consulting this.
/// Missing coverage is a cache miss, never permission to omit an assessment.
pub fn eligible(state: &AuditState, paths: &[String]) -> WorkflowResult<bool> {
    if state.last_error.is_some() || state.budget.active.is_some() || paths.is_empty() {
        return Ok(false);
    }
    let Some(snapshot) = &state.snapshot else { return Ok(false); };
    let Some(report) = state.ledger.history.last() else { return Ok(false); };
    if report.snapshot != snapshot.identity { return Ok(false); }
    let open = state.ledger.unresolved(&snapshot.identity)?;
    Ok(paths.iter().all(|path| state.declared_paths.contains(path)
        && report.records.iter().any(|record| &record.declared_path == path)
        && !open.contains(path)))
}
