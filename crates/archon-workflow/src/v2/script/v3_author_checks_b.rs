use super::*;
use crate::v2::review_findings;

pub(super) fn validate_review_kind_shape(
    details: &WorkflowDryRunPlanDetails,
    accepted_task_ids: &std::collections::BTreeSet<String>,
    review_kind: &str,
    purpose: &str,
    all_map_call_ids: &std::collections::BTreeSet<String>,
    defects: &mut Vec<String>,
) {
    let map_call_ids_for_kind = details
        .calls
        .iter()
        .filter(|call| {
            review_contract_kind(call) == Some(review_kind)
                && review_contract_stage(call) == Some(REVIEW_MAP_STAGE)
        })
        .map(|call| call.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let maps = details
        .review_map_claims
        .iter()
        .filter(|claim| claim.review_kind == review_kind)
        .collect::<Vec<_>>();
    if map_call_ids_for_kind.is_empty() {
        defects.push(format!(
            "missing {purpose} map review — add read-only critic map calls with {REVIEW_CONTRACT_MARKER}.kind='{review_kind}' and stage='{REVIEW_MAP_STAGE}'"
        ));
    }

    let mut by_task: std::collections::BTreeMap<&str, Vec<String>> = Default::default();
    for claim in &maps {
        if call_index(&details.calls, &claim.call_id).is_none() {
            defects.push(format!(
                "{purpose} map call `{}` was planned but did not execute in the live call sequence",
                claim.call_id
            ));
        }
        if claim.task_ids.len() != 1 {
            defects.push(format!(
                "{purpose} map item in call `{}` item {:?} covers {} task ids ({:?}) — each map item must cover exactly one accepted task",
                claim.call_id,
                claim.item_id,
                claim.task_ids.len(),
                claim.task_ids
            ));
            continue;
        }
        let task_id = claim.task_ids[0].as_str();
        if !accepted_task_ids.contains(task_id) {
            defects.push(format!(
                "{purpose} map item in call `{}` covers unknown or non-accepted task `{task_id}`",
                claim.call_id
            ));
        }
        by_task
            .entry(task_id)
            .or_default()
            .push(format!("{}:{:?}", claim.call_id, claim.item_id));
    }
    for missing in accepted_task_ids
        .iter()
        .filter(|task_id| !by_task.contains_key(task_id.as_str()))
    {
        defects.push(format!(
            "{purpose} map coverage omitted accepted task `{missing}`"
        ));
    }
    for (task_id, claims) in by_task {
        if claims.len() > 1 {
            defects.push(format!(
                "{purpose} map coverage includes accepted task `{task_id}` more than once ({})",
                claims.join(", ")
            ));
        }
    }

    let reducers = details
        .review_reduce_edges
        .iter()
        .filter(|edge| edge.review_kind == review_kind)
        .collect::<Vec<_>>();
    let finals = reducers
        .iter()
        .filter(|edge| edge.stage == REVIEW_REDUCE_FINAL_STAGE)
        .collect::<Vec<_>>();
    if finals.len() != 1 {
        defects.push(format!(
            "{purpose} must have exactly one final reducer with {REVIEW_CONTRACT_MARKER}.stage='{REVIEW_REDUCE_FINAL_STAGE}' (found {})",
            finals.len()
        ));
    }
    for edge in &reducers {
        if !edge.preserve_map_findings {
            defects.push(format!(
                "{purpose} reducer `{}` must declare preserveMapFindings: true",
                edge.call_id
            ));
        }
        if edge.max_input_bytes.is_none() && edge.max_findings_per_reduce.is_none() {
            defects.push(format!(
                "{purpose} reducer `{}` must declare a reduce bound such as {REVIEW_BOUNDS_HINT} or maxFindingsPerReduce",
                edge.call_id
            ));
        }
        if let Some(index) = call_index(&details.calls, &edge.call_id) {
            for source in edge
                .source_map_call_ids
                .iter()
                .chain(edge.source_reduce_call_ids.iter())
            {
                match call_index(&details.calls, source) {
                    Some(source_index) if source_index > index => {
                        defects.push(format!(
                            "{purpose} reducer `{}` references source `{source}` that runs after it",
                            edge.call_id
                        ));
                    }
                    Some(_) => {}
                    None => defects.push(format!(
                        "{purpose} reducer `{}` references source `{source}` that did not execute",
                        edge.call_id
                    )),
                }
            }
        }
    }

    if let Some(final_reduce) = finals.first() {
        if final_reduce.accounting_field.as_deref() != Some(review_kind) {
            defects.push(format!(
                "{purpose} final reducer `{}` must declare accountingField: '{review_kind}'",
                final_reduce.call_id
            ));
        }
        let direct_maps = final_reduce
            .source_map_call_ids
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let chunk_sources = final_reduce
            .source_reduce_call_ids
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let expected_maps = map_call_ids_for_kind.clone();
        if chunk_sources.is_empty() {
            if direct_maps != expected_maps {
                defects.push(format!(
                    "{purpose} final reducer `{}` must reference every {review_kind} map call exactly once: expected={expected_maps:?} actual={direct_maps:?}",
                    final_reduce.call_id
                ));
            }
        } else {
            for source in &chunk_sources {
                if !reducers
                    .iter()
                    .any(|edge| edge.stage == REVIEW_REDUCE_CHUNK_STAGE && edge.call_id == *source)
                {
                    defects.push(format!(
                        "{purpose} final reducer `{}` references unknown chunk reducer `{source}`",
                        final_reduce.call_id
                    ));
                }
            }
            let chunked_maps = reducers
                .iter()
                .filter(|edge| chunk_sources.contains(&edge.call_id))
                .flat_map(|edge| edge.source_map_call_ids.iter().cloned())
                .collect::<std::collections::BTreeSet<_>>();
            if chunked_maps != expected_maps {
                defects.push(format!(
                    "{purpose} chunk reducers must cover every {review_kind} map call exactly once before final reduce: expected={expected_maps:?} actual={chunked_maps:?}"
                ));
            }
        }
        for source in direct_maps.iter().chain(chunk_sources.iter()) {
            if all_map_call_ids.contains(source) && !expected_maps.contains(source) {
                defects.push(format!(
                    "{purpose} final reducer `{}` references map call `{source}` from another review kind",
                    final_reduce.call_id
                ));
            }
        }
    }
}

/// The accounting the script reports must be exactly what the host attached.
///
/// The host computes each review kind's finding set when the final reducer
/// completes (`review_findings::attach_host_review_findings`) and the script
/// reads that attachment through `reviewFindings`. This check is therefore
/// host-against-host: it re-derives nothing, so it cannot drift from the
/// script's copy of a rule -- there is no script copy. What it still catches is
/// a script that hides or invents findings between reading them and reporting
/// them, which is the only thing the script can get wrong.
pub fn validate_review_accounting_from_reducers(
    script_result: Option<&str>,
    details: &WorkflowDryRunPlanDetails,
    store: &WorkflowV2ResultStore,
) -> WorkflowResult<()> {
    let raw = script_result.ok_or_else(|| {
        WorkflowError::SpecInvalid("authored workflow returned no task accounting".to_string())
    })?;
    let accounting: serde_json::Value = serde_json::from_str(raw).map_err(|err| {
        WorkflowError::SpecInvalid(format!(
            "authored workflow task accounting was not JSON: {err}"
        ))
    })?;
    for (review_kind, purpose) in MANDATED_REVIEW_KINDS {
        let final_reduce = details
            .review_reduce_edges
            .iter()
            .find(|edge| edge.review_kind == review_kind && edge.stage == REVIEW_REDUCE_FINAL_STAGE)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "{purpose} accounting has no final reducer to bind `{review_kind}`"
                ))
            })?;
        let reduce_record = store
            .load_call_record(&final_reduce.call_id)?
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "{purpose} final reducer record `{}` is missing",
                    final_reduce.call_id
                ))
            })?;
        if reduce_record.invalidated_by.is_some() {
            return Err(WorkflowError::SpecInvalid(format!(
                "{purpose} final reducer `{}` was invalidated and cannot back accounting",
                final_reduce.call_id
            )));
        }
        let missing = review_findings::attached_missing_sources(&reduce_record.result.data);
        if !missing.is_empty() {
            return Err(WorkflowError::SpecInvalid(format!(
                "{purpose} final reducer `{}` named source map call(s) with no recorded result: {}",
                final_reduce.call_id,
                missing.join(", ")
            )));
        }
        let host = review_findings::attached(&reduce_record.result.data).ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "{purpose} final reducer `{}` carries no host review findings; the host attaches them when a reduce_final call completes, so this record was not produced by the live host",
                final_reduce.call_id
            ))
        })?;
        let reported = accounting
            .get(review_kind)
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "authored workflow accounting omitted `{review_kind}` — it must come from the {purpose} reducer"
                ))
            })?
            .clone();
        let dropped = review_findings::multiset_difference(&host, &reported);
        if !dropped.is_empty() {
            return Err(WorkflowError::SpecInvalid(format!(
                "{purpose} accounting field `{review_kind}` dropped {} finding(s) the host attached to `{}`: {}",
                dropped.len(),
                final_reduce.call_id,
                serde_json::to_string(&dropped)?
            )));
        }
        let invented = review_findings::multiset_difference(&reported, &host);
        if !invented.is_empty() {
            return Err(WorkflowError::SpecInvalid(format!(
                "{purpose} accounting field `{review_kind}` reports {} finding(s) the host never attached to `{}`: {}",
                invented.len(),
                final_reduce.call_id,
                serde_json::to_string(&invented)?
            )));
        }
    }
    Ok(())
}
