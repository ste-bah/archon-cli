//! The check policy a fixed decomposition's launch bound (Issue 349).
//!
//! A launch records `check_environment_policy` in its generated metadata and
//! binds that value into the anchored launch digest. A binary built before
//! that binding recorded neither: its metadata has no key, and its anchor
//! digests only the identity, arguments, catalog and route. Such a run still
//! resumes, and it resumes with no recorded check policy (`None`). That is
//! the value the run-time reader `policy_for_run` gives a run whose metadata
//! has no key and no observer snapshot, so every check site uses the default
//! check policy. Metadata without the key whose anchor is not that older form
//! lost the key after its launch, and is refused.

use anyhow::{Result, anyhow};
use archon_workflow::FixedRunIdentityV1;
use archon_workflow::acceptance_check_environment::CheckPolicy;

use crate::command::workflow_provider_route::TrustedProviderRouteSnapshot;

pub(crate) const CHECK_POLICY_KEY: &str = "check_environment_policy";

/// The launch metadata a fixed run must carry, apart from its policy key.
pub(crate) fn canonical_metadata(
    identity: &FixedRunIdentityV1,
    scaffold_hash: String,
    expected_arguments: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "workflow-generated-v2-metadata-v1",
        "run_kind": "fixed_decomposition_v1",
        "fixed_identity": identity,
        "scaffold_hash": scaffold_hash,
        "script_args": expected_arguments,
        "script_lifecycle": true,
    })
}

/// The launch's check policy, admitted against the verified bundle anchor.
pub(crate) fn admit(
    metadata: &serde_json::Value,
    mut expected: serde_json::Value,
    identity: &FixedRunIdentityV1,
    arguments: &serde_json::Value,
    catalog: &archon_workflow::CommandCapabilityCatalog,
    route: &TrustedProviderRouteSnapshot,
    anchored_digest: &str,
) -> Result<Option<CheckPolicy>> {
    let differs = || {
        anyhow!("fixed decomposition generated metadata differs from its canonical launch snapshot")
    };
    let Some(recorded) = metadata.get(CHECK_POLICY_KEY) else {
        if metadata != &expected {
            return Err(differs());
        }
        if pre_binding_digest(identity, arguments, catalog, route)? != anchored_digest {
            return Err(anyhow!(
                "fixed decomposition launch metadata has no check policy binding, and its launch digest anchor is not the pre-binding form, so the binding was removed after launch; relaunch the workflow"
            ));
        }
        return Ok(None);
    };
    let check_policy: Option<CheckPolicy> = serde_json::from_value(recorded.clone())
        .map_err(|error| anyhow!("fixed decomposition check policy binding is invalid: {error}"))?;
    if let Some(policy) = &check_policy {
        crate::command::acceptance_check_policy::validate_persisted(policy)?;
    }
    expected[CHECK_POLICY_KEY] = serde_json::to_value(&check_policy)?;
    if metadata != &expected {
        return Err(differs());
    }
    let digest = crate::command::workflow_decompose::fixed_launch_digest(
        identity,
        arguments,
        catalog,
        route,
        &check_policy,
    )?;
    if digest != anchored_digest {
        return Err(anyhow!(
            "fixed decomposition launch snapshot differs from the verified workflow bundle anchor"
        ));
    }
    Ok(check_policy)
}

/// The launch digest a binary without the check-policy binding anchored.
pub(crate) fn pre_binding_digest(
    identity: &FixedRunIdentityV1,
    arguments: &serde_json::Value,
    catalog: &archon_workflow::CommandCapabilityCatalog,
    route: &TrustedProviderRouteSnapshot,
) -> Result<String> {
    let bytes = serde_json::to_vec(&(identity, arguments, catalog, route))?;
    Ok(archon_workflow::task_set_contract::content_digest(&bytes))
}

#[cfg(test)]
#[path = "workflow_decompose_resume_policy_tests.rs"]
mod tests;
