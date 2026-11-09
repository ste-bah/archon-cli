use super::*;

/// Issue 495: the site path rule the first author and every publication
/// apply, held to a contract already published. Each refused check id maps
/// to the rule's own finding text.
pub(crate) fn site_path_refusals(
    contract: &AcceptanceContract,
    project_root: &Path,
    tasks_root: &Path,
) -> Result<std::collections::BTreeMap<String, String>> {
    let mut refusals = std::collections::BTreeMap::<String, String>::new();
    for finding in preflight::rooted_policy_findings(contract, project_root, tasks_root)? {
        if finding
            .identity
            .as_ref()
            .is_none_or(|identity| identity.code != "unavailable_site_path")
        {
            continue;
        }
        let id = finding
            .field
            .split('.')
            .next()
            .unwrap_or_default()
            .to_string();
        refusals
            .entry(id)
            .and_modify(|text| {
                text.push_str("; ");
                text.push_str(&finding.message);
            })
            .or_insert(finding.message);
    }
    Ok(refusals)
}
