//! Read-only identity of the embedded fixed decomposition runtime.

use anyhow::Result;

pub(crate) fn fixed_decomposition_identity() -> Result<serde_json::Value> {
    let binary_revision = env!("ARCHON_GIT_HASH");
    let catalog = crate::command::workflow_host_command_catalog::fixed_decomposition_catalog(
        binary_revision,
    )?;
    Ok(serde_json::json!({
        "template_version": archon_workflow::FIXED_DECOMPOSITION_TEMPLATE_VERSION,
        "binary_revision": binary_revision,
        "script_digest": archon_workflow::workflow_scaffold_hash(
            crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE,
        ),
        "catalog_digest": catalog.digest,
    }))
}
