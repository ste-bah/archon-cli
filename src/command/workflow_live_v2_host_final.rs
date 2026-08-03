use super::*;

pub(super) fn final_report_result(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let source_results = final_report_source_results(source_results(execution, v2_store)?);
    let required_task_ids = authoritative_task_ids(task_universe)
        .unwrap_or_else(|| required_task_ids_from_results(&source_results));
    let paths = report_paths(v2_store.root());
    let mut report = WorkflowV2FinalReportBuilder::new()
        .build(paths, &required_task_ids, &source_results)
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?;
    guard_final_report_artifact_paths_exist(&mut report, v2_store.root());
    guard_final_report_blocker_freshness(&mut report, v2_store.root());
    guard_final_report_against_dynamic_wave_evidence(&mut report, v2_store, task_universe)?;
    reconcile_final_task_statuses(&mut report, &required_task_ids);
    let report_path = artifact_path(
        v2_store.root(),
        &format!("{}-final-report", execution.call.id),
    );
    write_json(&report_path, &report)?;

    let mut result = WorkflowV2Result {
        status: report.status,
        summary: format!(
            "final report '{}' produced status {:?}",
            execution.call.id, report.status
        ),
        artifacts: vec![WorkflowV2Artifact {
            id: execution.call.id.clone(),
            path: report_path.display().to_string(),
            description: Some("workflow V2 final acceptance report".to_string()),
        }],
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "final report was derived from typed prior host-call results",
    ));
    if report.status != WorkflowV2Status::Accepted {
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "final report contains failed, review-needed, missing, residual, or unverified work",
        ));
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!(
                "final_report_not_accepted_{}",
                sanitize_id(&execution.call.id)
            ),
            description: format!(
                "failed={:?}; blocked={:?}; missing={:?}; residual_gaps={}",
                report.failed_tasks,
                report.blocked_tasks,
                report.missing_tasks,
                report.residual_gaps.len()
            ),
            severity: Some("review".to_string()),
        });
    }
    let mut data = serde_json::to_value(report)?;
    if result.status != WorkflowV2Status::Accepted
        && let Some(object) = data.as_object_mut()
        && let Some(blocker) = final_report_blocker_context(execution)
    {
        object.insert("blocker".to_string(), blocker);
    }
    result.data = data;
    Ok(result)
}

fn guard_final_report_blocker_freshness(report: &mut WorkflowV2FinalReport, v2_root: &Path) {
    let mut superseded = Vec::new();
    report.residual_gaps.retain(|gap| {
        let contradictions = contradicted_existence_claims(v2_root, &gap.description);
        if contradictions.is_empty() {
            return true;
        }
        superseded.push(format!(
            "superseded by current state: blocker {} dropped; {}",
            gap.id,
            contradictions.join("; ")
        ));
        false
    });
    if superseded.is_empty() {
        return;
    }
    report.review_findings.extend(superseded);
    let mut by_id = std::collections::BTreeMap::new();
    for gap in &report.residual_gaps {
        by_id.entry(gap.id.clone()).or_insert_with(|| gap.clone());
    }
    report.review_blockers = by_id.into_values().collect();
}

fn final_report_source_results(results: Vec<WorkflowV2Result>) -> Vec<WorkflowV2Result> {
    results
        .into_iter()
        .filter(|result| !proofless_accepted_metadata(result))
        .collect()
}

fn proofless_accepted_metadata(result: &WorkflowV2Result) -> bool {
    matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) && !source_result_has_concrete_evidence(result)
}

