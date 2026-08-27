//! One disposition boundary for decomposition-time correctness gates.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_core::config::GateMode;
use serde::Serialize;

pub(crate) const OFF_MESSAGE: &str = "gate_mode=off — nothing was evaluated; this is not a pass\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateId {
    WorkflowLintTaskFile,
    WorkflowLintTaskSet,
    RequirementsTrace,
    FreezeAcceptance,
    FreezeSkeleton,
}

impl GateId {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WorkflowLintTaskFile => "workflow_lint.task_file",
            Self::WorkflowLintTaskSet => "workflow_lint.task_set",
            Self::RequirementsTrace => "requirements_trace",
            Self::FreezeAcceptance => "freeze_acceptance",
            Self::FreezeSkeleton => "freeze_skeleton",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateFinding {
    pub(crate) gate_id: GateId,
    pub(crate) text: String,
    pub(crate) subject: String,
    pub(crate) source_path: Option<PathBuf>,
    pub(crate) remediation_scope: archon_workflow::RemediationScope,
}

impl GateFinding {
    pub(crate) fn new(
        gate_id: GateId,
        text: impl Into<String>,
        subject: impl Into<String>,
        source_path: Option<PathBuf>,
        remediation_scope: archon_workflow::RemediationScope,
    ) -> Self {
        Self {
            gate_id,
            text: text.into(),
            subject: subject.into(),
            source_path,
            remediation_scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GateEvaluation {
    pub(crate) report: String,
    pub(crate) findings: Vec<GateFinding>,
    operational_error: Option<String>,
    publication_identity: Option<String>,
}

impl GateEvaluation {
    pub(crate) fn new(report: impl Into<String>, findings: Vec<GateFinding>) -> Self {
        Self {
            report: report.into(),
            findings,
            operational_error: None,
            publication_identity: None,
        }
    }

    pub(crate) fn with_operational_error(mut self, error: impl Into<String>) -> Self {
        self.operational_error = Some(error.into());
        self
    }

    pub(crate) fn with_publication_identity(mut self, identity: impl Into<String>) -> Self {
        self.publication_identity = Some(identity.into());
        self
    }

    pub(crate) fn into_envelope(self) -> Result<archon_workflow::GateEnvelopeV1> {
        let operational_error = self.operational_error.map(|text| {
            archon_workflow::GateOperationalError {
                kind: "gate_operational".to_string(),
                text,
            }
        });
        let policy_findings = self
            .findings
            .into_iter()
            .map(|finding| archon_workflow::GatePolicyFinding {
                text: finding.text,
                subject: finding.subject,
                source_path: finding
                    .source_path
                    .map(|path| path.to_string_lossy().replace('\\', "/")),
                remediation_scope: finding.remediation_scope,
            })
            .collect();
        Ok(archon_workflow::GateEnvelopeV1 {
            schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
            report: serde_json::Value::String(self.report),
            policy_findings,
            operational_error,
        })
    }
}

#[derive(Debug)]
pub(crate) struct GateDisposition {
    report: String,
    diagnostics: Vec<String>,
    blocking_error: Option<String>,
    permit: Option<GatePublicationPermit>,
}

#[derive(Debug)]
pub(crate) struct GatePublicationPermit {
    gate: GateId,
    finding_texts: Vec<String>,
    publication_identity: Option<String>,
}

impl GatePublicationPermit {
    pub(crate) fn authorizes(
        &self,
        gate: GateId,
        finding_texts: &[String],
        publication_identity: &str,
    ) -> bool {
        self.gate == gate
            && self.finding_texts == finding_texts
            && self.publication_identity.as_deref() == Some(publication_identity)
    }
}

impl GateDisposition {
    pub(crate) fn report(&self) -> &str {
        &self.report
    }

    pub(crate) fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub(crate) fn take_publication_permit(&mut self) -> Option<GatePublicationPermit> {
        self.permit.take()
    }

    pub(crate) fn is_blocked(&self) -> bool {
        self.blocking_error.is_some()
    }

    pub(crate) fn require_allowed(&self) -> Result<()> {
        match &self.blocking_error {
            Some(error) => Err(anyhow!(error.clone())),
            None => Ok(()),
        }
    }
}

#[derive(Serialize)]
struct ShadowRecord<'a> {
    schema_version: &'static str,
    gate_id: &'static str,
    finding: &'a str,
    subject: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_path: Option<String>,
    mode: &'static str,
    timestamp: String,
    binary_commit: &'static str,
}

pub(crate) fn finding_subject(text: &str, fallback: &str) -> String {
    text.split(|ch: char| ch.is_whitespace() || matches!(ch, ':' | ',' | ';'))
        .map(|token| token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-'))
        .find(|token| {
            ["TASK-", "REQ-", "AC-", "SUP-"]
                .iter()
                .any(|prefix| token.starts_with(prefix))
        })
        .filter(|token| !token.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

pub(crate) fn shadow_log_path(cwd: &Path) -> PathBuf {
    cwd.join(".archon/logs/workflow-gates-shadow.jsonl")
}

pub(crate) fn run_sync_gate<F>(
    cwd: &Path,
    mode: GateMode,
    gate: GateId,
    evaluate: F,
) -> Result<GateDisposition>
where
    F: FnOnce() -> Result<GateEvaluation>,
{
    if mode == GateMode::Off {
        return Ok(GateDisposition {
            report: OFF_MESSAGE.to_string(),
            diagnostics: Vec::new(),
            blocking_error: None,
            permit: None,
        });
    }
    let evaluation = evaluate()?;
    if evaluation
        .findings
        .iter()
        .any(|finding| finding.gate_id != gate)
    {
        return Err(anyhow!(
            "gate evaluation returned a finding for the wrong gate"
        ));
    }
    if let Some(error) = evaluation.operational_error {
        return Ok(GateDisposition {
            report: evaluation.report,
            diagnostics: Vec::new(),
            blocking_error: Some(error),
            permit: None,
        });
    }
    match mode {
        GateMode::Off => unreachable!("off returned before evaluation"),
        GateMode::Observe => {
            append_shadow_records(cwd, &evaluation.findings)?;
            Ok(GateDisposition {
                report: evaluation.report,
                diagnostics: evaluation
                    .findings
                    .iter()
                    .map(|finding| format!("[shadow] {}", finding.text))
                    .collect(),
                blocking_error: None,
                permit: Some(GatePublicationPermit {
                    gate,
                    finding_texts: evaluation
                        .findings
                        .iter()
                        .map(|finding| finding.text.clone())
                        .collect(),
                    publication_identity: evaluation.publication_identity,
                }),
            })
        }
        GateMode::Enforce if evaluation.findings.is_empty() => Ok(GateDisposition {
            report: evaluation.report,
            diagnostics: Vec::new(),
            blocking_error: None,
            permit: Some(GatePublicationPermit {
                gate,
                finding_texts: Vec::new(),
                publication_identity: evaluation.publication_identity,
            }),
        }),
        GateMode::Enforce => Ok(GateDisposition {
            report: evaluation.report,
            diagnostics: Vec::new(),
            blocking_error: Some(format!(
                "{} blocking finding(s):\n  {}",
                evaluation.findings.len(),
                evaluation
                    .findings
                    .iter()
                    .map(|finding| finding.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            )),
            permit: None,
        }),
    }
}

fn append_shadow_records(cwd: &Path, findings: &[GateFinding]) -> Result<()> {
    if findings.is_empty() {
        return Ok(());
    }
    let path = shadow_log_path(cwd);
    let parent = path.parent().expect("shadow log has a parent");
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "creating workflow gate shadow log directory {}",
            parent.display()
        )
    })?;
    let timestamp = chrono::Utc::now().to_rfc3339();
    let mut lines = Vec::with_capacity(findings.len());
    for finding in findings {
        let record = ShadowRecord {
            schema_version: "workflow-gate-shadow-v1",
            gate_id: finding.gate_id.as_str(),
            finding: &finding.text,
            subject: &finding.subject,
            source_path: finding
                .source_path
                .as_ref()
                .map(|path| path.to_string_lossy().replace('\\', "/")),
            mode: "observe",
            timestamp: timestamp.clone(),
            binary_commit: env!("ARCHON_GIT_HASH"),
        };
        lines.push(serde_json::to_string(&record)?);
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening workflow gate shadow log {}", path.display()))?;
    append_serialized_records(&mut file, &lines)
        .with_context(|| format!("writing workflow gate shadow log {}", path.display()))
}

fn append_serialized_records(writer: &mut impl Write, records: &[String]) -> Result<()> {
    for record in records {
        let mut line = Vec::with_capacity(record.len() + 1);
        line.extend_from_slice(record.as_bytes());
        line.push(b'\n');
        let written = writer.write(&line)?;
        if written != line.len() {
            return Err(anyhow!(
                "short write while appending one shadow JSONL record: wrote {written} of {} bytes; inspect the log tail before retrying",
                line.len()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "workflow_gate_tests.rs"]
mod tests;
