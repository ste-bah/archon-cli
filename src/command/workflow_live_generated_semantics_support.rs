use archon_workflow::{
    WorkflowError, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2WriteMode,
};

use super::workflow_live_task_universe::WorkflowV2TaskUniverse;

pub(super) fn contains_completed_id_add_loop(compact: &str) -> bool {
    !completed_id_add_loop_pattern(compact).is_empty()
}

pub(super) fn completed_id_add_loop_pattern<'a>(compact: &'a str) -> &'a str {
    if compact.contains("for(constidofnewlyCompletedIds){completedIds.add(id);}") {
        "for(constidofnewlyCompletedIds){completedIds.add(id);}"
    } else if compact.contains("for(constidofnewlyCompletedIds){completedIds.add(id)}") {
        "for(constidofnewlyCompletedIds){completedIds.add(id)}"
    } else {
        ""
    }
}

pub(super) fn implementation_intent_call(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Implementation
        || call.write_mode.is_some()
        || call
            .options
            .item_kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("implementation"))
}

pub(super) fn require_helper_contract(compact: &str) -> archon_workflow::WorkflowResult<()> {
    require(
        compact.contains("functionacceptedOrNoopCanonicalTaskIdsFrom(outcomes)"),
        "generated decomposed PRD workflow must define acceptedOrNoopCanonicalTaskIdsFrom(outcomes)",
    )?;
    require(
        compact.contains("for(constoutcomeofarray(outcomes))")
            && compact.contains("outcome.status!==\"accepted\"&&outcome.status!==\"noop\"")
            && compact.contains("!hasConcreteEvidence(outcome)")
            && compact.contains("for(constidofcanonicalIdsFor(outcome))")
            && compact.contains("ids.push(id)")
            && compact.contains("returnids"),
        "generated decomposed PRD workflow helper must accept only accepted/noop outcomes with concrete evidence and non-empty canonical IDs from canonicalTaskUniverse",
    )
}

pub(super) fn require_repair_investigation_contract(
    compact: &str,
) -> archon_workflow::WorkflowResult<()> {
    require(
        compact.contains("normalizeGeneratedInventory(rawInventory)")
            && compact.contains("repairAttempts=[]")
            && compact.contains("inventory.unresolved_issues"),
        "generated decomposed PRD workflow must normalize inventory through the shared generated PRD contract before validation",
    )?;
    require(
        compact.contains("functiongeneratedContractIsSupportItem(item)")
            && compact.contains("support_items"),
        "generated decomposed PRD workflow must separate unowned support evidence from schedulable implementation inventory",
    )?;
    for (needle, message) in [
        (
            "w.reduce(\"inventory-shape-repair-\"+repairAttempt",
            "generated decomposed PRD workflow must run JS-owned inventory-shape-repair before malformed inventory can block",
        ),
        (
            "w.reduce(\"task-universe-reconcile-\"+repairAttempt",
            "generated decomposed PRD workflow must run JS-owned task-universe-reconcile for non-canonical dependencies",
        ),
        (
            "w.reduce(\"target-file-discovery-\"+repairAttempt",
            "generated decomposed PRD workflow must investigate missing target files before blocking",
        ),
        (
            "w.reduce(\"verification-requirements-discovery-\"+repairAttempt",
            "generated decomposed PRD workflow must investigate missing verification requirements before blocking",
        ),
        (
            "w.reduce(\"artifact-requirements-discovery-\"+repairAttempt",
            "generated decomposed PRD workflow must investigate missing artifact requirements before blocking",
        ),
        (
            "w.reduce(\"provider-environment-discovery-\"+repairAttempt",
            "generated decomposed PRD workflow must investigate provider/environment evidence before blocking",
        ),
        (
            "w.reduce(\"evidence-repair-\"+repairAttempt",
            "generated decomposed PRD workflow must repair missing no-op/evidence refs before blocking",
        ),
    ] {
        require(compact.contains(needle), message)?;
    }
    require(
        compact.contains("recordRepairAttempt(repairAttempts")
            && compact.contains("repair_attempts:repairAttempts"),
        "generated decomposed PRD workflow blocked/needs_review reports must include structured repair_attempts evidence",
    )?;
    require(
        occurs_before(
            compact,
            "w.reduce(\"inventory-shape-repair-\"+repairAttempt",
            "finalReport(\"blocked-malformed-inventory",
        ),
        "generated decomposed PRD workflow must not finalReport blocked-malformed-inventory before repair/investigation attempts",
    )?;
    reject(
        compact.contains("afterRepairIssueFingerprint===beforeRepairIssueFingerprint")
            || compact.contains("afterRepairIssueFingerprint==beforeRepairIssueFingerprint"),
        "generated decomposed PRD workflow must not stop repair/investigation loops solely because an issue fingerprint is stable before configured caps are exhausted",
    )?;
    Ok(())
}

