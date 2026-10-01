//! Disposition-facing requirement trace evaluation.

use std::path::Path;

use anyhow::Result;

use super::{TraceOptions, build_report_with_input_findings, falsify, persist, verdict};

/// Who an unreadable task file belongs to.
///
/// The interactive gate reads whatever the operator points at, so a file it
/// cannot parse is the gate failing to compute. The staged gate reads the
/// bodies the fixed decomposition has just published, so the same file is the
/// author's artifact being wrong — and reporting it operationally would stop
/// the run even in observe mode, naming no file to repair.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnreadableInputs {
    AreOperational,
    BelongToTheirAuthor,
}

pub(crate) fn evaluate_trace(
    cwd: &Path,
    options: &TraceOptions,
) -> Result<crate::command::workflow_gate::GateEvaluation> {
    evaluate(cwd, options, UnreadableInputs::AreOperational)
}

pub(crate) fn evaluate_trace_for_published_bodies(
    cwd: &Path,
    options: &TraceOptions,
) -> Result<crate::command::workflow_gate::GateEvaluation> {
    evaluate(cwd, options, UnreadableInputs::BelongToTheirAuthor)
}

fn evaluate(
    cwd: &Path,
    options: &TraceOptions,
    unreadable: UnreadableInputs,
) -> Result<crate::command::workflow_gate::GateEvaluation> {
    let (mut report, input_findings, prd_findings) =
        build_report_with_input_findings(cwd, options)?;
    if !input_findings.is_empty() {
        let error = format!(
            "traceability input error:\n  {}",
            input_findings.join("\n  ")
        );
        let mut rendered =
            verdict::render_verdict(&report, options.json, false, input_findings.clone())?;
        if options.json {
            rendered.report.push_str("\n");
        } else {
            rendered.report.push_str("\nExcluded task files:\n");
            for finding in &input_findings {
                rendered.report.push_str(&format!("  {finding}\n"));
            }
        }
        if unreadable == UnreadableInputs::AreOperational {
            return Ok(crate::command::workflow_gate::GateEvaluation::new(
                rendered.report,
                Vec::new(),
            )
            .with_operational_error(error));
        }
        let typed = input_findings
            .into_iter()
            .map(|text| {
                let subject =
                    crate::command::workflow_gate::finding_subject(&text, "requirements trace");
                crate::command::workflow_gate::GateFinding::new(
                    crate::command::workflow_gate::GateId::RequirementsTrace,
                    text,
                    subject,
                    None,
                    archon_workflow::RemediationScope::Body,
                )
            })
            .collect();
        return Ok(crate::command::workflow_gate::GateEvaluation::new(
            rendered.report,
            typed,
        ));
    }
    if options.falsify {
        falsify::execute_plans(cwd, &mut report);
    }
    if let Some(store_path) = &options.persist {
        persist(cwd, store_path, &report)?;
    }
    let mut policy_findings = verdict::policy_findings(&report, prd_findings);
    // PLAN-2: published bodies are a decomposition's, and at decomposition
    // there is no code index for the mutation experiments to anchor in. Every
    // claim is tested instead against the repository the task set records;
    // a refuted or untestable claim is a finding on its own body.
    let claim_trace = if unreadable == UnreadableInputs::BelongToTheirAuthor {
        let task_dir = super::absolute(cwd, &options.tasks);
        let (bindings, _) = super::load_bindings_with_findings(&task_dir)?;
        let trace = super::claims::falsify_claims(&task_dir, bindings)?;
        policy_findings.extend(trace.findings);
        policy_findings.sort_by(|left, right| {
            (&left.text, &left.source_path).cmp(&(&right.text, &right.source_path))
        });
        policy_findings.dedup();
        Some(trace.report)
    } else {
        None
    };
    let mut rendered = verdict::render_verdict(
        &report,
        options.json,
        true,
        policy_findings
            .iter()
            .map(|finding| finding.text.clone())
            .collect(),
    )?;
    if let Some(section) = claim_trace {
        rendered.report.push_str(&section);
    }
    let typed = policy_findings
        .into_iter()
        .map(|finding| {
            crate::command::workflow_gate::GateFinding::new(
                crate::command::workflow_gate::GateId::RequirementsTrace,
                finding.text,
                finding.subject,
                Some(finding.source_path),
                finding.remediation_scope,
            )
        })
        .collect();
    Ok(crate::command::workflow_gate::GateEvaluation::new(
        rendered.report,
        typed,
    ))
}
