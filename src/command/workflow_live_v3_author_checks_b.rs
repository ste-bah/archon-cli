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

pub(super) fn validate_review_accounting_from_reducers(
    script_result: Option<&str>,
    details: &WorkflowDryRunPlanDetails,
    store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<()> {
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
        let map_findings = collect_map_findings(details, store, review_kind)?;
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
        let reduce_findings = extract_review_findings_from_record(&reduce_record)?;
        assert_multiset_contains(
            &reduce_findings,
            &map_findings,
            &format!(
                "{purpose} final reducer `{}` dropped map findings",
                final_reduce.call_id
            ),
        )?;
        let accounting_findings = accounting
            .get(review_kind)
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "authored workflow accounting omitted `{review_kind}` — it must come from the {purpose} reducer"
                ))
            })?
            .clone();
        assert_multiset_equal(
            &accounting_findings,
            &reduce_findings,
            &format!(
                "authored workflow accounting field `{review_kind}` does not match final reducer `{}`",
                final_reduce.call_id
            ),
        )?;
    }
    Ok(())
}

pub(super) fn collect_map_findings(
    details: &WorkflowDryRunPlanDetails,
    store: &WorkflowV2ResultStore,
    review_kind: &str,
) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
    let mut findings = Vec::new();
    let call_ids = details
        .review_map_claims
        .iter()
        .filter(|claim| claim.review_kind == review_kind)
        .map(|claim| claim.call_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for call_id in call_ids {
        let record = store.load_call_record(&call_id)?.ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "mandatory review map record `{call_id}` is missing"
            ))
        })?;
        if record.invalidated_by.is_some() {
            return Err(WorkflowError::SpecInvalid(format!(
                "mandatory review map record `{call_id}` was invalidated"
            )));
        }
        findings.extend(extract_review_findings_from_record(&record)?);
    }
    Ok(findings)
}

pub(super) fn extract_review_findings_from_record(
    record: &WorkflowV2CallRecord,
) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
    let mut findings = Vec::new();
    collect_findings_arrays(&record.result.data, &mut findings);
    if findings.is_empty() {
        return Ok(Vec::new());
    }
    Ok(findings)
}

pub(super) fn collect_findings_arrays(value: &serde_json::Value, findings: &mut Vec<serde_json::Value>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_findings_arrays(item, findings);
            }
        }
        serde_json::Value::Object(object) => {
            for key in ["findings", "adversarial_findings", "uncovered_requirements"] {
                if let Some(array) = object.get(key).and_then(serde_json::Value::as_array) {
                    findings.extend(array.iter().cloned());
                }
            }
            for key in ["data", "result", "items", "outcomes"] {
                if let Some(child) = object.get(key) {
                    collect_findings_arrays(child, findings);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn assert_multiset_contains(
    haystack: &[serde_json::Value],
    needles: &[serde_json::Value],
    context: &str,
) -> archon_workflow::WorkflowResult<()> {
    let haystack = finding_multiset(haystack)?;
    let needles = finding_multiset(needles)?;
    for (finding, count) in needles {
        let have = haystack.get(&finding).copied().unwrap_or(0);
        if have < count {
            return Err(WorkflowError::SpecInvalid(format!(
                "{context}: missing finding {finding} expected {count} found {have}"
            )));
        }
    }
    Ok(())
}

pub(super) fn assert_multiset_equal(
    left: &[serde_json::Value],
    right: &[serde_json::Value],
    context: &str,
) -> archon_workflow::WorkflowResult<()> {
    let left = finding_multiset(left)?;
    let right = finding_multiset(right)?;
    if left != right {
        return Err(WorkflowError::SpecInvalid(format!(
            "{context}: left={left:?} right={right:?}"
        )));
    }
    Ok(())
}

pub(super) fn finding_multiset(
    values: &[serde_json::Value],
) -> archon_workflow::WorkflowResult<std::collections::BTreeMap<String, usize>> {
    let mut out = std::collections::BTreeMap::new();
    for value in values {
        let key = serde_json::to_string(value)?;
        *out.entry(key).or_default() += 1;
    }
    Ok(out)
}