pub(super) fn implementation_write_fanout(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Fanout
        && matches!(call.write_mode, Some(WorkflowV2WriteMode::Worktree))
        && call
            .options
            .item_kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("implementation"))
}

pub(super) fn call_source_matches(call: &WorkflowV2HostCall, expected: &str) -> bool {
    call.options.source.as_deref() == Some(expected)
        || call
            .options
            .extra
            .get("raw_source_expr")
            .and_then(serde_json::Value::as_str)
            == Some(expected)
}

pub(super) fn declares_variable(compact: &str, name: &str) -> bool {
    compact.contains(&format!("let{name}="))
        || compact.contains(&format!("const{name}="))
        || compact.contains(&format!("var{name}="))
}

pub(super) fn contains_dynamic_implementation_wave_call(compact: &str) -> bool {
    compact.contains(
        "w.fanout(\"implementation-wave-\"+currentImplementationWaveIndex,readyImplementationItems",
    ) || compact.contains(
        "w.fanout('implementation-wave-'+currentImplementationWaveIndex,readyImplementationItems",
    )
}

pub(super) fn contains_dynamic_remediation_wave_call(compact: &str) -> bool {
    compact.contains(
        "w.fanout(\"remediation-wave-\"+currentImplementationWaveIndex,remediationInventory.items",
    ) || compact.contains(
        "w.fanout('remediation-wave-'+currentImplementationWaveIndex,remediationInventory.items",
    )
}

pub(super) fn contains_remediation_inventory_from_nonaccepted_wave_outcomes(
    compact: &str,
    calls: &[WorkflowV2HostCall],
) -> bool {
    let allowed_filtered_sources = remediation_filter_vars(compact);
    let source_expressions = remediation_inventory_reduce_source_expressions(compact);
    if source_expressions.is_empty()
        || source_expressions.iter().any(|source| {
            !remediation_source_is_allowed(source, allowed_filtered_sources.as_slice())
        })
    {
        return false;
    }
    calls.iter().any(remediation_inventory_reduce)
}

pub(super) fn remediation_source_is_allowed(
    source: &str,
    allowed_filtered_sources: &[String],
) -> bool {
    source == "wave.outcomes"
        || allowed_filtered_sources.iter().any(|allowed| {
            allowed.as_str() == source || source_contains_array_entry(source, allowed)
        })
}

fn source_contains_array_entry(source: &str, expected: &str) -> bool {
    source.starts_with('[')
        && source.ends_with(']')
        && source
            .trim_matches(|ch| ch == '[' || ch == ']')
            .split(',')
            .map(str::trim)
            .any(|entry| entry == expected)
}

pub(super) fn remediation_inventory_reduce_source_expressions(compact: &str) -> Vec<String> {
    let mut sources = Vec::new();
    for prefix in [
        "w.reduce(\"remediation-inventory-\"+currentImplementationWaveIndex,",
        "w.reduce('remediation-inventory-'+currentImplementationWaveIndex,",
    ] {
        let mut offset = 0;
        while let Some(relative) = compact[offset..].find(prefix) {
            let source_start = offset + relative + prefix.len();
            let Some(source_end) = source_expression_end(compact, source_start) else {
                break;
            };
            sources.push(compact[source_start..source_end].trim().to_string());
            offset = source_end.saturating_add(1);
        }
    }
    sources
}

fn source_expression_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut depth = 0usize;
    let mut quote = None;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(active) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
                continue;
            }
            if byte == active {
                quote = None;
            }
            index += 1;
            continue;
        }
        match byte {
            b'\'' | b'"' | b'`' => quote = Some(byte),
            b'[' | b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => return Some(index),
            _ => {}
        }
        index += 1;
    }
    None
}

