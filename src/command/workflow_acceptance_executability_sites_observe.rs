//! Run a Scratch check copy and retain the sibling revisions it materialized.

use super::*;

/// The scratch observation the acceptance stage's guardian performs, run
/// in-process over the unpublished candidate contract, at the target
/// repository's HEAD, under the same repository lease. Its own task, so a
/// synchronous copy phase never blocks the caller's run-control race; its
/// evidence is removed afterwards (a crash reaches the author as a finding).
pub(super) async fn observe(
    binding: NativeBinding,
    commit: Option<String>,
    contract: AcceptanceContract,
    digest: String,
    refs: Vec<FrozenCommandRef>,
    cancel: Arc<AtomicBool>,
    hooks: ObserveHooks,
) -> anyhow::Result<(
    Vec<CheckResult>,
    Vec<super::hermetic::SiblingRevision>,
    BTreeMap<String, Vec<super::hermetic::SiblingRevision>>,
    std::path::PathBuf,
)> {
    let identity = archon_shell::paths::canonicalize(&binding.policy.repository)?;
    let _lease = crate::command::acceptance_scratch_guardian::acquire_lease(
        &std::env::temp_dir().join("archon-native-observer-locks"),
        &identity.to_string_lossy(),
    )?;
    let head = match commit {
        Some(commit) => commit,
        None => super::git_head(&binding.policy.repository)
            .ok_or_else(|| anyhow::anyhow!("cannot read the repository HEAD"))?,
    };
    let evidence = binding
        .policy
        .scratch_parent
        .join(format!("acceptance-probe-{}", uuid::Uuid::new_v4()));
    let observed = observe_commands_hooked(
        &binding.policy,
        &head,
        &contract,
        &digest,
        &refs,
        &evidence,
        cancel,
        &hooks,
    )
    .await;
    let _ = std::fs::remove_dir_all(&evidence);
    let observed = observed?;
    if !observed.operational_errors.is_empty()
        || !observed.teardown_verified
        || !observed.live_roots_unchanged
    {
        anyhow::bail!(
            "scratch observation void: {}",
            observed.operational_errors.join("; ")
        );
    }
    let entries = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let cwd_key = |cwd| match cwd {
        archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => 0_u8,
        archon_workflow::task_set_contract::TrustedCwd::RepoRoot => 1_u8,
    };
    let mut sources = BTreeMap::new();
    for entry in contract.acceptance.iter().chain(&contract.supplementary) {
        if let AcceptanceCheck::Command { command, cwd } = &entry.check {
            let root = match cwd {
                archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => {
                    &binding.policy.project
                }
                archon_workflow::task_set_contract::TrustedCwd::RepoRoot => {
                    &binding.policy.repository
                }
            };
            for sibling in archon_workflow::acceptance_sibling_paths::discover(
                root,
                std::slice::from_ref(command),
            ) {
                sources.insert(
                    (cwd_key(*cwd), relative_path(root, &sibling.alias)),
                    sibling.source,
                );
            }
        }
    }
    let mut siblings = BTreeMap::<PathBuf, super::hermetic::SiblingRevision>::new();
    for evidence in &observed.check_evidence {
        let Some(entry) = entries.get(evidence.acceptance_id.as_str()) else {
            continue;
        };
        let AcceptanceCheck::Command { cwd, .. } = &entry.check else {
            continue;
        };
        for sibling in &evidence.sibling_revisions {
            let root_kind = sibling.cwd.unwrap_or(*cwd);
            let source = sources
                .get(&(cwd_key(root_kind), sibling.path.clone()))
                .ok_or_else(|| anyhow::anyhow!("scratch sibling source disappeared"))?;
            let sibling = super::hermetic::SiblingRevision {
                path: sibling.path.clone(),
                commit: sibling.commit.clone(),
                target: None,
            };
            if let Some(previous) = siblings.get(source)
                && previous.commit != sibling.commit
            {
                anyhow::bail!(
                    "scratch checks observed conflicting revisions for sibling {} ({previous:?} and {:?})",
                    sibling.path.display(),
                    sibling.commit
                );
            }
            siblings.entry(source.clone()).or_insert(sibling);
        }
    }
    let all_siblings = siblings;
    let mut check_siblings = BTreeMap::new();
    for reference in refs {
        let Some(entry) = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .find(|entry| entry.id == reference.acceptance_id)
        else {
            continue;
        };
        if let AcceptanceCheck::Command { command, cwd } = &entry.check {
            let root = match cwd {
                archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => {
                    &binding.policy.project
                }
                archon_workflow::task_set_contract::TrustedCwd::RepoRoot => {
                    &binding.policy.repository
                }
            };
            let revisions = archon_workflow::acceptance_sibling_paths::discover(
                root,
                std::slice::from_ref(command),
            )
            .into_iter()
            .filter_map(|sibling| all_siblings.get(&sibling.source).cloned())
            .collect();
            check_siblings.insert(reference.acceptance_id.clone(), revisions);
        }
    }
    let siblings = all_siblings.into_values().collect();
    Ok((observed.checks, siblings, check_siblings, evidence))
}

fn relative_path(from: &std::path::Path, to: &std::path::Path) -> PathBuf {
    let left = from.components().collect::<Vec<_>>();
    let right = to.components().collect::<Vec<_>>();
    let common = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
    let mut path = PathBuf::new();
    for _ in common..left.len() {
        path.push("..");
    }
    for component in &right[common..] {
        path.push(component.as_os_str());
    }
    path
}
