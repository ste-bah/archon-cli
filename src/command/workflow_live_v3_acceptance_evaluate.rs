use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn evaluate(
    runtime: &WorkflowV2ScriptRuntime,
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    llm: Option<&dyn archon_workflow::WorkflowLlmClient>,
    request: &StageRequest,
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
    ran: &mut Option<output::Frozen>,
) -> WorkflowResult<()> {
    let mut context = match exec::resolve_context(
        store,
        run_id,
        runtime.target_repository_root.as_deref(),
        task_universe,
        runtime.cargo_limits.clone(),
    ) {
        Ok(context) => context,
        Err(error) => {
            record.operational_errors.push(error.to_string());
            return Ok(());
        }
    };
    record.execution = Some(context.execution_record());
    // A5: every authored or repaired check must fail on the tree before
    // implementation: the one its freeze recorded (Issue 328).
    let base = Baseline::round_commit(&context.repository, &context.task_root, run_dir);
    let site = author::Site {
        llm,
        context: &context,
        run_dir,
        base: base.as_deref(),
        store,
        run_id,
        call_id,
    };
    // REM-13: no frozen contract is never a note: the host authors and
    // freezes one, then runs it.
    if let Some(authored) = author::heal_unfrozen(&site).await? {
        record.contract_repairs.extend(authored.repair);
        if !authored.errors.is_empty() {
            record.contract_present = context.contract_path().exists();
            record.operational_errors.extend(authored.errors);
            return Ok(());
        }
    }
    if !context.contract_path().exists() {
        record.contract_present = false;
        // A8: absent is never a pass. An authored run completes only on a
        // frozen contract whose checks ran, declared or not.
        record.operational_errors.push(match exec::contract_declaration(&context, task_universe) {
            Some(declared) => format!(
                "{} is missing although the task set declares an acceptance contract ({declared})",
                context.contract_path().display()
            ),
            None => format!(
                "{} is missing: an authored run completes only on a frozen acceptance contract whose checks ran; author and freeze one for this task set: cd {} && archon workflow freeze-acceptance --tasks {} --prd <the task set's PRD>",
                context.contract_path().display(),
                context.project.display(),
                context.task_root.display()
            ),
        });
        return Ok(());
    }
    record.contract_present = true;
    let (mut contract, mut chain_digest, frozen) = match exec::load_contract(&context) {
        Ok(loaded) => loaded,
        Err(paused @ WorkflowError::ControlPaused(_)) => return Err(paused),
        Err(error) => {
            record
                .operational_errors
                .push(format!("frozen acceptance contract is not usable: {error}"));
            return Ok(());
        }
    };
    if !frozen {
        record.operational_errors.push(format!(
            "{} has no acceptance-contract.lock: the contract is not frozen, so its checks cannot be trusted as the task set's acceptance; run the freeze before implementation",
            context.contract_path().display()
        ));
        return Ok(());
    }
    for requested in &request.check_ids {
        if !contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .any(|criterion| &criterion.id == requested)
        {
            record.operational_errors.push(format!(
                "requested acceptance check '{requested}' is not in the frozen contract"
            ));
        }
    }
    if !record.operational_errors.is_empty() {
        return Ok(());
    }
    if let Some(authored) = author_drift::heal_drift(&site, &contract).await? {
        let added = authored
            .repair
            .as_ref()
            .is_some_and(|repair| repair.repaired);
        record.contract_repairs.extend(authored.repair);
        record.operational_errors.extend(authored.errors);
        if added {
            match exec::load_contract(&context) {
                Ok((reloaded, digest, _)) => (contract, chain_digest) = (reloaded, digest),
                Err(paused @ WorkflowError::ControlPaused(_)) => return Err(paused),
                Err(error) => {
                    record.operational_errors.push(format!(
                        "the extended acceptance contract is not usable: {error}"
                    ));
                    return Ok(());
                }
            }
        }
    }
    // PLAN-11: no check runs on a source nobody judged; pending source
    // changes settle before a repair republishes (and re-pins) the chain.
    let sources = sources::apply(llm, &context, &contract, run_dir, record).await?;
    // A check the judge did not accept can never run: repair the contract
    // before running it, and never hand it to the implementing tasks.
    let Some(mut defects) = repair::apply(
        llm,
        &context,
        &mut contract,
        &mut chain_digest,
        record,
        base.as_deref(),
    )
    .await?
    else {
        return Ok(());
    };
    let mut authored_pending = pending::ids_from_record(record);
    for (id, why) in &sources.defects {
        defects.entry(id.clone()).or_insert(why.clone());
    }
    let evidence_dir =
        round_dir(run_dir, request.round).join(format!("attempt-{:02}", record.attempt));
    // Run the whole contract so a fix cannot hide a regression in another check.
    // A contract defect cannot run: it is recorded.
    let mut results = {
        let selected: Vec<&AcceptanceCriterion> = (contract.acceptance.iter())
            .chain(&contract.supplementary)
            .filter(|criterion| {
                !defects.contains_key(&criterion.id) && !authored_pending.contains(&criterion.id)
            })
            .collect();
        poll_v2_run_control(store, run_id, call_id)?;
        // A1: what could not run is the host's to get running, in-round.
        let mut repairs = Vec::new();
        let ran = env::execute_repairing(
            store,
            run_id,
            call_id,
            &mut context,
            &contract,
            &chain_digest,
            &selected,
            &evidence_dir,
            &mut repairs,
        )
        .await?;
        write_repairs(&evidence_dir, &repairs);
        if !ran.site_errors.is_empty() {
            record.operational_errors.extend(ran.site_errors);
            return Ok(());
        }
        (selected.iter().map(|criterion| criterion.id.clone()))
            .zip(ran.results)
            .collect::<std::collections::BTreeMap<String, CheckResult>>()
    };
    // A check that crashed in its own code is a contract defect too: it is
    // repaired, republished and re-run in this round, never handed to a task.
    let round = repair::Round {
        llm,
        context: &context,
        store,
        run_id,
        call_id,
        evidence_dir: &evidence_dir,
        base: base.as_deref(),
    };
    (repair::repair_crashed(
        &round,
        &mut contract,
        &mut chain_digest,
        &mut results,
        record,
    ))
    .await?
    .into_iter()
    .for_each(|(id, defect)| drop(defects.insert(id, defect)));
    pending::extend_ids(&mut authored_pending, record);
    // Issue 219: a pass counts only from a check proven able to fail.
    let held = failability::hold_passing(
        &round,
        run_dir,
        &mut contract,
        &mut chain_digest,
        &mut results,
        &mut defects,
        record,
    );
    held.await?;
    pending::extend_ids(&mut authored_pending, record);
    let all: Vec<&AcceptanceCriterion> = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .collect();
    let selected: Vec<&AcceptanceCriterion> = (all.iter().copied())
        .filter(|criterion| {
            !defects.contains_key(&criterion.id) && !authored_pending.contains(&criterion.id)
        })
        .collect();
    // Issue 544: the tip's failure fingerprints, from the raw captures.
    let tip_signatures: std::collections::BTreeMap<String, String> = results
        .iter()
        .filter_map(|(id, result)| Some((id.clone(), regression::typed_signature(result)?)))
        .collect();
    for criterion in &all {
        let result = results.remove(&criterion.id);
        if let Some(result) = &result {
            write_output_files(&evidence_dir, result);
        }
        if let Some(defect) = defects.get(&criterion.id) {
            record
                .checks
                .push(repair::defect_record(criterion, defect, result.as_ref()));
            continue;
        }
        if authored_pending.contains(&criterion.id) {
            let pending = pending::record_from_round(
                criterion,
                exec::check_kind(criterion),
                result.as_ref(),
                record,
            );
            record.checks.push(pending);
            continue;
        }
        let Some(result) = result else {
            continue;
        };
        record
            .checks
            .push(check_record(criterion, &result, task_universe));
    }
    sources::fail_unsatisfied(record, &sources.must_fail);
    // A8: a contract of which no check ran proves nothing.
    if record.checks.is_empty() {
        record.operational_errors.push(format!(
            "the frozen contract at {} ran no check: an authored run completes only on checks that ran",
            context.contract_path().display()
        ));
    }
    // Batch J: every failed check is searched for a point of the run it held
    // at and, from there, for the landing that broke it, so the remediation
    // reaches the task that can fix it; the rest carry what the search found.
    poll_v2_run_control(store, run_id, call_id)?;
    regression::attribute_with(
        &context,
        &selected,
        &chain_digest,
        run_dir,
        &evidence_dir,
        record,
        archon_workflow::v2::acceptance_regression::SearchBudget::default(),
        &tip_signatures,
    )
    .await?;
    use archon_workflow::v2::acceptance_routing as routing;
    // Batch O2: a file the run's scope-amendment ledger records an owner for
    // goes to that owner (an unreadable ledger is reported below, when the
    // grants are recorded).
    let owned = routing::recorded_ownership(run_dir).unwrap_or_default();
    routing::route_failures_owned(task_universe, &context.repository, &all, &owned, record);
    // A13 / hole 12: a failed check no rule gave a unit, and one no task's
    // `implements` names, is reassigned to the tasks nearest what it
    // implicates, never raised and dropped.
    routing::mark_blocked(record);
    routing::reroute(task_universe, record);
    // ACC-H3: every grant goes through the chained scope-amendment ledger,
    // one link per check, so the write fan-out's amended universe holds it
    // and stored project data lands through the audited project inputs.
    archon_workflow::stage_write::with_write(|| {
        routing::record_routed_grants(run_dir, task_universe, &context.repository, record);
        WorkflowResult::Ok(())
    })?;
    // A7: the contract is held to the PRD as it is now (what the in-round
    // authoring still owes is already a round error per check).
    // M3: always, on the contract as finally reloaded.
    record
        .operational_errors
        .extend(drift::drift_errors(&context, &contract));
    *ran = Some(output::Frozen {
        contract,
        digest: chain_digest,
    });
    Ok(())
}
