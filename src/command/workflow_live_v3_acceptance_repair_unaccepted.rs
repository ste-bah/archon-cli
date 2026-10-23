use super::*;

/// Repair every non-accepted check in `contract`. `None` when there is none.
/// `ControlPaused` when the republish met a journal another publish left
/// that no recovery can settle (Issue 338): never a failed repair.
#[cfg(all(test, unix))]
pub async fn repair_unaccepted(
    llm: Option<&dyn WorkflowLlmClient>,
    context: &StageContext,
    contract: &AcceptanceContract,
    base: Option<&str>,
) -> WorkflowResult<Option<(AcceptanceContractRepairV1, Defects)>> {
    Ok(repair_unaccepted_detailed(llm, context, contract, base)
        .await?
        .map(|(mut repairs, defects)| (repairs.remove(0), defects)))
}

async fn repair_unaccepted_detailed(
    llm: Option<&dyn WorkflowLlmClient>,
    context: &StageContext,
    contract: &AcceptanceContract,
    base: Option<&str>,
) -> WorkflowResult<Option<(Vec<AcceptanceContractRepairV1>, Defects)>> {
    let ids = non_accepted_ids(contract);
    if ids.is_empty() {
        return Ok(None);
    }
    // A check the judge never accepted never ran: there is no verdict of
    // its own to hold a repair to, only the baseline.
    let outcome = republish(llm, context, contract, &ids, &BTreeMap::new(), base, None).await;
    if let Some(paused) = outcome.as_ref().err().and_then(pause_if_unsettled) {
        return Err(paused);
    }
    let check_ids = ids.iter().cloned().collect::<Vec<_>>();
    Ok(Some(match outcome {
        Ok(result) => {
            let proven: Vec<_> = ids
                .iter()
                .filter(|id| !result.unproven.contains_key(*id))
                .cloned()
                .collect();
            let unresolved: Vec<_> = result.unproven.iter().collect();
            let mut repairs = Vec::new();
            if !proven.is_empty() || unresolved.is_empty() {
                repairs.push(AcceptanceContractRepairV1 {
                    check_ids: if proven.is_empty() {
                        check_ids.clone()
                    } else {
                        proven
                    },
                    trigger: REPAIR_TRIGGER_NOT_ACCEPTED.into(),
                    repaired: result.published,
                    freeze_event_id: result.freeze_event_id.clone(),
                    failure: String::new(),
                    diagnostics: result.diagnostics.clone(),
                });
            }
            let defects = unresolved
                .iter()
                .map(|(id, reason)| {
                    (
                        (*id).clone(),
                        format!("the check remains pending because its probe could not prove it: {reason}\nThe subject stays routed to its author."),
                    )
                })
                .collect();
            if !unresolved.is_empty() {
                repairs.push(AcceptanceContractRepairV1 {
                    check_ids: unresolved.iter().map(|(id, _)| (*id).clone()).collect(),
                    trigger: REPAIR_TRIGGER_NOT_ACCEPTED.into(),
                    repaired: false,
                    freeze_event_id: result.freeze_event_id.clone(),
                    failure: unresolved
                        .iter()
                        .map(|(id, reason)| format!("{id}: {reason}"))
                        .collect::<Vec<_>>()
                        .join(" | "),
                    diagnostics: result.diagnostics.clone(),
                });
            }
            (repairs, defects)
        }
        Err(error) => {
            let command = reauthor_command(&context.project, &context.task_root, contract, &ids);
            let defects = ids
                .iter()
                .map(|id| {
                    (
                        id.clone(),
                        format!(
                            "contract defect: frozen check '{id}' was not accepted by the judge, so it can never run and no task can fix it; the in-round re-author did not produce an accepted check ({error:#}); repair the contract with: {command}"
                        ),
                    )
                })
                .collect();
            (
                vec![AcceptanceContractRepairV1 {
                    check_ids,
                    trigger: REPAIR_TRIGGER_NOT_ACCEPTED.into(),
                    repaired: false,
                    freeze_event_id: String::new(),
                    failure: format!("{error:#}"),
                    diagnostics: Vec::new(),
                }],
                defects,
            )
        }
    }))
}

/// Repair the round's contract in place and record the attempt. Returns the
/// checks that remain contract defects, or `None` when the repaired contract
/// could not be reloaded (recorded as the round's operational error).
/// `ControlPaused` when the republished set's interrupted publish cannot be
/// settled (Issue 336): the run pauses; it is not the round's error.
pub async fn apply(
    llm: Option<&dyn WorkflowLlmClient>,
    context: &StageContext,
    contract: &mut AcceptanceContract,
    chain_digest: &mut String,
    record: &mut AcceptanceRoundRecordV1,
    base: Option<&str>,
) -> WorkflowResult<Option<Defects>> {
    let Some((repairs, defects)) = repair_unaccepted_detailed(llm, context, contract, base).await?
    else {
        return Ok(Some(Defects::new()));
    };
    let repaired = repairs.iter().any(|repair| repair.repaired);
    for repair in &repairs {
        if !repair.repaired && !repair.failure.is_empty() {
            record
                .operational_errors
                .push(format!("{HOST_UNPROVEN} {}", repair.failure));
        }
    }
    record.contract_repairs.extend(repairs);
    if repaired {
        match exec::load_contract(context) {
            Ok((reloaded, digest, _)) => (*contract, *chain_digest) = (reloaded, digest),
            Err(paused @ WorkflowError::ControlPaused(_)) => return Err(paused),
            Err(error) => {
                record.operational_errors.push(format!(
                    "repaired acceptance contract is not usable: {error}"
                ));
                return Ok(None);
            }
        }
    }
    Ok(Some(defects))
}
