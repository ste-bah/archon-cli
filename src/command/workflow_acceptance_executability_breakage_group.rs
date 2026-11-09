//! Sequential controls and perturbations, reserving the final control's budget.
use super::*;
struct Item {
    mutant: SuppliedMutant,
    control: String,
    key: String,
    cacheable: bool,
    cached: Option<store::CachedRun>,
    inspection_error: Option<RunOutcome>,
}
fn once(summary: &mut ProofSummary, code: &str) {
    if !summary.refusals.iter().any(|c| c == code) {
        summary.refusals.push(code.into());
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute(
    probe: &HostProbe,
    req: &BreakageRequest<'_>,
    id: &str,
    mutants: Vec<SuppliedMutant>,
    work: &Path,
    root: Option<&Path>,
    prepared: &Result<(), RunOutcome>,
    proofs: &mut BreakageProofs,
    summary: &mut ProofSummary,
) {
    let entry = req
        .contract
        .acceptance
        .iter()
        .chain(&req.contract.supplementary)
        .find(|e| e.id == id)
        .expect("selected check");
    let digest = check_digest(&entry.check);
    let mut items = Vec::new();
    for mutant in mutants {
        let key = store::keys(
            probe,
            req.contract,
            id,
            &mutant.patch_digest(),
            req.min_deadline_secs,
        )
        .await;
        let cacheable = key.is_some();
        let (control, key) = key.unwrap_or_else(|| {
            once(summary, "cache_unavailable");
            (id.into(), uuid::Uuid::new_v4().to_string())
        });
        let cached = if let Some(root) = root
            && cacheable
        {
            let (root, key, digest, id) = (
                root.to_path_buf(),
                key.clone(),
                digest.clone(),
                id.to_string(),
            );
            match run::blocking(move || {
                store::read(&root, &key, &digest, &id)
                    .map_err(|e| decide::unrun("proofs_unreadable", &e))
            })
            .await
            {
                Ok(v) => v,
                Err(_) => {
                    once(summary, "proofs_unreadable");
                    None
                }
            }
        } else {
            None
        };
        items.push(Item {
            mutant,
            control,
            key,
            cacheable,
            cached,
            inspection_error: None,
        });
    }
    let uncached = items.iter().any(|i| i.cached.is_none());
    let cached_group = items.iter().any(|i| i.cached.is_some());
    let mut locally_reused = false;
    if items.iter().any(|i| i.cached.is_some()) {
        probe.resume.progress.reused(false);
    }
    let mut control = ControlRecord {
        check_id: id.into(),
        check_digest: digest.clone(),
        pre: RunOutcome::BudgetExhausted,
        post: None,
        restored_runs: 0,
    };
    let mut copy = None;
    let mut inventory = None;
    let mut poisoned = false;
    let mut tainted = false;
    let mut observed_control = false;
    let snapshot = req
        .classification
        .base_snapshot
        .as_ref()
        .expect("validated snapshot");
    if uncached && req.max_runs.saturating_sub(summary.runs_executed) >= 2 {
        let materialised = match prepared {
            Ok(()) => run::materialise(probe, snapshot, work).await,
            Err(e) => Err(e.clone()),
        };
        match materialised {
            Ok(c) => {
                let owned = c.clone();
                inventory = run::blocking(move || patch::inventory(owned.copy()))
                    .await
                    .ok();
                let (outcome, _, executed) = run::observe(probe, req, &c, id, &[], None).await;
                summary.runs_executed += u64::from(executed);
                observed_control = executed;
                control.pre = run::control(outcome);
                if matches!(control.pre, RunOutcome::Passed { .. }) {
                    let owned = c.clone();
                    let before = inventory.clone();
                    match run::blocking(move || {
                        let before = before.ok_or_else(|| {
                            decide::unrun("host_io", "copy inventory unavailable")
                        })?;
                        patch::restore(owned.copy(), Path::new(""), &[], &before)
                    })
                    .await
                    {
                        Ok(n) => control.restored_runs += n,
                        Err(e) => control.pre = e,
                    }
                }
                copy = Some(c);
            }
            Err(e) => control.pre = e,
        }
    }
    if let Some(c) = &copy
        && matches!(control.pre, RunOutcome::Passed { .. })
    {
        let mut ranks = BTreeMap::new();
        for item in &mut items {
            let touched = if let Some(cached) = &item.cached {
                Some(cached.touched.clone())
            } else {
                let (copy, file, bytes) = (
                    c.clone(),
                    work.join("patches")
                        .join(format!("{}.patch", item.mutant.mutant_id())),
                    item.mutant.patch.as_bytes().to_vec(),
                );
                match run::blocking(move || {
                    patch::inspect(copy.copy(), &file, &bytes).map(|(paths, _)| paths)
                })
                .await
                {
                    Ok(paths) => Some(paths),
                    Err(error) => {
                        item.inspection_error = Some(error);
                        None
                    }
                }
            };
            let pair = req
                .classification
                .entries
                .iter()
                .find(|e| e.task_id == item.mutant.task_id && e.id == item.mutant.requirement_id)
                .expect("validated pair");
            let countable = touched.is_some_and(|paths| {
                selection::attribution(
                    &paths,
                    &selection::keys(pair, snapshot, &probe.repository, false),
                    &selection::keys(pair, snapshot, &probe.repository, true),
                )
                .is_none()
            });
            ranks.insert(item.key.clone(), !countable);
        }
        items.sort_by_key(|i| {
            (
                ranks.get(&i.key).copied().unwrap_or(true),
                i.mutant.task_id.clone(),
                i.mutant.requirement_id.clone(),
                i.mutant.mutant_id(),
            )
        });
    }
    let mut pending = Vec::new();
    let mut outputs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut observed: BTreeMap<String, (RunOutcome, Vec<String>, Evidence)> = BTreeMap::new();
    for item in items {
        let mutant = &item.mutant;
        let mut record = MutantRun {
            check_id: id.into(),
            check_digest: digest.clone(),
            control_key: item.control.clone(),
            mutant_id: mutant.mutant_id(),
            patch_digest: mutant.patch_digest(),
            task_id: mutant.task_id.clone(),
            requirement_id: mutant.requirement_id.clone(),
            kind: mutant.kind,
            touched: Vec::new(),
            outcome: RunOutcome::BudgetExhausted,
            counted: false,
            not_counted: None,
            evidence: Evidence {
                patch: mutant.patch.clone(),
                output: None,
            },
        };
        let mut reused = false;
        if let Some(cached) = &item.cached {
            record.outcome = cached.outcome.clone();
            record.touched = cached.touched.clone();
            record.evidence = cached.evidence.clone();
            proofs
                .controls
                .insert(item.control.clone(), cached.control.clone());
            summary.runs_reused += 1;
            reused = true;
        } else if let Some((outcome, touched, evidence)) = observed.get(&item.key) {
            record.outcome = outcome.clone();
            record.touched = touched.clone();
            record.evidence = evidence.clone();
            summary.runs_reused += 1;
            locally_reused = true;
        } else if matches!(control.pre, RunOutcome::BudgetExhausted) { /* no group budget */
        } else if !matches!(control.pre, RunOutcome::Passed { .. }) {
            record.outcome = match &control.pre {
                RunOutcome::Unrun { code, .. } if code != "zero_work" => control.pre.clone(),
                RunOutcome::NoExitStatus => control.pre.clone(),
                _ => decide::unrun("control_mismatch", "the initial control did not pass"),
            };
            record.not_counted = Some(NotCounted::ControlMismatch);
        } else if poisoned {
            record.outcome =
                decide::unrun("copy_poisoned", "the previous copy could not be restored");
            if let Some(old) = copy.take()
                && let Err(e) = run::remove(old).await
            {
                summary.refusals.push(decide::refusal(&e));
            }
            match run::materialise(probe, snapshot, work).await {
                Ok(c) => {
                    let owned = c.clone();
                    match run::blocking(move || patch::inventory(owned.copy())).await {
                        Ok(v) => {
                            inventory = Some(v);
                            poisoned = false;
                            copy = Some(c);
                        }
                        Err(e) => {
                            record.outcome = e;
                            copy = Some(c);
                        }
                    }
                }
                Err(e) => record.outcome = e,
            }
        } else if let Some(error) = &item.inspection_error {
            record.outcome = error.clone();
        } else if req.max_runs.saturating_sub(summary.runs_executed) > 1 {
            if let (Some(c), Some(before)) = (&copy, &inventory) {
                let file = work
                    .join("patches")
                    .join(format!("{}.patch", mutant.mutant_id()));
                let (owned, file_owned, bytes) =
                    (c.clone(), file.clone(), mutant.patch.as_bytes().to_vec());
                let applied =
                    run::blocking(move || patch::apply(owned.copy(), &file_owned, &bytes)).await;
                match applied {
                    Ok(touched) => {
                        record.touched = touched;
                        let elapsed = match control.pre {
                            RunOutcome::Passed { elapsed_ms } => elapsed_ms / 1000,
                            _ => 0,
                        };
                        let wall = probe.check_bound_secs().saturating_sub(1).min(
                            req.min_deadline_secs
                                .max(elapsed.saturating_mul(3).saturating_add(30)),
                        );
                        let (outcome, output, executed) =
                            run::observe(probe, req, c, id, &record.touched, Some(wall)).await;
                        summary.runs_executed += u64::from(executed);
                        record.outcome = outcome;
                        record.evidence.output = output
                            .as_ref()
                            .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                        if let Some(bytes) = output {
                            outputs.insert(item.key.clone(), bytes);
                        }
                    }
                    Err(e) => record.outcome = e,
                }
                let (owned, touched, before) = (c.clone(), record.touched.clone(), before.clone());
                match run::blocking(move || patch::restore(owned.copy(), &file, &touched, &before))
                    .await
                {
                    Ok(n) => control.restored_runs += n,
                    Err(e) => {
                        poisoned = true;
                        tainted = true;
                        if !matches!(&record.outcome, RunOutcome::Unrun { code, .. } if code == "mtime_not_set")
                        {
                            record.outcome = e;
                        }
                    }
                }
            } else {
                record.outcome = decide::unrun("host_io", "copy inventory unavailable");
            }
        }
        if !reused && store::reusable(&record.outcome) {
            observed.insert(
                item.key.clone(),
                (
                    record.outcome.clone(),
                    record.touched.clone(),
                    record.evidence.clone(),
                ),
            );
        }
        let context_key = content_digest(
            &serde_json::to_vec(&(&item.key, &mutant.task_id, &mutant.requirement_id))
                .expect("serializable pair"),
        );
        pending.push((item, context_key, record, reused));
    }
    if uncached
        && matches!(control.pre, RunOutcome::Passed { .. })
        && let Some(c) = &copy
    {
        if poisoned {
            control.post = Some(decide::unrun("copy_poisoned", "restoration failed"));
        } else {
            let (outcome, _, executed) = run::observe(probe, req, c, id, &[], None).await;
            summary.runs_executed += u64::from(executed);
            control.post = Some(run::control(outcome));
            if let Some(before) = &inventory {
                let (owned, before) = (c.clone(), before.clone());
                match run::blocking(move || {
                    patch::restore(owned.copy(), Path::new(""), &[], &before)
                })
                .await
                {
                    Ok(n) => control.restored_runs += n,
                    Err(e) => {
                        control.post = Some(e);
                        tainted = true;
                    }
                }
            }
        }
    }
    if (observed_control && !control.stable() || tainted)
        && let Some(root) = root
    {
        let (root, keys) = (
            root.to_path_buf(),
            pending
                .iter()
                .filter(|(i, _, _, _)| i.cacheable)
                .map(|(i, _, _, _)| i.key.clone())
                .collect::<Vec<_>>(),
        );
        if run::blocking(move || {
            for key in keys {
                match std::fs::remove_file(root.join("runs").join(format!("{key}.json"))) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(decide::unrun("proofs_not_saved", &e.to_string())),
                }
            }
            Ok(())
        })
        .await
        .is_err()
        {
            once(summary, "proofs_not_saved");
        }
    }
    let mut saved = false;
    for (item, key, mut record, reused) in pending {
        let controls = if reused && !observed_control {
            &item.cached.as_ref().expect("cached").control
        } else {
            &control
        };
        let pair = req
            .classification
            .entries
            .iter()
            .find(|e| e.task_id == record.task_id && e.id == record.requirement_id)
            .expect("validated pair");
        if matches!(record.outcome, RunOutcome::Killed { .. }) {
            record.not_counted = if !controls.stable() {
                Some(if matches!(controls.pre, RunOutcome::Passed { .. }) {
                    NotCounted::ControlUnstable
                } else {
                    NotCounted::ControlMismatch
                })
            } else {
                selection::attribution(
                    &record.touched,
                    &selection::keys(pair, snapshot, &probe.repository, false),
                    &selection::keys(pair, snapshot, &probe.repository, true),
                )
            };
            record.counted = record.not_counted.is_none();
        }
        if observed_control {
            proofs
                .controls
                .insert(item.control.clone(), control.clone());
        } else if !reused {
            proofs
                .controls
                .entry(item.control.clone())
                .or_insert_with(|| control.clone());
        }
        if !reused
            && item.cacheable
            && !tainted
            && control.stable()
            && store::reusable(&record.outcome)
            && let Some(root) = root
        {
            let output = outputs.get(&item.key).cloned();
            let cached = store::CachedRun {
                schema: BREAKAGE_SCHEMA,
                key: item.key,
                control: control.clone(),
                outcome: record.outcome.clone(),
                touched: record.touched.clone(),
                evidence: record.evidence.clone(),
            };
            let (root, patch, output, ck, mid) = (
                root.to_path_buf(),
                item.mutant.patch.into_bytes(),
                output,
                item.control,
                record.mutant_id.clone(),
            );
            match run::blocking(move || {
                store::save(&root, &cached, &patch, output.as_deref(), &ck, &mid)
                    .map_err(|e| decide::unrun("proofs_not_saved", &e))
            })
            .await
            {
                Ok(evidence) => {
                    record.evidence = evidence;
                    saved = true;
                }
                Err(_) => once(summary, "proofs_not_saved"),
            }
        }
        proofs.runs.insert(key, record);
    }
    if locally_reused && !cached_group {
        probe.resume.progress.reused(false);
    }
    if saved {
        probe.resume.progress.saved(false);
    }
    if let Some(c) = copy
        && let Err(e) = run::remove(c).await
    {
        summary.refusals.push(decide::refusal(&e));
    }
}
