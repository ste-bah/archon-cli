//! Status detail extension for fixed decomposition runs.

use anyhow::{Context, Result};
use archon_workflow::{
    DecompositionPhase, FixedDecompositionStateV1, SubjectDisposition, WorkflowStore,
    WorkflowV2ResultStore,
};

pub(crate) fn render(store: &WorkflowStore, run_id: &str) -> Result<Option<String>> {
    let path = store
        .run_dir(run_id)
        .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let state: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(&path)
            .with_context(|| format!("reading fixed decomposition status {}", path.display()))?,
    )
    .with_context(|| format!("parsing fixed decomposition status {}", path.display()))?;
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"));
    let checkpoint = v2_store.load_checkpoint()?.unwrap_or_default();
    let records = v2_store.load_call_records()?;
    let mut out = String::from("\nfixed decomposition:\n");
    out.push_str("run_kind: fixed_decomposition_v1\n");
    out.push_str(&format!(
        "template_version: {}\nstarting_binary_revision: {}\nscript_digest: {}\ncatalog_digest: {}\n",
        state.identity.template_version,
        state.identity.starting_binary_revision,
        state.identity.script_digest,
        state.identity.catalog_digest,
    ));
    out.push_str(&format!(
        "project_root: {}\nprd: {}\ntask_root: {}\nphase: {}\nlog_path: {}\nresume_eligible_calls: {}\n",
        state.identity.project_root_identity,
        state.identity.prd_identity,
        state.identity.task_root_identity,
        phase_label(state.phase),
        state.log_path,
        checkpoint.completed_call_ids.len(),
    ));
    let observer_records = store
        .run_dir(run_id)
        .join("observer/run-end-acceptance.jsonl");
    out.push_str(&format!(
        "observer_state: {}\n",
        if observer_records.exists() {
            "run-end acceptance observed"
        } else {
            "not started"
        }
    ));
    append_provider_route(store, run_id, &mut out)?;
    append_call_summary(&records, &mut out);
    if !state.attempts.is_empty() {
        out.push_str("attempts:\n");
        for (subject, attempt) in state.attempts {
            let budget = phase_attempt_budget(state.phase)
                .map_or_else(|| "none".to_string(), |value| value.to_string());
            out.push_str(&format!(
                "- {subject} attempt={}/{budget} interrupted={} last_error={}\n",
                attempt.logical_attempt,
                attempt.interrupted,
                attempt.last_error.as_deref().unwrap_or("none")
            ));
        }
    }
    if !state.dispositions.is_empty() {
        let mut pending = 0usize;
        let mut done = 0usize;
        let mut with_shadows = 0usize;
        let mut other = 0usize;
        for disposition in state.dispositions.values() {
            match disposition {
                SubjectDisposition::Pending => pending += 1,
                SubjectDisposition::Accepted => done += 1,
                SubjectDisposition::AcceptedWithShadowFindings => with_shadows += 1,
                _ => other += 1,
            }
        }
        out.push_str(&format!(
            "subject_totals: pending={pending} accepted={done} accepted_with_shadow_findings={with_shadows} other={other}\n"
        ));
        out.push_str("dispositions:\n");
        for (subject, disposition) in state.dispositions {
            out.push_str(&format!("- {subject}={}\n", disposition_label(disposition)));
        }
    }
    Ok(Some(out))
}

/// Author attempt budgets, mirrored from the fixed script so status can report
/// "attempt 3 of 6" rather than a bare attempt number.
///
/// `fixed_script_budgets_match_the_mirror` fails if the script's constants ever
/// diverge from these.
pub(crate) fn phase_attempt_budget(phase: DecompositionPhase) -> Option<u32> {
    match phase {
        DecompositionPhase::Acceptance => Some(6),
        DecompositionPhase::Skeleton => Some(6),
        DecompositionPhase::Bodies => Some(10),
        _ => None,
    }
}

fn elapsed_secs(from: &str, to: Option<&str>) -> Option<i64> {
    let start = chrono::DateTime::parse_from_rfc3339(from).ok()?;
    let end = match to {
        Some(value) if !value.is_empty() => chrono::DateTime::parse_from_rfc3339(value).ok()?,
        _ => chrono::Utc::now().into(),
    };
    Some((end - start).num_seconds())
}

fn phase_label(phase: DecompositionPhase) -> &'static str {
    match phase {
        DecompositionPhase::Identity => "identity",
        DecompositionPhase::Acceptance => "acceptance",
        DecompositionPhase::Skeleton => "skeleton",
        DecompositionPhase::Bodies => "bodies",
        DecompositionPhase::SetGates => "set_gates",
        DecompositionPhase::Reconciliation => "reconciliation",
        DecompositionPhase::Completed => "completed",
    }
}

fn disposition_label(disposition: SubjectDisposition) -> &'static str {
    match disposition {
        SubjectDisposition::Pending => "pending",
        SubjectDisposition::Accepted => "accepted",
        SubjectDisposition::AcceptedWithShadowFindings => "accepted_with_shadow_findings",
        SubjectDisposition::Failed => "failed",
        SubjectDisposition::Blocked => "blocked",
        SubjectDisposition::Interrupted => "interrupted",
    }
}

fn append_provider_route(store: &WorkflowStore, run_id: &str, out: &mut String) -> Result<()> {
    let path = store
        .run_dir(run_id)
        .join(crate::command::workflow_decompose::FIXED_PROVIDER_ROUTE_PATH);
    if !path.exists() {
        return Ok(());
    }
    let route: crate::command::workflow_provider_route::TrustedProviderRouteSnapshot =
        serde_json::from_slice(
            &std::fs::read(&path)
                .with_context(|| format!("reading fixed provider route {}", path.display()))?,
        )
        .with_context(|| format!("parsing fixed provider route {}", path.display()))?;
    out.push_str(&format!(
        "provider_route: {} digest={}\n",
        route.origin,
        route.endpoint_digest.as_deref().unwrap_or("none")
    ));
    Ok(())
}

