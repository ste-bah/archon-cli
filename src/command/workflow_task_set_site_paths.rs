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
        if !finding
            .identity
            .as_ref()
            .is_some_and(|identity| identity.code == "unavailable_site_path")
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

pub(crate) fn acceptance_pin_path(project_root: &Path, tasks_root: &Path) -> PathBuf {
    let canonical = tasks_root
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap_or_else(|_| tasks_root.to_path_buf());
    let key = content_digest(canonical.to_string_lossy().as_bytes());
    archon_workflow::task_set_lineage::pin_store_dir(project_root).join(format!("{key}.json"))
}

pub(super) fn project_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
