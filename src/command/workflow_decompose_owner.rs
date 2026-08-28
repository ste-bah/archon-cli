//! Host-owned retained-session evidence for interactive fixed decomposition.

use std::collections::BTreeSet;

use archon_workflow::{WorkflowError, WorkflowResult, WorkflowStore};

pub(crate) const FIXED_INTERACTIVE_OWNER_PATH: &str = "decomposition/interactive-owner.json";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FixedInteractiveOwnerRecord {
    pub(crate) schema_version: u32,
    pub(crate) owner_identity: String,
    pub(crate) actions: BTreeSet<String>,
}

pub(crate) fn initialize(
    store: &WorkflowStore,
    run_id: &str,
    owner_identity: &str,
) -> WorkflowResult<()> {
    validate_owner(owner_identity)?;
    let record = FixedInteractiveOwnerRecord {
        schema_version: 1,
        owner_identity: owner_identity.to_string(),
        actions: BTreeSet::from(["launch".to_string()]),
    };
    store.write_run_json(run_id, FIXED_INTERACTIVE_OWNER_PATH, &record)
}

pub(crate) fn require_owner(
    store: &WorkflowStore,
    run_id: &str,
    owner_identity: Option<&str>,
) -> WorkflowResult<()> {
    let Some(record) = read(store, run_id)? else {
        return if owner_identity.is_none() {
            Ok(())
        } else {
            Err(WorkflowError::StateCorrupt(
                "fixed run has no retained interactive owner record".into(),
            ))
        };
    };
    let supplied = owner_identity.ok_or_else(|| {
        WorkflowError::PolicyDenied(
            "fixed run is owned by a retained interactive session; use that session".into(),
        )
    })?;
    validate_owner(supplied)?;
    if record.schema_version != 1 || record.owner_identity != supplied {
        return Err(WorkflowError::PolicyDenied(
            "fixed run retained-owner identity differs from this interactive session".into(),
        ));
    }
    Ok(())
}

pub(crate) fn record_action(
    store: &WorkflowStore,
    run_id: &str,
    owner_identity: Option<&str>,
    action: &str,
) -> WorkflowResult<()> {
    if !matches!(action, "pause" | "resume" | "status" | "cancel") {
        return Err(WorkflowError::SpecInvalid(format!(
            "unknown fixed interactive owner action '{action}'"
        )));
    }
    require_owner(store, run_id, owner_identity)?;
    if owner_identity.is_none() {
        return Ok(());
    }
    store.with_run_lock(run_id, |locked| {
        let path = locked.run_dir(run_id).join(FIXED_INTERACTIVE_OWNER_PATH);
        let mut record: FixedInteractiveOwnerRecord =
            serde_json::from_slice(&std::fs::read(&path).map_err(|source| WorkflowError::Io {
                path: path.clone(),
                source,
            })?)?;
        record.actions.insert(action.to_string());
        locked.write_run_json(run_id, FIXED_INTERACTIVE_OWNER_PATH, &record)
    })
}

pub(crate) fn read(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<Option<FixedInteractiveOwnerRecord>> {
    let path = store.run_dir(run_id).join(FIXED_INTERACTIVE_OWNER_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    serde_json::from_slice(&bytes).map(Some).map_err(Into::into)
}

fn validate_owner(owner_identity: &str) -> WorkflowResult<()> {
    if owner_identity.len() < 16
        || owner_identity.len() > 128
        || !owner_identity
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(WorkflowError::SpecInvalid(
            "fixed interactive owner identity is malformed".into(),
        ));
    }
    Ok(())
}
