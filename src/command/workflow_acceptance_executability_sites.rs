//! Where a probe's hermetic runs happen: the scratch observation at a
//! commit, or the probe's own copy, with verdicts a freeze already observed
//! reused only while the tree and the project data they ran on are the same.

use std::path::Path;
use std::sync::OnceLock;

use archon_workflow::acceptance_check_environment as check_env;
use archon_workflow::acceptance_scratch::{CHECK_DEFERRED, ObserveHooks, observe_commands_hooked};

use super::hermetic::{Unrun, data_digest};
use super::*;

/// A freeze probe's site and the repository it proves checks against: the
/// hermetic scratch site when a policy is configured (warm from a
/// per-repository cache), else the probe's own copy. A configured policy
/// that cannot be captured, or a repository record that cannot be believed
/// (Issue 366, N1), runs nothing: the repository is then only a placeholder.
pub(super) fn freeze_site(project: &Path, tasks_root: &Path) -> (Site, PathBuf) {
    let site = match crate::command::acceptance_scratch_policy::capture(project, tasks_root) {
        Ok(Some(binding)) => {
            let key = content_digest(binding.policy.repository.to_string_lossy().as_bytes());
            Site::Scratch(Box::new(
                binding.with_run_build_cache(&format!("acceptance-probe-{}", &key[..12])),
            ))
        }
        Ok(None) => Site::Hermetic,
        Err(error) => Site::Unavailable(format!(
            "the [workflow.acceptance_execution] policy could not be captured ({error}); nothing is run until it is repaired"
        )),
    };
    let binding = match &site {
        Site::Scratch(binding) => Some(&**binding),
        Site::Direct | Site::Hermetic | Site::Unavailable(_) => None,
    };
    match live_root::freeze_repository(project, tasks_root, binding) {
        Ok(repository) => (site, repository),
        Err(why) if !matches!(site, Site::Unavailable(_)) => (
            Site::Unavailable(format!("{why}; nothing is run until it is repaired")),
            project.to_path_buf(),
        ),
        Err(_) => (site, project.to_path_buf()),
    }
}

#[cfg(test)]
impl HostProbe {
    /// Fail the next `count` hermetic runs as the host's environment would.
    #[cfg(unix)]
    pub(crate) fn with_injected_failures(self, count: usize) -> Self {
        (self.injected_failures).store(count, std::sync::atomic::Ordering::SeqCst);
        self
    }

    pub(super) fn take_injected_failure(&self) -> bool {
        use std::sync::atomic::Ordering::SeqCst;
        (self.injected_failures)
            .fetch_update(SeqCst, SeqCst, |left| left.checked_sub(1))
            .is_ok()
    }
}

/// Verdicts already observed in this process, by site, tree and check: a
/// freeze's trees do not move under it, so each check is run once per tree.
fn memo() -> &'static Mutex<BTreeMap<String, CheckResult>> {
    static MEMO: OnceLock<Mutex<BTreeMap<String, CheckResult>>> = OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// The state of the project data the site copies, for `tree`.
fn data_state(probe: &HostProbe, tree: &Baseline) -> Vec<String> {
    let mut states = probe.data_states.lock().expect("data states lock");
    states
        .entry(tree.repository.clone())
        .or_insert_with(|| match &probe.site {
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
        })
        .clone()
}

#[path = "workflow_acceptance_executability_sites_reuse.rs"]
mod reuse;
pub(super) use reuse::check_key;

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
                .then(|| reuse::memo_key(probe, tree, &data, contract, &reference.acceptance_id))
                .flatten()
        })
        .collect();
    let store = probe.store();
    let seen: Vec<Option<CheckResult>> = (keys.iter())
        .map(|key| {
            let key = key.as_ref()?;
            // A verdict is reusable only while its original evidence file
            // still exists and validates. The process memo is merely a speed
            // layer over that durable evidence, never evidence itself.
            let store = store.as_ref()?;
            let saved = store.load(key)?;
            let remembered = (probe.process_memo())
                .then(|| memo().lock().ok()?.get(key).cloned())
                .flatten();
            probe.reused_from_disk();
            if let Ok(mut memo) = memo().lock() {
                memo.insert(key.clone(), saved.clone());
            }
            Some(remembered.unwrap_or(saved))
        })
        .collect();
    for ((reference, key), saved) in refs.iter().zip(&keys).zip(&seen) {
        let entry = (contract.acceptance.iter())
            .chain(&contract.supplementary)
            .find(|entry| entry.id == reference.acceptance_id);
        let why = if let Some(entry) = entry {
            if saved.is_some() {
                reuse::memo_reason(probe, tree, &data, entry)
            } else if key.is_some() {
                "no matching prior verdict"
            } else if let AcceptanceCheck::Command { command, .. } = &entry.check {
                crate::command::workflow_task_set::workflow_acceptance_check_reuse::assess(
                    command,
                    "",
                    &crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity()
                        .map_or_else(|| String::new(), |(version, _, _)| version.to_string()),
                    "site environment",
                )
                .reason
            } else {
                "full input closure: same commit and project data"
            }
        } else {
            "check is absent from the contract"
        };
        if let Ok(mut audit) = probe.reuse_audit.lock() {
            audit.insert(
                reference.acceptance_id.clone(),
                super::ReuseAudit {
                    reused: saved.is_some(),
                    why: why.to_string(),
                    evidence: saved.as_ref().and_then(|_| {
                        key.as_ref()
                            .and_then(|key| store.as_ref()?.evidence_ref(key))
                    }),
                },
            );
        }
    }
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
    probe.promote(store.as_ref(), &written);
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
            && !super::verdict::may_be_host_failure(&result)
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
        && archon_shell::spawn::command("git")
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
    archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|head| object_id(head))
}

