//! Add provider identity without replacing the immutable, launch-bound snapshot.
use super::TrustedProviderRouteSnapshot;
use anyhow::{Result, bail};
use archon_workflow::WorkflowStore;

const UPGRADE_PATH: &str = "decomposition/provider-route-upgrade.json";
const NOTE: &str =
    "Legacy provider route verified; provider bindings and model choices added on resume";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteUpgrade {
    note: String,
    route: TrustedProviderRouteSnapshot,
}

fn compare(
    saved: &TrustedProviderRouteSnapshot,
    current: &TrustedProviderRouteSnapshot,
) -> Result<()> {
    let mut expected = current.clone();
    if saved.providers.is_empty() {
        expected.providers.clear();
        expected.role_models.clear();
        expected.tier_aliases.clear();
    }
    if saved != &expected {
        bail!(
            "fixed decomposition resume paused: provider route differs from the verified snapshot; restore the launch-bound provider configuration"
        );
    }
    Ok(())
}

fn read_upgrade(store: &WorkflowStore, run_id: &str) -> Result<Option<RouteUpgrade>> {
    let path = store.run_dir(run_id).join(UPGRADE_PATH);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let bytes = archon_workflow::v2::store_file::read_store_file(&path)?;
    let record: RouteUpgrade = serde_json::from_slice(&bytes)?;
    if record.note != NOTE || record.route.providers.is_empty() {
        bail!("invalid provider route upgrade record: {}", path.display());
    }
    Ok(Some(record))
}

/// Used by live resume and dry run; never writes during admission.
pub(crate) fn validate_resume(
    store: &WorkflowStore,
    run_id: &str,
    saved: &TrustedProviderRouteSnapshot,
    current: &TrustedProviderRouteSnapshot,
) -> Result<()> {
    compare(saved, current)?;
    if let Some(record) = read_upgrade(store, run_id)? {
        compare(&record.route, current)?;
    }
    Ok(())
}

/// Record the explicit upgrade only after all immutable launch inputs pass admission.
pub(crate) fn record_upgrade(
    store: &WorkflowStore,
    run_id: &str,
    saved: &TrustedProviderRouteSnapshot,
    current: &TrustedProviderRouteSnapshot,
) -> Result<()> {
    validate_resume(store, run_id, saved, current)?;
    if saved.providers.is_empty() && read_upgrade(store, run_id)?.is_none() {
        store.write_run_json(
            run_id,
            UPGRADE_PATH,
            &RouteUpgrade {
                note: NOTE.into(),
                route: current.clone(),
            },
        )?;
        crate::command::workflow_task_set::sync_file(&store.run_dir(run_id).join(UPGRADE_PATH))?;
        crate::command::workflow_task_set::sync_parent(&store.run_dir(run_id).join(UPGRADE_PATH))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::workflow_provider_route::{
        ProviderEndpointPolicy, resolve_provider_routes,
    };

    #[test]
    fn legacy_endpoint_snapshot_upgrades_once_and_then_binds_new_fields() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let mut config = archon_core::config::ArchonConfig::default();
        config.api.base_url = Some("http://127.0.0.1:1234/v1/messages".into());
        let current = resolve_provider_routes(&config, ProviderEndpointPolicy::ConfiguredOnly);
        let legacy_json = serde_json::json!({
            "origin": "trusted_config",
            "endpoint": "http://127.0.0.1:1234/v1/messages",
            "endpoint_digest": archon_workflow::task_set_contract::content_digest(
                b"http://127.0.0.1:1234/v1/messages"),
        });
        let legacy = serde_json::from_value(legacy_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&legacy).unwrap(), legacy_json);
        validate_resume(&store, "legacy", &legacy, &current).unwrap();
        assert!(read_upgrade(&store, "legacy").unwrap().is_none());
        record_upgrade(&store, "legacy", &legacy, &current).unwrap();
        record_upgrade(&store, "legacy", &legacy, &current).unwrap();
        assert_eq!(read_upgrade(&store, "legacy").unwrap().unwrap().note, NOTE);
        config
            .llm
            .tier_aliases
            .insert("opus".into(), "sonnet".into());
        let changed = resolve_provider_routes(&config, ProviderEndpointPolicy::ConfiguredOnly);
        assert!(validate_resume(&store, "legacy", &legacy, &changed).is_err());
        let mut wrong_endpoint = current.clone();
        wrong_endpoint.endpoint = Some("http://127.0.0.1:9999".into());
        assert!(validate_resume(&store, "fresh", &legacy, &wrong_endpoint).is_err());
    }
}
