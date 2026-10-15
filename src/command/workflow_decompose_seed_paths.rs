use super::*;

/// The script, catalog and template a seed is for: what a binary revision
/// drift alone leaves unchanged.
pub(crate) fn runtime_key(runtime: &FixedRunIdentityV1) -> String {
    let harness = format!(
        "{}\n{}\n{}",
        runtime.template_version, runtime.script_digest, runtime.catalog_digest
    );
    archon_workflow::task_set_contract::content_digest(harness.as_bytes())[..16].to_string()
}

pub(crate) fn seed_path(index: usize, runtime: &FixedRunIdentityV1) -> String {
    format!(
        "{SEEDS_DIR}/transition-{index}-{}.json",
        runtime_key(runtime)
    )
}
