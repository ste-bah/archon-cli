//! Launch-bound fixed decomposition arguments and their binding digest.

use std::collections::BTreeMap;
use std::path::Path;

/// Launch-bound script arguments; resume rebuilds and compares the persisted
/// copy. The frozen chain is carried forward verbatim on resume.
#[allow(clippy::too_many_arguments)]
pub(crate) fn fixed_script_arguments(
    project_root: &Path,
    prd_path: &Path,
    prd_digest: &str,
    acceptance_criteria: BTreeMap<String, String>,
    config: &archon_core::config::ArchonConfig,
    task_root: &Path,
    repository_root: &Path,
    frozen_chain: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "projectRoot": super::path_text(project_root),
        "repositoryRoot": super::path_text(repository_root),
        "prdPath": super::path_text(prd_path),
        "prdDigest": prd_digest,
        "acceptanceCriteria": acceptance_criteria,
        "authorMaxParallelism": config.subagent.max_concurrent.max(1),
        "taskRoot": super::path_text(task_root),
        "gateMode": super::gate_mode_text(config.workflow.gate_mode),
        "excludedDirs": archon_leann::language::default_exclude_patterns(),
        "frozenChain": frozen_chain,
    })
}

pub(crate) fn fixed_launch_digest(
    identity: &archon_workflow::FixedRunIdentityV1,
    arguments: &serde_json::Value,
    catalog: &archon_workflow::CommandCapabilityCatalog,
    route: &super::super::workflow_provider_route::TrustedProviderRouteSnapshot,
    check_policy: &Option<archon_workflow::acceptance_check_environment::CheckPolicy>,
) -> anyhow::Result<String> {
    let bytes = serde_json::to_vec(&(identity, arguments, catalog, route, check_policy))?;
    Ok(archon_workflow::task_set_contract::content_digest(&bytes))
}