fn append_call_summary(records: &[archon_workflow::WorkflowV2CallRecord], out: &mut String) {
    let authors = records
        .iter()
        .filter(|record| record.call.method == archon_workflow::WorkflowV2HostMethod::Agent)
        .count();
    let bodies = records
        .iter()
        .filter(|record| {
            record.call.method == archon_workflow::WorkflowV2HostMethod::Agent
                && record.call.id.starts_with("body-")
        })
        .count();
    let host_commands = records
        .iter()
        .filter(|record| record.call.method == archon_workflow::WorkflowV2HostMethod::HostCommand)
        .count();
    let shadow_findings = records.iter().map(host_finding_count).sum::<usize>();
    let mut accepted = 0usize;
    let mut interrupted = 0usize;
    let mut failed = 0usize;
    for record in records {
        match record.status {
            archon_workflow::WorkflowV2Status::Accepted
            | archon_workflow::WorkflowV2Status::Noop => accepted += 1,
            archon_workflow::WorkflowV2Status::Cancelled => interrupted += 1,
            archon_workflow::WorkflowV2Status::Failed
            | archon_workflow::WorkflowV2Status::Blocked => failed += 1,
            _ => {}
        }
    }
    out.push_str(&format!(
        "calls: total={} authors={authors} bodies={bodies} host_commands={host_commands}\ncall_status: accepted={accepted} interrupted={interrupted} failed={failed}\nshadow_findings: {shadow_findings}\n",
        records.len()
    ));
    if let Some(active) = records.iter().find(|record| {
        matches!(
            record.status,
            archon_workflow::WorkflowV2Status::Pending | archon_workflow::WorkflowV2Status::Running
        )
    }) {
        let elapsed = elapsed_secs(&active.started_at, None);
        out.push_str(&format!(
            "active_call: {} method={} attempt={} elapsed_secs={}\n",
            active.call.id,
            active.call.method.as_str(),
            active.attempt,
            elapsed.map_or_else(|| "unknown".to_string(), |value| value.to_string())
        ));
        if let Some(request) = &active.call.options.host_command {
            out.push_str(&format!("active_capability: {}\n", request.command_id));
            if let Some(elapsed) = elapsed {
                if let Ok(catalog) =
                    crate::command::workflow_host_command_catalog::fixed_decomposition_catalog(
                        env!("ARCHON_GIT_HASH"),
                    )
                {
                    if let Some(capability) = catalog.capabilities.get(&request.command_id) {
                        out.push_str(&format!(
                            "active_remaining_secs: {}\n",
                            (capability.timeout_secs as i64 - elapsed).max(0)
                        ));
                    }
                }
            }
        } else {
            // Author dispatches carry the provider backstop rather than a
            // capability timeout.
            out.push_str(&format!(
                "active_model: {}\n",
                crate::command::workflow_live::workflow_live_runner::tier_model_alias(
                    archon_workflow::ProviderTier::Planner
                )
            ));
            if let Some(elapsed) = elapsed {
                out.push_str(&format!(
                    "active_remaining_secs: {}\n",
                    (1_500i64 - elapsed).max(0)
                ));
            }
        }
    } else {
        out.push_str("active_call: none\n");
    }
    if let Some(last_error) = records
        .iter()
        .rev()
        .find(|record| {
            matches!(
                record.status,
                archon_workflow::WorkflowV2Status::Failed
                    | archon_workflow::WorkflowV2Status::Blocked
                    | archon_workflow::WorkflowV2Status::Cancelled
            )
        })
        .map(|record| record.result.summary.as_str())
    {
        out.push_str(&format!("last_error: {}\n", one_line(last_error, 180)));
    } else {
        out.push_str("last_error: none\n");
    }
}

fn host_finding_count(record: &archon_workflow::WorkflowV2CallRecord) -> usize {
    if record.call.method != archon_workflow::WorkflowV2HostMethod::HostCommand {
        return 0;
    }
    serde_json::from_value::<archon_workflow::HostCommandResult>(record.result.data.clone())
        .ok()
        .and_then(|outcome| outcome.gate_envelope)
        .map_or(0, |envelope| envelope.policy_findings.len())
}

fn one_line(value: &str, max_chars: usize) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::phase_attempt_budget;
    use archon_workflow::DecompositionPhase;

    /// Status reports "attempt 3 of 6" by mirroring budgets the fixed script
    /// owns. A mirror that drifts silently reports a wrong budget, so this
    /// fails the moment the script and the mirror disagree.
    #[test]
    fn fixed_script_budgets_match_the_mirror() {
        let source = crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE;
        for (constant, phase) in [
            ("ACCEPTANCE_ATTEMPTS", DecompositionPhase::Acceptance),
            ("SKELETON_ATTEMPTS", DecompositionPhase::Skeleton),
            ("BODY_ATTEMPTS", DecompositionPhase::Bodies),
        ] {
            let needle = format!("const {constant} = ");
            let start = source
                .find(&needle)
                .unwrap_or_else(|| panic!("{constant} is declared in the fixed script"))
                + needle.len();
            let value: u32 = source[start..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or_else(|error| panic!("{constant} is numeric: {error}"));
            assert_eq!(
                Some(value),
                phase_attempt_budget(phase),
                "{constant} drifted from the status mirror"
            );
        }
    }
}
