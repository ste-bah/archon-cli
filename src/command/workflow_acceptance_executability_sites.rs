//! Where a probe's hermetic runs happen: the scratch observation at a
//! commit, or the probe's own copy, with verdicts a freeze already observed
//! reused only while the tree and the project data they ran on are the same.

use std::path::Path;
use std::sync::OnceLock;

use archon_workflow::acceptance_scratch::{CHECK_DEFERRED, ObserveHooks, observe_commands_hooked};

use super::hermetic::{Unrun, data_digest};
use super::*;

/// Verdicts already observed in this process, by site, tree and check: a
/// freeze's trees do not move under it, so each check is run once per tree.
fn memo() -> &'static Mutex<BTreeMap<String, CheckResult>> {
    static MEMO: OnceLock<Mutex<BTreeMap<String, CheckResult>>> = OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// The state of the project data the site copies, for `tree`.
fn data_state(probe: &HostProbe, tree: &Baseline) -> Vec<String> {
    match &probe.site {
        Site::Scratch(binding) => {
            let policy = &binding.policy;
            (policy.project_inputs.iter())
                .map(|input| data_digest(&policy.project.join(input), &policy.repository))
                .chain([data_digest(&policy.task_root, &policy.repository)])
                .collect()
        }
        Site::Direct | Site::Hermetic | Site::Unavailable(_) => {
            vec![data_digest(&probe.project, &tree.repository)]
        }
    }
}

/// The memo key of `reference` on `tree` with project data in `data`: the
/// project data is part of the tree a verdict is evidence of, so data that
/// changed since makes an observed verdict no evidence at all.
fn memo_key(
    probe: &HostProbe,
    tree: &Baseline,
    data: &[String],
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
    let key = serde_json::json!([
        site,
        tree.repository,
        probe.project,
        tree.commit,
        data,
        entry.id,
        entry.check,
    ]);
    Some(content_digest(key.to_string().as_bytes()))
}

/// The memo key of check `id` of `contract` on `tree`, with the project
/// data as it is now.
pub(super) fn check_key(
    probe: &HostProbe,
    tree: &Baseline,
    contract: &AcceptanceContract,
    id: &str,
) -> Option<String> {
    memo_key(probe, tree, &data_state(probe, tree), contract, id)
}

/// Run `refs` once on `tree`, hermetically: in the scratch observation at
/// that commit (without the shared build cache when `cold`), else in the
/// probe's own copy. A freeze probe reuses verdicts it already observed.
pub(super) async fn run_at(
    probe: &HostProbe,
    tree: &Baseline,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    cold: bool,
) -> Result<Vec<CheckResult>, Unrun> {
    if let Site::Unavailable(reason) = &probe.site {
        return Err(Unrun(reason.clone()));
    }
    let data = if probe.memo {
        data_state(probe, tree)
    } else {
        Vec::new()
    };
    let keys: Vec<Option<String>> = (refs.iter())
        .map(|reference| {
            (probe.memo)
                .then(|| memo_key(probe, tree, &data, contract, &reference.acceptance_id))
                .flatten()
        })
        .collect();
    let store = probe.store();
    let seen: Vec<Option<CheckResult>> = (keys.iter())
        .map(|key| {
            let key = key.as_ref()?;
            let remembered = (probe.process_memo())
                .then(|| memo().lock().ok()?.get(key).cloned())
                .flatten();
            remembered.or_else(|| {
                let saved = store.as_ref()?.load(key)?;
                probe.reused_from_disk();
                if let Ok(mut memo) = memo().lock() {
                    memo.insert(key.clone(), saved.clone());
                }
                Some(saved)
            })
        })
        .collect();
    let pending: Vec<FrozenCommandRef> = (refs.iter().zip(&seen))
        .filter(|(_, seen)| seen.is_none())
        .map(|(reference, _)| reference.clone())
        .collect();
    #[cfg(test)]
    if !pending.is_empty() && probe.take_injected_failure() {
        return Err(Unrun("injected host failure".into()));
    }
    // Issue 255: nothing new starts once the freeze's budget is spent.
    if !pending.is_empty() && (probe.is_incomplete() || !probe.budget().allows_observation()) {
        probe.defer(pending.iter().map(|r| r.acceptance_id.as_str()));
        return Err(Unrun(CHECK_DEFERRED.to_string()));
    }
    let pending_keys: BTreeMap<String, String> = (refs.iter().zip(&keys).zip(&seen))
        .filter(|(_, seen)| seen.is_none())
        .filter_map(|((reference, key), _)| Some((reference.acceptance_id.clone(), key.clone()?)))
        .collect();
    let written = Arc::new(Mutex::new(Vec::new()));
    let hooks = probe.observe_hooks(&pending_keys, store.as_ref(), &written);
    let ran = if pending.is_empty() {
        Ok(Vec::new())
    } else if let Site::Scratch(binding) = &probe.site {
        let mut binding = NativeBinding::clone(binding);
        if cold {
            binding.policy.build_cache = None;
        }
        let cancel = CancelOnDrop(Arc::new(AtomicBool::new(false)));
        tokio::spawn(observe(
            binding,
            Some(tree.commit.clone()),
            contract.clone(),
            digest.to_string(),
            pending,
            cancel.0.clone(),
            hooks,
        ))
        .await
        .map_err(|error| Unrun(format!("scratch probe task failed: {error}")))
        .and_then(|observed| observed.map_err(|error| Unrun(format!("{error:#}"))))
    } else {
        hermetic::run_in_copy(
            probe,
            &tree.repository,
            &tree.commit,
            contract,
            digest,
            &pending,
            &hooks,
        )
        .await
    };
    // A voided run's verdicts were told early, but are no evidence.
    let ran = ran.inspect_err(|_| probe.unsave(store.as_ref(), &written))?;
    probe.defer(
        (ran.iter())
            .filter(|result| result.operational_error.as_deref() == Some(CHECK_DEFERRED))
            .map(|result| result.acceptance_id.as_str()),
    );
    let mut ran: BTreeMap<String, CheckResult> = (ran.into_iter())
        .map(|result| (result.acceptance_id.clone(), result))
        .collect();
    let mut results = Vec::new();
    for ((reference, key), seen) in refs.iter().zip(keys).zip(seen) {
        if let Some(seen) = seen {
            results.push(seen);
            continue;
        }
        let Some(result) = ran.remove(&reference.acceptance_id) else {
            continue;
        };
        if let (Some(key), None) = (key, &result.operational_error)
            && let Ok(mut memo) = memo().lock()
        {
            memo.insert(key, result.clone());
        }
        results.push(result);
    }
    Ok(results)
}

/// Whether `text` is a full object id: SHA-1 (40) or SHA-256 (64) hex.
pub(super) fn object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether `commit` names a commit in `repository`.
pub(super) fn resolves(repository: &Path, commit: &str) -> bool {
    object_id(commit)
        && std::process::Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(["cat-file", "-e", &format!("{commit}^{{commit}}")])
            .output()
            .is_ok_and(|output| output.status.success())
}

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
) -> anyhow::Result<Vec<CheckResult>> {
    let identity = binding
        .policy
        .repository
        .canonicalize()
        .map(archon_shell::paths::plain)?;
    let _lease = crate::command::acceptance_scratch_guardian::acquire_lease(
        &std::env::temp_dir().join("archon-native-observer-locks"),
        &identity.to_string_lossy(),
    )?;
    let head = match commit {
        Some(commit) => commit,
        None => git_head(&binding.policy.repository)
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
    Ok(observed.checks)
}

pub(super) fn git_head(repository: &std::path::Path) -> Option<String> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|head| object_id(head))
}
