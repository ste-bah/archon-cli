//! Deterministic input and set-coverage verdict for requirement trace.

use std::path::PathBuf;

use anyhow::Result;
#[cfg(test)]
use anyhow::anyhow;
use archon_knowledge::traceability::TraceReport;

use super::render;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TracePolicyFinding {
    pub(super) text: String,
    pub(super) subject: String,
    pub(super) source_path: PathBuf,
    pub(super) remediation_scope: archon_workflow::RemediationScope,
}

pub(super) fn policy_findings(
    report: &TraceReport,
    prd_findings: Vec<String>,
) -> Vec<TracePolicyFinding> {
    let prd_path = PathBuf::from(&report.prd_path);
    let task_dir = PathBuf::from(&report.task_dir);
    let mut findings = prd_findings
        .into_iter()
        .map(|text| TracePolicyFinding {
            subject: crate::command::workflow_gate::finding_subject(&text, "PRD"),
            text,
            source_path: prd_path.clone(),
            remediation_scope: archon_workflow::RemediationScope::PrdInput,
        })
        .collect::<Vec<_>>();
    if report.coverage.requirements_total == 0 {
        findings.push(TracePolicyFinding {
            text: format!(
                "PRD {} defines zero obligations; add a line-leading REQ-<AREA>-<NNN> bullet or a row under an obligation table, then re-run the same trace command",
                report.prd_path
            ),
            subject: "PRD".into(),
            source_path: prd_path.clone(),
            remediation_scope: archon_workflow::RemediationScope::PrdInput,
        });
    }
    if report.coverage.citations_total == 0 {
        findings.push(TracePolicyFinding {
            text: format!(
                "task directory {} cites zero obligations; add each owned PRD ID to at least one TASK file's implements list, then re-run the same trace command",
                report.task_dir
            ),
            subject: "task directory".into(),
            source_path: task_dir,
            remediation_scope: archon_workflow::RemediationScope::Skeleton,
        });
    }
    for phantom in &report.coverage.phantom {
        findings.push(TracePolicyFinding {
            text: format!(
                "task '{}' cites unknown obligation '{}' in {}; remove it from that TASK file's implements list or correct it to an ID defined by the PRD",
                phantom.task_id, phantom.cited_id, phantom.source_path
            ),
            subject: phantom.task_id.clone(),
            source_path: PathBuf::from(&phantom.source_path),
            remediation_scope: archon_workflow::RemediationScope::Skeleton,
        });
    }
    for obligation in &report.coverage.unclaimed {
        findings.push(TracePolicyFinding {
            text: format!(
                "PRD obligation '{obligation}' is claimed by no task; add it to at least one TASK file's implements list or remove or correct the obligation in the PRD"
            ),
            subject: obligation.clone(),
            source_path: prd_path.clone(),
            remediation_scope: archon_workflow::RemediationScope::Skeleton,
        });
    }
    findings.extend(failed_claims(report));
    findings.sort_by(|left, right| {
        (&left.text, &left.source_path).cmp(&(&right.text, &right.source_path))
    });
    findings.dedup();
    findings
}

/// Batch O: every claim a falsification run refuted -- its verifier still
/// passed with the anchored code replaced by an abort -- goes back to the
/// body of the task that made it. A claim no experiment decided raises
/// nothing here: only a refutation is evidence against it.
fn failed_claims(report: &TraceReport) -> Vec<TracePolicyFinding> {
    use archon_knowledge::traceability::FalsificationOutcome;
    let task_dir = PathBuf::from(&report.task_dir);
    report
        .rows
        .iter()
        .flat_map(|row| row.anchors.iter().map(move |verdict| (row, verdict)))
        .filter(|(_, verdict)| {
            matches!(
                verdict.falsification_outcome,
                Some(FalsificationOutcome::EdgeIsDecoration { .. })
            )
        })
        .map(|(row, verdict)| {
            let task = &verdict.anchor.task_id;
            let command = verdict
                .falsification
                .as_ref()
                .map(|plan| plan.command.clone())
                .unwrap_or_default();
            TracePolicyFinding {
                text: format!(
                    "task {task}: its claim of '{}' is not proven: with {} replaced by an abort, its verifier `{command}` still passed, so that code and test do not carry the obligation; re-author the task body so its implements, Files Expected to Change and Focused Tests name code and a test that do",
                    row.requirement_id,
                    verdict.anchor.citation()
                ),
                subject: task.clone(),
                source_path: task_dir.join(format!("{task}.md")),
                remediation_scope: archon_workflow::RemediationScope::Body,
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TraceVerdict {
    pub(super) report: String,
    pub(super) blocking_findings: Vec<String>,
}

impl TraceVerdict {
    #[cfg(test)]
    pub(crate) fn require_clean(&self) -> Result<()> {
        if self.blocking_findings.is_empty() {
            return Ok(());
        }
        Err(anyhow!(
            "requirement trace has {} blocking finding(s):\n  {}; make each exact edit and re-run the same `requirements trace` command",
            self.blocking_findings.len(),
            self.blocking_findings.join("\n  ")
        ))
    }
}

#[cfg(test)]
pub(super) fn blocking_findings(
    report: &TraceReport,
    mut input_findings: Vec<String>,
) -> Vec<String> {
    let task_population_complete = input_findings.is_empty();
    if report.coverage.requirements_total == 0 {
        input_findings.push(format!(
            "PRD {} defines zero obligations; add a line-leading REQ-<AREA>-<NNN> bullet or a row under an obligation table, then re-run the same trace command",
            report.prd_path
        ));
    }
    if task_population_complete {
        if report.coverage.citations_total == 0 {
            input_findings.push(format!(
                "task directory {} cites zero obligations; add each owned PRD ID to at least one TASK file's implements list, then re-run the same trace command",
                report.task_dir
            ));
        }
        for phantom in &report.coverage.phantom {
            input_findings.push(format!(
                "task '{}' cites unknown obligation '{}' in {}; remove it from that TASK file's implements list or correct it to an ID defined by the PRD",
                phantom.task_id, phantom.cited_id, phantom.source_path
            ));
        }
        for obligation in &report.coverage.unclaimed {
            input_findings.push(format!(
                "PRD obligation '{obligation}' is claimed by no task; add it to at least one TASK file's implements list or remove or correct the obligation in the PRD"
            ));
        }
    }
    input_findings.sort();
    input_findings.dedup();
    input_findings
}

pub(super) fn render_verdict(
    report: &TraceReport,
    json: bool,
    task_population_complete: bool,
    blocking_findings: Vec<String>,
) -> Result<TraceVerdict> {
    let mut rendered = if json {
        serde_json::to_string_pretty(report)?
    } else if task_population_complete {
        render::report(report)
    } else {
        format!(
            "Requirement traceability\n  PRD:   {}\n  Tasks: {}\n\nCoverage: NOT COMPUTED because the task population is incomplete or unreadable.\n",
            report.prd_path, report.task_dir
        )
    };
    if !json && !blocking_findings.is_empty() {
        rendered.push_str("\nBLOCKING trace input/coverage findings:\n");
        for finding in &blocking_findings {
            rendered.push_str(&format!("  {finding}\n"));
        }
    }
    Ok(TraceVerdict {
        report: rendered,
        blocking_findings,
    })
}

#[cfg(test)]
#[path = "verdict_tests.rs"]
mod tests;