/// The environment the probe's site gives a check, but the HOME each check
/// gets (the host's, or a fresh one in the probe's copy), and the names its policy forwards from the host: the one
/// check-environment rule (Issue 345,
/// `archon_workflow::acceptance_check_environment`) applied to the probe's
/// host environment. A site with no policy has the default one; the probe's
/// copy builds into its own warm target.
pub(super) fn site_environment(probe: &HostProbe) -> (BTreeMap<String, String>, Vec<String>) {
    let default = || check_env::CheckPolicy::default_for(&probe.host);
    match &probe.site {
        Site::Scratch(binding) => (
            scratch_environment(&binding.policy, &probe.host),
            binding.policy.environment_allowlist.clone(),
        ),
        Site::Hermetic => {
            let target = hermetic::warm_target(&probe.copy_parent, &probe.repository);
            let site = [("CARGO_TARGET_DIR", target.as_path())];
            (
                check_env::site_variables(&probe.host, &default(), &site),
                Vec::new(),
            )
        }
        Site::Direct | Site::Unavailable(_) => (
            check_env::site_variables(&probe.host, &default(), &[]),
            Vec::new(),
        ),
    }
}

/// The variables a check's tool is listed with at `probe`'s site (Issue
/// 333), from that site's own context only: exactly what the site gives
/// every check, its bound and forwarded variables, never the rest of the
/// host's (Issues 282, 345). The listing is given a fresh HOME.
pub(super) fn listing_environment(probe: &HostProbe) -> BTreeMap<String, String> {
    site_environment(probe).0
}

/// The variables a scratch site of `policy` gives every check, besides its
/// own fresh HOME, TMPDIR and build directories: those its policy binds,
/// its toolchain PATH, and the values it forwards from `host`. A forwarded
/// variable `host` lacks fails the scratch's own preparation, naming it;
/// this record of the site is made without it.
pub(super) fn scratch_environment(
    policy: &archon_workflow::acceptance_scratch::ScratchPolicy,
    host: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut env = check_env::site_variables(host, &check_env::CheckPolicy::configured(policy), &[]);
    for name in &policy.environment_allowlist {
        if let Ok(value) = check_env::forwarded_values(host, std::slice::from_ref(name)) {
            env.extend(value);
        }
    }
    env
}

impl HostProbe {
    /// This probe with `host` as the host's environment.
    #[cfg(test)]
    pub(crate) fn with_host_environment(mut self, host: BTreeMap<String, String>) -> Self {
        self.host = host;
        self
    }
}

/// Captured once per freeze, using the check site's PATH and forwarded values.
fn runtime_identity(probe: &HostProbe) -> &serde_json::Value {
    probe.identity.get_or_init(|| {
        let (environment, _) = site_environment(probe);
        let tools: Vec<_> = [("rustc", "-Vv"), ("cargo", "-V")]
            .into_iter()
            .map(|(name, arg)| {
                let executable = environment.get("PATH").and_then(|path| {
                    std::env::split_paths(path)
                        .map(|dir| dir.join(name))
                        .find(|path| path.is_file())
                });
                let binary = executable.as_ref().and_then(|path| {
                    Some((
                        path.canonicalize().ok()?,
                        content_digest(&std::fs::read(path).ok()?),
                    ))
                });
                let mut version = archon_shell::spawn::command(name);
                let version = archon_shell::spawn::replace_environment(&mut version, &environment)
                    .arg(arg)
                    .current_dir(&probe.repository)
                    .output()
                    .ok()
                    .filter(|out| out.status.success())
                    .map(|out| String::from_utf8_lossy(&out.stdout).into_owned());
                serde_json::json!([binary, version])
            })
            .collect();
        serde_json::json!([env!("ARCHON_GIT_HASH"), environment, tools])
    })
}

// The identity tests build their trees with the Unix-only probe fixtures.
#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_identity_tests.rs"]
mod identity_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_environment_tests.rs"]
mod environment_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_sites_recorded_binding_tests.rs"]
mod recorded_binding_tests;
