use super::super::*;
use super::*;

/// Inventories run through the same disposable base-tree runner and durable cache.
/// The in-call cache shares one listing per selected package/target/directory.
pub(crate) async fn findings(
    probe: &HostProbe,
    base: &Baseline,
    contract: &AcceptanceContract,
    results: &[super::super::hermetic::CheckEvidence],
) -> BTreeMap<String, String> {
    let mut listing_probe = HostProbe::at_with_limits(
        probe.project.clone(),
        probe.repository.clone(),
        None,
        probe.resource_limits.clone(),
    );
    listing_probe.site = match &probe.site {
        Site::Scratch(binding) => Site::Scratch(binding.clone()),
        Site::Unavailable(_) => return BTreeMap::new(),
        Site::Direct | Site::Hermetic => Site::Hermetic,
    };
    listing_probe.host = probe.host.clone();
    listing_probe.task_root = probe.task_root.clone();
    listing_probe.copy_parent = probe.copy_parent.clone();
    listing_probe.check_cap_secs = probe.check_cap_secs;
    listing_probe.memo = probe.memo;
    listing_probe.resume = crate::command::workflow_freeze_budget::FreezeResume::saving(
        crate::command::workflow_freeze_budget::FreezeBudget::unlimited(),
        probe.resume.persist,
    );
    // An optional inventory failure must not defer the acceptance check itself.
    let mut cache = probe
        .test_inventories
        .lock()
        .expect("inventory cache")
        .clone();
    let mut found = BTreeMap::new();
    let (environment, forwarded) = sites::site_environment(probe);
    let redactor = super::super::super::passability::evidence::Redactor::for_environment(
        environment,
        &forwarded,
    );
    for result in results {
        let Some(entry) = contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .find(|entry| entry.id == result.acceptance_id)
        else {
            continue;
        };
        let Some((_, text)) = executed_text(entry) else {
            continue;
        };
        if !matches!(
            archon_workflow::acceptance_check_crash::classify_check_run(text, result),
            archon_workflow::acceptance_check_crash::CheckRunClass::Passed
        ) {
            continue;
        }
        let invocations = invocations(text);
        if invocations.is_empty() {
            probe.note(format!(
                "check '{}': inventory unavailable (no supported inventory invocation could be resolved)",
                entry.id
            ));
        }
        let mut misses = Vec::new();
        for invocation in invocations {
            let key = format!(
                "{}:{:?}:{}",
                base.commit,
                match &entry.check {
                    AcceptanceCheck::Command { cwd, .. } => Some(cwd),
                    _ => None,
                },
                invocation.list
            );
            if !cache.contains_key(&key) {
                if probe.is_incomplete() || !probe.budget().allows_observation() {
                    probe.note(unavailable_note(
                        &entry.id,
                        "inventory deferred by host budget",
                    ));
                    cache.insert(key, None);
                    continue;
                }
                let mut listing = contract.clone();
                let candidate = listing
                    .acceptance
                    .iter_mut()
                    .chain(&mut listing.supplementary)
                    .find(|candidate| candidate.id == entry.id)
                    .unwrap();
                let cwd = match &entry.check {
                    AcceptanceCheck::Command { cwd, .. } => *cwd,
                    _ => continue,
                };
                candidate.check = AcceptanceCheck::Command {
                    command: invocation.list.clone(),
                    cwd,
                };
                let digest = match contract_digest(&listing) {
                    Ok(digest) => digest,
                    Err(_) => continue,
                };
                let ids = [entry.id.clone()].into_iter().collect();
                let refs = refs_for(&listing, &digest, &ids);
                let run = repairs::tree_results(
                    &listing_probe,
                    base,
                    &listing,
                    &digest,
                    &refs,
                    None,
                    false,
                )
                .await;
                let names = run
                    .results
                    .get(&entry.id)
                    .filter(|item| item.passed())
                    .and_then(|item| {
                        inventory(&invocation.runner, &String::from_utf8_lossy(&item.stdout))
                    });
                cache.insert(key.clone(), names);
            }
            match cache.get(&key).and_then(Option::as_ref) {
                Some(names) => {
                    if let Some(reason) =
                        mismatch_for_run(result.exit_code, &invocation, Some(names))
                    {
                        // Every dead filter of one all-dead command gives the same text.
                        if !misses.contains(&reason) {
                            misses.push(reason);
                        }
                    } else if result.reports_zero_work() {
                        misses.push(format!(
                            "{} reported 0 tests; nearest real test paths: {}",
                            invocation.line,
                            nearest_paths(&invocation, names).join(", ")
                        ));
                    }
                }
                None => probe.note(unavailable_note(&entry.id, &invocation.line)),
            }
        }
        if !misses.is_empty() {
            found.insert(
                entry.id.clone(),
                format!(
                    "check '{}' exited successfully but has a test inventory defect: {}",
                    entry.id,
                    super::super::super::passability::evidence::inert(
                        &redactor.redact(&misses.join("; "))
                    )
                ),
            );
        }
    }
    probe
        .test_inventories
        .lock()
        .expect("inventory cache")
        .extend(cache);
    found
}
