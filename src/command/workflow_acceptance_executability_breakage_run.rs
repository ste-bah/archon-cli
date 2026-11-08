//! One bounded observation, with all copy IO off the asynchronous executor.
use super::*;
use archon_workflow::base_snapshot::{BaseSnapshot, SnapshotEntry, key_path, skip_copy_entry};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
pub(super) fn inputs(probe: &HostProbe) -> HostProbe {
    let mut inputs = HostProbe::at_with_limits(
        probe.project.clone(),
        probe.repository.clone(),
        None,
        probe.resource_limits.clone(),
    );
    inputs.site = Site::Hermetic;
    inputs.task_root = probe.task_root.clone();
    inputs.host = probe.host.clone();
    inputs.baseline = probe.baseline.clone();
    inputs.identity = probe.identity.clone();
    inputs.memo = probe.memo;
    inputs.copy_parent = probe.copy_parent.clone();
    inputs.resume = probe.resume.clone();
    inputs.check_cap_secs = probe.check_cap_secs;
    *inputs.data_states.lock().expect("private input cache") =
        probe.data_states.lock().expect("input cache").clone();
    inputs
}
pub(super) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, RunOutcome> + Send + 'static,
) -> Result<T, RunOutcome> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| decide::unrun("host_io", &e.to_string()))?
}
pub(super) async fn materialise(
    probe: &HostProbe,
    snapshot: &BaseSnapshot,
    work: &Path,
) -> Result<Arc<BaseCopy>, RunOutcome> {
    let (parent, repository, project, task, snapshot, work) = (
        probe.copy_parent.clone(),
        probe.repository.clone(),
        probe.project.clone(),
        probe.task_root.clone(),
        snapshot.clone(),
        work.to_path_buf(),
    );
    blocking(move || {
        materialise_base_copy(
            &parent,
            &repository,
            &project,
            task.as_deref(),
            &snapshot,
            &snapshot.commit,
            &work.join("build-output"),
            Some(&work.join("base.git")),
        )
        .map(Arc::new)
        .map_err(patch::host)
    })
    .await
}
pub(super) async fn remove(copy: Arc<BaseCopy>) -> Result<(), RunOutcome> {
    blocking(move || {
        Arc::try_unwrap(copy)
            .map_err(|_| decide::unrun("host_io", "copy still borrowed"))?
            .remove()
            .map_err(|e| decide::unrun("host_io", &e))
    })
    .await
}
fn remove_absent(
    copy: &HermeticCopy,
    snapshot: &BaseSnapshot,
    live_repository: &Path,
    live_project: &Path,
) -> Result<(), RunOutcome> {
    for (key, entry) in &snapshot.entries {
        if matches!(entry, SnapshotEntry::Tracked { .. }) || skip_copy_entry(key, entry) {
            continue;
        }
        let relative = key_path(key);
        let mut paths = vec![
            copy.repository.join(&relative),
            copy.project.join(&relative),
        ];
        if let Ok(prefix) = live_project.strip_prefix(live_repository)
            && let Ok(rel) = relative.strip_prefix(prefix)
            && !rel.as_os_str().is_empty()
        {
            paths.push(copy.project.join(rel));
        }
        for path in paths {
            // Check every ancestor: a check may have replaced a directory with a link.
            let Some(root) = [&copy.repository, &copy.project]
                .into_iter()
                .find(|r| path.starts_with(r))
            else {
                continue;
            };
            if path
                .strip_prefix(root)
                .expect("bounded path")
                .ancestors()
                .skip(1)
                .any(|p| {
                    root.join(p)
                        .symlink_metadata()
                        .is_ok_and(|m| m.file_type().is_symlink())
                })
            {
                return Err(decide::unrun("host_io", "linked placeholder ancestor"));
            }
            if let Ok(meta) = path.symlink_metadata() {
                let result = if meta.is_dir() || meta.file_type().is_symlink() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                };
                result.map_err(|e| decide::unrun("host_io", &e.to_string()))?;
            }
        }
    }
    Ok(())
}
fn resolve(
    copy: &HermeticCopy,
    command: &str,
    cwd: &Path,
    touched: &[String],
    raw: &str,
) -> Option<(String, LocatedClass)> {
    let path = PathBuf::from(raw.replace('\\', "/"));
    let candidates = if path.is_absolute() {
        vec![path]
    } else {
        vec![
            cwd.join(&path),
            copy.repository.join(&path),
            copy.project.join(&path),
        ]
    };
    let path = candidates.into_iter().find_map(|p| {
        archon_shell::paths::canonicalize(p)
            .ok()
            .filter(|p| p.is_file())
    })?;
    if !path.starts_with(&copy.root) {
        return Some((path.to_string_lossy().into(), LocatedClass::External));
    }
    let key = path
        .strip_prefix(&copy.repository)
        .ok()
        .map(relative_key)
        .unwrap_or_else(|| relative_key(path.strip_prefix(&copy.root).expect("copy path")));
    let class = if touched.contains(&key) {
        LocatedClass::Mutated
    } else {
        let named = verdict_shell::simple_commands(command)
            .into_iter()
            .flat_map(|c| std::iter::once(c.word).chain(c.args))
            .filter(|w| !verdict_shell::expands(w))
            .any(|w| {
                let operand = PathBuf::from(w.replace('\\', "/"));
                [cwd, &copy.repository, &copy.project].iter().any(|root| {
                    archon_shell::paths::canonicalize(root.join(&operand))
                        .ok()
                        .is_some_and(|p| path == p || path.starts_with(p))
                })
            });
        if named {
            LocatedClass::CheckNamed
        } else {
            LocatedClass::Elsewhere
        }
    };
    Some((key, class))
}
pub(super) async fn observe(
    probe: &HostProbe,
    req: &BreakageRequest<'_>,
    copy: &Arc<BaseCopy>,
    id: &str,
    touched: &[String],
    wall: Option<u64>,
) -> (RunOutcome, Option<Vec<u8>>, bool) {
    let (owned, snapshot, repo, project) = (
        copy.clone(),
        req.classification
            .base_snapshot
            .clone()
            .expect("validated snapshot"),
        probe.repository.clone(),
        probe.project.clone(),
    );
    if let Err(e) = blocking(move || remove_absent(owned.copy(), &snapshot, &repo, &project)).await
    {
        return (e, None, false);
    }
    let refs = refs_for(req.contract, req.chain_digest, &[id.to_string()].into());
    if refs.len() != 1 {
        return (
            decide::unrun("host_io", "check reference unavailable"),
            None,
            false,
        );
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let fired = Arc::new(AtomicBool::new(false));
    let timer = wall.map(|secs| {
        let (cancel, fired) = (cancel.clone(), fired.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(secs)).await;
            fired.store(true, Ordering::SeqCst);
            cancel.store(true, Ordering::SeqCst);
        })
    });
    let start = Instant::now();
    let execution = Arc::new(inputs(probe));
    let (owned, copied, contract, digest, runtime) = (
        execution.clone(),
        copy.clone(),
        req.contract.clone(),
        req.chain_digest.to_string(),
        tokio::runtime::Handle::current(),
    );
    let result = blocking(move || {
        runtime
            .block_on(run_refs_in_copy(
                &owned,
                copied.copy(),
                &owned.repository,
                &contract,
                &digest,
                &refs,
                &archon_workflow::acceptance_scratch::ObserveHooks::for_resource_limits(
                    owned.resource_limits.clone(),
                ),
                &Mutex::new(None),
                cancel,
            ))
            .map_err(patch::host)
    })
    .await;
    if let Some(timer) = timer {
        timer.abort();
    }
    let elapsed_ms = start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let result = match result {
        Ok(run) => match run.results.into_iter().next() {
            Some(r) => r,
            None => return (decide::unrun("host_io", "missing observation"), None, true),
        },
        Err(e) => return (e, None, true),
    };
    let output = store::output(probe, &result);
    let (copy, contract, id, touched, deadline) = (
        copy.clone(),
        req.contract.clone(),
        id.to_string(),
        touched.to_vec(),
        fired.load(Ordering::SeqCst),
    );
    match blocking(move || {
        let entry = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .find(|c| c.id == id)
            .expect("selected check");
        let (command, cwd) = match &entry.check {
            AcceptanceCheck::Command { command, cwd } => (
                command.as_str(),
                if *cwd == archon_workflow::task_set_contract::TrustedCwd::RepoRoot {
                    copy.repository()
                } else {
                    copy.project()
                },
            ),
            _ => ("", copy.repository()),
        };
        let context = CheckSite::new(sites::site_environment(&execution).0, &contract);
        let outcome = decide::decide(
            &result,
            elapsed_ms,
            deadline,
            wall.unwrap_or(0),
            &|p| resolve(copy.copy(), command, cwd, &touched, p),
            &|| {
                verdict::mutant_cause(command, &result, &context)
                    .map(|(detail, cause)| (detail, cause.code().into()))
            },
        );
        Ok(outcome)
    })
    .await
    {
        Ok(outcome) => (outcome, Some(output), true),
        Err(e) => (e, None, true),
    }
}

pub(super) fn control(outcome: RunOutcome) -> RunOutcome {
    match outcome {
        RunOutcome::Survived { elapsed_ms } => RunOutcome::Passed { elapsed_ms },
        RunOutcome::Killed {
            exit_code,
            elapsed_ms,
        } => RunOutcome::Failed {
            exit_code,
            elapsed_ms,
        },
        other => other,
    }
}
