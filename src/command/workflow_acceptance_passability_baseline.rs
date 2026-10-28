use super::*;

/// The probe's findings, and one more for each accepted check of `contract`
/// that failed on the baseline in a way it cannot pass as written; each
/// such check is refuted in `contract` (see the module docs).
pub(crate) async fn judge_baseline_failures(
    project_root: &Path,
    repository_root: &Path,
    tasks_root: &Path,
    prd_text: &str,
    client: &dyn WorkflowLlmClient,
    contract: &mut AcceptanceContract,
    probed: PreImplementation,
    resume: &FreezeResume,
) -> Result<Vec<GateFinding>> {
    let mut findings = probed.findings;
    let baseline = probed.baseline;
    // Deliver already proven authored defects before any fallible host observation.
    if findings
        .iter()
        .any(|finding| finding.remediation_scope != archon_workflow::RemediationScope::Operational)
    {
        return Ok(findings);
    }
    // No baseline means every probed check is already unproven, the host's.
    let Some(runs) = baseline else {
        return Ok(findings);
    };
    // A check the probe already sends back is its author's (or the host's).
    let flagged: BTreeSet<&str> = findings.iter().map(|f| f.subject.as_str()).collect();
    let ids: BTreeSet<String> = entries(contract)
        .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
        .filter(|entry| !flagged.contains(entry.id.as_str()))
        .map(|entry| entry.id.clone())
        .collect();
    let requirements =
        archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(prd_text);
    let redactor = Redactor::for_runs(&runs);
    let candidates = candidates(
        client,
        crate::command::workflow_task_set::judge::JUDGE_MODEL_ALIAS,
        contract,
        &ids,
        &runs,
        &requirements,
        &redactor,
    );
    if candidates.is_empty() {
        return Ok(findings);
    }
    let store = (resume.persist).then(|| Store {
        dir: project_root.join(FREEZE_CACHE_DIR).join("passability"),
    });
    let mut refuted = BTreeMap::new();
    let mut pending = Vec::new();
    for candidate in candidates {
        let Some(command) = usage::check_command(contract, &candidate.id) else {
            pending.push(candidate);
            continue;
        };
        if let Some((program, path, rejected_line)) = usage::external_usage_error(
            &runs,
            &candidate.id,
            repository_root,
            project_root,
            command,
        ) {
            let rejected_line = evidence::inert(&rejected_line);
            let rule = format!(
                "{program} ({}, outside the repository) rejected the check's command line: {rejected_line}",
                path.display()
            );
            let reason = format!(
                "nothing the repository implements can change how {program} parses its arguments; fix the invocation"
            );
            let decision = Decision {
                verdict: JudgeDecision::Refuted,
                counterexample: rejected_line,
                reason: reason.clone(),
                sampling: Some(serde_json::json!({"provenance": "host"})),
            };
            if let Some(store) = store.as_ref() {
                audit::write(
                    &store.dir,
                    &candidate.key,
                    &candidate.id,
                    &decision,
                    candidate.evidence.exit_code,
                )
                .with_context(|| format!("writing passability audit for check {}", candidate.id))?;
            }
            let text = evidence::finding_text(
                &candidate.id,
                CANNOT_PASS,
                &candidate.evidence,
                &rule,
                &reason,
            );
            refuted.insert(candidate.id.clone(), (decision, text));
        } else {
            pending.push(candidate);
        }
    }
    if refuted.is_empty() && !pending.is_empty() {
        refuted.extend(
            assess(
                client,
                crate::command::workflow_task_set::judge::JUDGE_MODEL_ALIAS,
                contract,
                &pending,
                &redactor,
                store.as_ref(),
                resume,
                None,
            )
            .await?,
        );
    }
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    for (id, (decision, text)) in refuted {
        let entry = (contract.acceptance.iter_mut())
            .chain(&mut contract.supplementary)
            .find(|entry| entry.id == id)
            .expect("a candidate is an entry of the contract");
        entry.judgment.verdict = JudgeDecision::Refuted;
        entry.judgment.reason = format!("{CANNOT_PASS}: {}", decision.reason);
        entry
            .judgment
            .counterexample
            .clone_from(&decision.counterexample);
        entry.judgment.host_call_id = if decision
            .sampling
            .as_ref()
            .and_then(|value| value.get("provenance"))
            .and_then(serde_json::Value::as_str)
            == Some("host")
        {
            "host:argument-parser-usage".to_string()
        } else {
            format!("acceptance-passability-batch:{id}")
        };
        entry.judgment.sampling.clone_from(&decision.sampling);
        findings.push(GateFinding::new(
            GateId::FreezeAcceptance,
            text,
            id,
            Some(contract_path.clone()),
            archon_workflow::RemediationScope::CandidateArtifact,
        ));
    }
    Ok(findings)
}
