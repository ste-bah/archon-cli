use super::*;

/// Git's tree object commits to every path, mode, and file blob the check
/// can read inside its repository-relative bounded closure.
fn repository_tree_digest(repository: &Path, commit: &str, path: &str) -> Option<String> {
    let entries = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-tree", "-r", "-t", "-z", commit, "--", path])
        .output()
        .ok()?;
    if !entries.status.success() {
        return None;
    }
    let mut closure = Vec::from(path.as_bytes());
    for entry in entries.stdout.split(|byte| *byte == 0) {
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            continue;
        };
        let metadata = &entry[..tab];
        let found = std::str::from_utf8(&entry[tab + 1..]).ok()?;
        let mode = metadata
            .split(|byte| *byte == b' ')
            .next()
            .unwrap_or_default();
        if mode == b"120000"
            && (path == found
                || path
                    .strip_prefix(found)
                    .is_some_and(|rest| rest.starts_with('/')))
        {
            return None;
        }
        closure.extend_from_slice(entry);
        closure.push(0);
    }
    Some(content_digest(&closure))
}

/// The memo key of `reference` on `tree` with project data in `data`: the
/// project data is part of the tree a verdict is evidence of, so data that
/// changed since makes an observed verdict no evidence at all.
pub(super) fn memo_key(
    probe: &HostProbe,
    tree: &Baseline,
    _data: &[String],
    contract: &AcceptanceContract,
    id: &str,
) -> Option<String> {
    let entry = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id)?;
    let site = match &probe.site {
        Site::Scratch(binding) => serde_json::to_string(&binding.policy).ok()?,
        Site::Direct | Site::Hermetic | Site::Unavailable(_) => "hermetic".to_string(),
    };
    let (environment, _) = site_environment(probe);
    let environment = serde_json::to_string(&environment).ok()?;
    let AcceptanceCheck::Command {
        command,
        cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
    } = &entry.check
    else {
        return None;
    };
    let path =
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::bounded_path(command)?;
    let tree_digest = repository_tree_digest(&tree.repository, &tree.commit, path)?;
    let (logic, logic_digest, build) =
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity()?;
    let assessment = crate::command::workflow_task_set::workflow_acceptance_check_reuse::assess(
        command,
        &tree_digest,
        &logic.to_string(),
        &environment,
    );
    if !assessment.reusable {
        return None;
    }
    let check_closure_key =
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::reuse_key(
            command,
            &tree_digest,
            &logic.to_string(),
            &environment,
        );
    let key = serde_json::json!([
        check_closure_key,
        site,
        runtime_identity(probe),
        tree.repository,
        probe.project,
        tree_digest,
        entry.id,
        entry.criterion.as_bytes(),
        entry.covers,
        command.as_bytes(),
        logic,
        logic_digest,
        build,
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::shell_binary_digest()?,
        environment,
    ]);
    Some(content_digest(key.to_string().as_bytes()))
}

/// The memo key of check `id` of `contract` on `tree`, with the project
/// data as it is now.
pub(crate) fn check_key(
    probe: &HostProbe,
    tree: &Baseline,
    contract: &AcceptanceContract,
    id: &str,
) -> Option<String> {
    memo_key(probe, tree, &data_state(probe, tree), contract, id)
}
