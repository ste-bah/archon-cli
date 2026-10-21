use super::*;

/// Git's tree object commits to every path, mode, and file blob the check
/// can read inside its repository-relative bounded closure.
fn repository_tree_digest(repository: &Path, commit: &str, path: &str) -> Option<String> {
    let entries = repository_tree_entries(repository, commit, path)?;
    let mut closure = Vec::from(path.as_bytes());
    for entry in &entries {
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

fn repository_tree_entries(repository: &Path, commit: &str, path: &str) -> Option<Vec<Vec<u8>>> {
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-tree", "-r", "-t", "-z", commit, "--", path])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| entry.to_vec())
            .collect(),
    )
}

/// Whether `path` is a regular file in the exact committed tree a probe uses.
pub(crate) fn repository_tree_contains(repository: &Path, commit: &str, path: &str) -> bool {
    let Some(entries) = repository_tree_entries(repository, commit, path) else {
        return false;
    };
    entries.iter().any(|entry| {
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            return false;
        };
        let Ok(found) = std::str::from_utf8(&entry[tab + 1..]) else {
            return false;
        };
        let mode = entry[..tab]
            .split(|byte| *byte == b' ')
            .next()
            .unwrap_or_default();
        found == path && matches!(mode, b"100644" | b"100755")
    })
}

/// The memo key of `reference` on `tree` with project data in `data`: the
/// project data is part of the tree a verdict is evidence of, so data that
/// changed since makes an observed verdict no evidence at all.
pub(super) fn memo_key(
    probe: &HostProbe,
    tree: &Baseline,
    data: &[String],
    contract: &AcceptanceContract,
    id: &str,
) -> Option<String> {
    let entry = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id)?;
    let key = bounded_memo_key(probe, tree, data, entry)
        .or_else(|| full_closure_memo_key(probe, tree, data, entry))?;
    let siblings = sibling_input_state(probe, tree, entry)?;
    Some(content_digest(
        serde_json::json!([key, siblings]).to_string().as_bytes(),
    ))
}

/// Sibling files sit outside the project data closure. Bind their copied
/// revision to the verdict key so a retry reuses the same snapshot, while a
/// changed sibling gets a fresh observation.
fn sibling_input_state(
    probe: &HostProbe,
    tree: &Baseline,
    entry: &archon_workflow::task_set_contract::AcceptanceCriterion,
) -> Option<Vec<(PathBuf, String)>> {
    let AcceptanceCheck::Command { command, cwd } = &entry.check else {
        return Some(Vec::new());
    };
    let root = match (&probe.site, cwd) {
        (Site::Scratch(binding), archon_workflow::task_set_contract::TrustedCwd::ProjectRoot) => {
            binding.policy.project.as_path()
        }
        (Site::Scratch(binding), archon_workflow::task_set_contract::TrustedCwd::RepoRoot) => {
            binding.policy.repository.as_path()
        }
        (_, archon_workflow::task_set_contract::TrustedCwd::ProjectRoot) => probe.project.as_path(),
        (_, archon_workflow::task_set_contract::TrustedCwd::RepoRoot) => tree.repository.as_path(),
    };
    archon_workflow::acceptance_sibling_paths::discover(root, std::slice::from_ref(command))
        .into_iter()
        .map(|sibling| {
            let revision = if sibling.source.join(".git").exists() {
                let output = archon_shell::spawn::command("git")
                    .arg("-C")
                    .arg(&sibling.source)
                    .args(["rev-parse", "HEAD"])
                    .output()
                    .ok()?;
                output
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())?
            } else {
                sibling_content_digest(&sibling.source)?
            };
            Some((sibling.alias, revision))
        })
        .collect()
}

fn sibling_content_digest(root: &Path) -> Option<String> {
    fn visit(root: &Path, path: &Path, bytes: &mut Vec<u8>) -> Option<()> {
        let mut entries = std::fs::read_dir(path)
            .ok()?
            .map(|entry| entry.ok())
            .collect::<Option<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            if [".git", "target", "node_modules"]
                .iter()
                .any(|excluded| name == *excluded)
            {
                continue;
            }
            let child = entry.path();
            let metadata = std::fs::symlink_metadata(&child).ok()?;
            bytes.extend_from_slice(child.strip_prefix(root).ok()?.to_string_lossy().as_bytes());
            bytes.push(0);
            if metadata.is_dir() {
                bytes.push(b'd');
                visit(root, &child, bytes)?;
            } else if metadata.is_file() {
                bytes.push(b'f');
                bytes.extend_from_slice(&std::fs::read(&child).ok()?);
                bytes.push(0);
            } else if metadata.file_type().is_symlink() {
                bytes.push(b'l');
                bytes.extend_from_slice(
                    std::fs::read_link(&child)
                        .ok()?
                        .to_string_lossy()
                        .as_bytes(),
                );
                bytes.push(0);
            }
        }
        Some(())
    }

    let mut bytes = Vec::new();
    visit(root, root, &mut bytes)?;
    Some(content_digest(&bytes))
}

/// Checks with a literal repository path can reuse across commits when that
/// path's complete tree is unchanged.
fn bounded_memo_key(
    probe: &HostProbe,
    tree: &Baseline,
    data: &[String],
    entry: &archon_workflow::task_set_contract::AcceptanceCriterion,
) -> Option<String> {
    let site = match &probe.site {
        Site::Scratch(binding) => serde_json::to_string(&binding.policy).ok()?,
        Site::Direct | Site::Hermetic | Site::Unavailable(_) => "hermetic".to_string(),
    };
    let (environment, _) = site_environment(probe);
    let shell_path = environment.get("PATH").cloned();
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
        data,
        tree_digest,
        entry.id,
        entry.criterion.as_bytes(),
        entry.covers,
        command.as_bytes(),
        logic,
        logic_digest,
        build,
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::shell_binary_digest(
            shell_path.as_deref(),
        )?,
        environment,
    ]);
    Some(content_digest(key.to_string().as_bytes()))
}

/// The original freeze memo key: every check reuses only for the same site,
/// runtime, repository commit, project, data, and exact check definition.
fn full_closure_memo_key(
    probe: &HostProbe,
    tree: &Baseline,
    data: &[String],
    entry: &archon_workflow::task_set_contract::AcceptanceCriterion,
) -> Option<String> {
    let site = match &probe.site {
        Site::Scratch(binding) => serde_json::to_string(&binding.policy).ok()?,
        Site::Direct | Site::Hermetic | Site::Unavailable(_) => "hermetic".to_string(),
    };
    let key = serde_json::json!([
        site,
        runtime_identity(probe),
        tree.repository,
        probe.project,
        tree.commit,
        data,
        entry.id,
        entry.check,
    ]);
    Some(content_digest(key.to_string().as_bytes()))
}

/// The reuse guarantee represented by the key selected for this check.
pub(super) fn memo_reason(
    probe: &HostProbe,
    tree: &Baseline,
    data: &[String],
    entry: &archon_workflow::task_set_contract::AcceptanceCriterion,
) -> &'static str {
    match &entry.check {
        AcceptanceCheck::Command {
            command,
            cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
        } if bounded_memo_key(probe, tree, data, entry).is_some() => {
            "all inputs are host-proven identical"
        }
        _ => "full input closure: same commit and project data",
    }
}

/// The memo key of check `id` of `contract` on `tree`, with the project
/// data as it is now.
pub(crate) fn check_key(
    probe: &HostProbe,
    tree: &Baseline,
    contract: &AcceptanceContract,
    id: &str,
) -> Option<String> {
    memo_key(probe, tree, &data_state(probe, tree).ok()?, contract, id)
}