fn guard_final_report_against_dynamic_wave_evidence(
    report: &mut WorkflowV2FinalReport,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> archon_workflow::WorkflowResult<()> {
    let records = v2_store.load_call_records()?;
    let mut dynamic_universe = authoritative_task_ids(task_universe)
        .map(|ids| ids.into_iter().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    let mut completed_ids = BTreeSet::new();
    let mut ledger_accepted_ids = BTreeSet::new();
    let mut ledger_noop_ids = BTreeSet::new();
    let mut ledger_task_coverage = Vec::new();
    let mut ledger_commands = Vec::new();
    let was_accepted_report = report.status == WorkflowV2Status::Accepted;
    for record in records {
        completed_ids.extend(
            record
                .completed_ids
                .iter()
                .filter(|id| !id.trim().is_empty())
                .cloned(),
        );
        let mut record_has_valid_completion = false;
        for evidence in &record.completion_evidence {
            if !artifact_paths_exist(v2_store.root(), &evidence.artifact_paths) {
                report.residual_gaps.push(WorkflowV2ResidualGap {
                    id: format!(
                        "missing_evidence_artifact_{}",
                        sanitize_id(&evidence.task_id)
                    ),
                    description: format!(
                        "task {} references missing artifact evidence",
                        evidence.task_id
                    ),
                    severity: Some("review".to_string()),
                });
                if was_accepted_report {
                    report.status = WorkflowV2Status::NeedsReview;
                    continue;
                }
                continue;
            }
            if matches!(
                evidence.status,
                WorkflowV2Status::Accepted | WorkflowV2Status::Noop
            ) {
                record_has_valid_completion = true;
            }
        }
        if record_has_valid_completion {
            ledger_task_coverage.extend(record.result.task_coverage.clone());
            ledger_commands.extend(record.result.commands_run.clone());
        }
        let Some(graph) = record.source_task_graph else {
            continue;
        };
        dynamic_universe.extend(
            graph
                .canonical_task_universe
                .into_iter()
                .filter(|id| !id.trim().is_empty()),
        );
        completed_ids.extend(
            graph
                .completed_ids
                .into_iter()
                .filter(|id| !id.trim().is_empty()),
        );
    }
    let (credit, _) = validated_completion_credit(v2_store, task_universe)?;
    let implementation_ids = credit.implementation;
    let verification_ids = credit.verification;
    let noop_ids = credit.noop;
    if dynamic_universe.is_empty() {
        return Ok(());
    }
    for task_id in &dynamic_universe {
        if noop_ids.contains(task_id)
            || (implementation_ids.contains(task_id) && verification_ids.contains(task_id))
        {
            completed_ids.insert(task_id.clone());
        }
        if noop_ids.contains(task_id) {
            ledger_noop_ids.insert(task_id.clone());
        } else if implementation_ids.contains(task_id) && verification_ids.contains(task_id) {
            ledger_accepted_ids.insert(task_id.clone());
        }
    }
    ledger_accepted_ids = ledger_accepted_ids
        .intersection(&completed_ids)
        .cloned()
        .collect();
    ledger_noop_ids = ledger_noop_ids
        .intersection(&completed_ids)
        .cloned()
        .collect();
    report.accepted_tasks = merge_sorted_strings(
        std::mem::take(&mut report.accepted_tasks),
        ledger_accepted_ids,
    );
    report.noop_tasks =
        merge_sorted_strings(std::mem::take(&mut report.noop_tasks), ledger_noop_ids);
    merge_ledger_task_coverage(report, ledger_task_coverage, &completed_ids);
    merge_ledger_commands(report, ledger_commands);
    let claimed = report
        .accepted_tasks
        .iter()
        .chain(report.noop_tasks.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let unsupported_claims = claimed
        .difference(&completed_ids)
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing_dynamic = dynamic_universe
        .difference(&completed_ids)
        .cloned()
        .collect::<BTreeSet<_>>();
    report
        .missing_tasks
        .retain(|task_id| !completed_ids.contains(task_id));
    if unsupported_claims.is_empty() && missing_dynamic.is_empty() {
        // The builder downgrades a report whose direct source results carry no
        // commands; the durable ledger merged above is the authoritative
        // command evidence. Restore acceptance when the ledger reconciled
        // cleanly and nothing else is wrong.
        if report.status == WorkflowV2Status::NeedsReview
            && !report.commands_run.is_empty()
            && report.failed_tasks.is_empty()
            && report.blocked_tasks.is_empty()
            && report.missing_tasks.is_empty()
            && report.residual_gaps.is_empty()
        {
            report.status = WorkflowV2Status::Accepted;
        }
        return Ok(());
    }

    if was_accepted_report {
        report.status = WorkflowV2Status::NeedsReview;
    }
    report
        .accepted_tasks
        .retain(|task_id| completed_ids.contains(task_id));
    report
        .noop_tasks
        .retain(|task_id| completed_ids.contains(task_id));
    if was_accepted_report {
        report.missing_tasks = merge_sorted_strings(
            std::mem::take(&mut report.missing_tasks),
            missing_dynamic.clone(),
        );
    }
    report.failed_tasks = merge_sorted_strings(
        std::mem::take(&mut report.failed_tasks),
        unsupported_claims.clone(),
    );
    if !unsupported_claims.is_empty() || was_accepted_report {
        report.residual_gaps.push(WorkflowV2ResidualGap {
            id: "dynamic_wave_acceptance_evidence".to_string(),
            description: format!(
                "final report task claims must be backed by sanitized accepted/noop dynamic wave outcomes; unsupported_claims=[{}]; missing_dynamic_tasks=[{}]",
                unsupported_claims.into_iter().collect::<Vec<_>>().join(", "),
                missing_dynamic.into_iter().collect::<Vec<_>>().join(", ")
            ),
            severity: Some("blocking".to_string()),
        });
    }
    Ok(())
}

fn merge_ledger_task_coverage(
    report: &mut WorkflowV2FinalReport,
    extra: Vec<archon_workflow::WorkflowV2TaskCoverage>,
    completed_ids: &BTreeSet<String>,
) {
    let mut seen = report
        .task_coverage
        .iter()
        .map(task_coverage_key)
        .collect::<BTreeSet<_>>();
    for coverage in extra {
        if completed_ids.contains(&coverage.task_id) && seen.insert(task_coverage_key(&coverage)) {
            report.task_coverage.push(coverage);
        }
    }
}

fn task_coverage_key(coverage: &archon_workflow::WorkflowV2TaskCoverage) -> String {
    format!(
        "{}::{:?}::{}",
        coverage.task_id, coverage.status, coverage.summary
    )
}

fn merge_ledger_commands(
    report: &mut WorkflowV2FinalReport,
    extra: Vec<archon_workflow::WorkflowV2CommandRecord>,
) {
    let mut seen = report
        .commands_run
        .iter()
        .map(command_key)
        .collect::<BTreeSet<_>>();
    for command in extra {
        if seen.insert(command_key(&command)) {
            report.commands_run.push(command);
        }
    }
}

fn command_key(command: &archon_workflow::WorkflowV2CommandRecord) -> String {
    format!(
        "{:?}::{:?}::{:?}::{}",
        command.kind, command.status, command.exit_code, command.command
    )
}

fn guard_final_report_artifact_paths_exist(report: &mut WorkflowV2FinalReport, v2_root: &Path) {
    let project_root = project_root_for_v2_root(v2_root);
    report.artifacts.retain(|artifact| {
        final_artifact_path_exists(artifact.path.trim(), project_root.as_deref())
    });
}

fn project_root_for_v2_root(v2_root: &Path) -> Option<PathBuf> {
    v2_root
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == ".archon"))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

fn final_artifact_path_exists(raw: &str, project_root: Option<&Path>) -> bool {
    if raw.is_empty() {
        return false;
    }
    if archon_workflow::v2::artifact_refs::is_nonfilesystem_artifact_ref(raw) {
        return true;
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return path.exists();
    }
    project_root.is_some_and(|root| root.join(path).exists()) || path.exists()
}

fn merge_sorted_strings(mut existing: Vec<String>, extra: BTreeSet<String>) -> Vec<String> {
    existing.extend(extra);
    existing.sort();
    existing.dedup();
    existing
}

pub(super) fn final_acceptance_gate_result(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    failed_inputs: usize,
    checked_inputs: usize,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let Some(required_task_ids) = authoritative_task_ids(task_universe) else {
        let mut result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: "final acceptance gate requires authoritative task universe".to_string(),
            ..WorkflowV2Result::default()
        };
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: "final_gate_missing_task_universe".to_string(),
            description: "generated decomposed PRD final gate cannot trust JS-supplied task IDs"
                .to_string(),
            severity: Some("blocking".to_string()),
        });
        return Ok(result);
    };
    let (completed, missing, artifact_gaps) = completion_ledger_state(
        v2_store,
        required_task_ids.iter().cloned().collect(),
        task_universe,
    )?;
    if failed_inputs == 0 && missing.is_empty() && artifact_gaps.is_empty() && checked_inputs > 0 {
        let mut result = WorkflowV2Result::accepted(format!(
            "final acceptance gate '{}' accepted {} authoritative task(s)",
            execution.call.id,
            completed.len()
        ));
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "final gate checked authoritative task universe and durable completion evidence ledger",
        ));
        result.data = serde_json::json!({
            "checked": checked_inputs,
            "completed_task_ids": completed,
        });
        return Ok(result);
    }
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!(
            "final acceptance gate '{}' needs review with missing or unsupported completion evidence",
            execution.call.id
        ),
        ..WorkflowV2Result::default()
    };
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: "final_gate_completion_evidence".to_string(),
        description: format!(
            "failed_inputs={failed_inputs}; missing_task_ids=[{}]; artifact_gaps=[{}]",
            missing.iter().cloned().collect::<Vec<_>>().join(", "),
            artifact_gaps.join(", ")
        ),
        severity: Some("blocking".to_string()),
    });
    result.data = serde_json::json!({
        "checked": checked_inputs,
        "completed_task_ids": completed,
        "missing_task_ids": missing,
        "artifact_gaps": artifact_gaps,
    });
    Ok(result)
}
