use super::*;

#[cfg(test)]
pub(crate) fn evaluate_lint(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
) -> Result<crate::command::workflow_gate::GateEvaluation> {
    let site = crate::command::acceptance_check_policy::command_site_path(cwd)?;
    evaluate_lint_with_path(cwd, source, mode, &site)
}

pub(crate) fn evaluate_lint_with_path(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
    site_path: &std::ffi::OsStr,
) -> Result<crate::command::workflow_gate::GateEvaluation> {
    let _read = chain_read(cwd, source)?;
    preflight::operational_input(cwd, source)?;
    let mut fault_classes = Vec::new();
    let graph_error = if matches!(source, LintSource::Tasks(_)) {
        load_graph(cwd, source).err().map(|error| {
            let text = crate::command::workflow_host_fault::diagnostic(&error);
            fault_classes.push(crate::command::workflow_host_fault::classify(&error));
            format!(
                "task graph could not be lowered: {text}; correct the named depends_on/blocks declaration or task status, then re-run `workflow lint --tasks <DIR>`"
            )
        })
    } else {
        None
    };
    let report = run_lint_with_mode_at_path(cwd, source, mode, site_path)?;
    let gate_id = match source {
        LintSource::TaskFile(_) => crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        _ => crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
    };
    let source_path = match source {
        LintSource::TaskFile(path) | LintSource::Tasks(path) | LintSource::Spec(path) => {
            Some(absolute(cwd, path))
        }
        LintSource::Graph(_) => None,
    };
    let subject = describe(source);
    let mut deterministic = std::collections::BTreeMap::new();
    let mut contract_defects = Vec::new();
    let (base_findings, inherited_findings) = match source {
        LintSource::TaskFile(path) => {
            let lint = task_file::inspect_at_path(cwd, path, mode, Some(site_path));
            deterministic = lint.deterministic;
            (lint.blockers, lint.inherited_blockers)
        }
        _ => {
            let root = match source {
                LintSource::Tasks(path) => Some(absolute(cwd, path)),
                LintSource::Spec(_) | LintSource::Graph(_) | LintSource::TaskFile(_) => None,
            };
            // Kept with their identities, never re-derived from message text.
            contract_defects = contracts::blocking_defects(root.as_deref());
            let mut blockers = Vec::new();
            let mut inherited = std::collections::BTreeSet::new();
            if let Some(root) = root.as_deref() {
                let lint = task_set::inspect(cwd, root, mode)?;
                blockers.extend(lint.blockers);
                deterministic = lint.deterministic;
                inherited = lint.inherited_blockers;
            }
            blockers.extend(
                declarations::tasks_without_a_runnable_test_at_path(root.as_deref(), site_path)
                    .into_iter()
                    .map(|task| declarations::missing_runnable_test_finding(&task)),
            );
            (blockers, inherited)
        }
    };
    let default_scope = match source {
        LintSource::TaskFile(_) => archon_workflow::RemediationScope::Body,
        _ => archon_workflow::RemediationScope::Skeleton,
    };
    let mut findings = base_findings
        .into_iter()
        .map(|text| {
            let remediation_scope = if inherited_findings.contains(&text) {
                archon_workflow::RemediationScope::InheritedPredecessor
            } else {
                default_scope
            };
            let finding_subject = crate::command::workflow_gate::finding_subject(&text, &subject);
            let identity = deterministic
                .get_mut(&text)
                .and_then(|identities| identities.pop_front());
            let mut finding = crate::command::workflow_gate::GateFinding::new(
                gate_id,
                text,
                finding_subject,
                source_path.clone(),
                remediation_scope,
            );
            finding.deterministic_defect = identity;
            let root = source_path.as_deref().map(|path| match source {
                LintSource::TaskFile(_) => path.parent().unwrap_or(cwd),
                _ => path,
            });
            finding_scope::route(&mut finding, root);
            finding
        })
        .collect::<Vec<_>>();
    let path = source_path.as_deref();
    let gate = (gate_id, subject.as_str(), path, default_scope);
    findings.extend(contracts::gate_findings(contract_defects, gate));
    let coverage_root = match source {
        LintSource::TaskFile(_) => None,
        LintSource::Tasks(path) => Some(absolute(cwd, path)),
        LintSource::Spec(_) | LintSource::Graph(_) => None,
    };
    let mut repository_error = None;
    if let Some(root) = coverage_root.as_deref() {
        findings.extend(tool_obligations::set_findings(cwd, root));
        findings.extend(scope_declarations::set_findings(root));
        match prd_tool_coverage::set_findings(cwd, root) {
            Ok(tools) => findings.extend(tools),
            Err(error) => {
                fault_classes.push(crate::command::workflow_host_fault::classify(&error));
                repository_error = Some(format!(
                    "prd tool coverage failed: {}",
                    crate::command::workflow_host_fault::diagnostic(&error)
                ));
            }
        }
        match repository_claims::set_findings(cwd, root) {
            Ok(claims) => findings.extend(claims),
            Err(error) => {
                fault_classes.push(crate::command::workflow_host_fault::classify(&error));
                let text = format!(
                    "repository claim check failed: {}",
                    crate::command::workflow_host_fault::diagnostic(&error)
                );
                repository_error = Some(
                    repository_error.map_or(text.clone(), |existing| format!("{existing}; {text}")),
                );
            }
        }
        match owner_coverage::set_findings(root) {
            Ok(owners) => findings.extend(owners),
            Err(error) => {
                fault_classes.push(crate::command::workflow_host_fault::classify(&error));
                let text = format!(
                    "repository owner coverage failed: {}",
                    crate::command::workflow_host_fault::diagnostic(&error)
                );
                repository_error = Some(match repository_error {
                    Some(existing) => format!("{existing}; {text}"),
                    None => text,
                });
            }
        }
        match focused_test_files::set_findings_at_path(root, site_path) {
            Ok(owned) => findings.extend(owned),
            Err(error) => {
                fault_classes.push(crate::command::workflow_host_fault::classify(&error));
                let text = format!(
                    "focused test file ownership failed: {}",
                    crate::command::workflow_host_fault::diagnostic(&error)
                );
                repository_error = Some(match repository_error {
                    Some(existing) => format!("{existing}; {text}"),
                    None => text,
                });
            }
        }
    }
    findings.extend(
        coverage::policy_findings(coverage_root.as_deref())
            .into_iter()
            .map(|finding| {
                let mut gate = crate::command::workflow_gate::GateFinding::new(
                    gate_id,
                    finding.text,
                    finding.subject,
                    Some(finding.source_path),
                    finding.remediation_scope,
                );
                if let Some(identity) = finding.identity {
                    gate = gate.with_defect(identity);
                }
                gate
            }),
    );
    coverage::deduplicate(&mut findings);
    let evaluation = crate::command::workflow_gate::GateEvaluation::new(report, findings);
    let operational = match (graph_error, repository_error) {
        (Some(graph), Some(repository)) => Some(format!("{graph}; {repository}")),
        (graph, repository) => graph.or(repository),
    };
    Ok(match operational {
        Some(error) => evaluation
            .with_operational_error(error)
            .with_operational_error_kind(
                if fault_classes
                    .iter()
                    .all(|class| *class == crate::command::workflow_host_fault::Class::Transient)
                {
                    "transient_host_fault"
                } else {
                    "permanent_host_fault"
                },
            ),
        None => evaluation,
    })
}