pub(super) fn remediation_inventory_reduce(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Reduce
        && (call
            .options
            .extra
            .get("dynamic_id_prefix")
            .and_then(serde_json::Value::as_str)
            == Some("remediation-inventory-")
            || call.id.starts_with("remediation-inventory-"))
}

pub(super) fn remediation_filter_vars(compact: &str) -> Vec<String> {
    let mut vars = Vec::new();
    for decl in ["const", "let", "var"] {
        let mut offset = 0;
        while let Some(relative) = compact[offset..].find(decl) {
            let start = offset + relative + decl.len();
            let Some((name, value_start)) = parse_assignment(compact, start) else {
                offset = start;
                continue;
            };
            let value_end = compact[value_start..]
                .find(';')
                .map(|relative_end| value_start + relative_end)
                .unwrap_or(compact.len());
            let value = &compact[value_start..value_end];
            if (value.contains("wave.outcomes")
                && value.contains(".filter(")
                && filters_out_accepted_and_noop(value))
                || value.contains("nonAcceptedOutcomes(wave.outcomes)")
            {
                vars.push(name.to_string());
            }
            offset = value_end.saturating_add(1);
        }
    }
    vars
}

pub(super) fn parse_assignment(compact: &str, name_start: usize) -> Option<(&str, usize)> {
    let rest = compact.get(name_start..)?;
    let name_len = rest
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '$')
        .map(|(index, ch)| index + ch.len_utf8())
        .last()?;
    let name = compact.get(name_start..name_start + name_len)?;
    let equals_index = name_start + name_len;
    if compact.as_bytes().get(equals_index).copied() != Some(b'=') {
        return None;
    }
    Some((name, equals_index + 1))
}

pub(super) fn filters_out_accepted_and_noop(value: &str) -> bool {
    let excludes_by_comparison = (value.contains("status!==\"accepted\"")
        || value.contains("status!=\"accepted\""))
        && (value.contains("status!==\"noop\"") || value.contains("status!=\"noop\""))
        && value.contains("&&");
    let excludes_by_literal_set = value.contains("![\"accepted\",\"noop\"].includes(")
        || value.contains("![\"noop\",\"accepted\"].includes(");
    excludes_by_comparison || excludes_by_literal_set
}

pub(super) fn occurs_before(compact: &str, before: &str, after: &str) -> bool {
    match (compact.find(before), compact.find(after)) {
        (Some(before_index), Some(after_index)) => before_index < after_index,
        _ => false,
    }
}

pub(super) fn extract_embedded_task_universe(
    source: &str,
) -> archon_workflow::WorkflowResult<WorkflowV2TaskUniverse> {
    let task_universe_pos = source.find("taskUniverse").ok_or_else(|| {
        WorkflowError::SpecInvalid(
            "generated decomposed PRD workflow must declare taskUniverse".to_string(),
        )
    })?;
    let after_name = &source[task_universe_pos..];
    let open_offset = after_name.find('{').ok_or_else(|| {
        WorkflowError::SpecInvalid(
            "generated decomposed PRD workflow taskUniverse must be a JSON object literal"
                .to_string(),
        )
    })?;
    let open = task_universe_pos + open_offset;
    let close = matching_json_object_close(source, open).ok_or_else(|| {
        WorkflowError::SpecInvalid(
            "generated decomposed PRD workflow taskUniverse JSON object is unterminated"
                .to_string(),
        )
    })?;
    serde_json::from_str(&source[open..=close]).map_err(|err| {
        WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow taskUniverse must be parseable JSON: {err}"
        ))
    })
}

pub(super) fn matching_json_object_close(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in source[open..].char_indices() {
        let absolute = open + offset;
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(absolute);
                }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn compact_js(source: &str) -> String {
    source.chars().filter(|ch| !ch.is_whitespace()).collect()
}

pub(super) fn require(
    condition: bool,
    message: &'static str,
) -> archon_workflow::WorkflowResult<()> {
    if condition {
        Ok(())
    } else {
        Err(WorkflowError::SpecInvalid(message.to_string()))
    }
}

pub(super) fn reject(
    condition: bool,
    message: &'static str,
) -> archon_workflow::WorkflowResult<()> {
    require(!condition, message)
}
