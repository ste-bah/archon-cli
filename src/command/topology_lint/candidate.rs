//! Candidate-byte task-file evaluation for fixed-decomposition body landing.

use std::path::Path;

use anyhow::{Context, Result};

pub(crate) fn evaluate_task_file_candidate(
    cwd: &Path,
    path: &Path,
    candidate: &[u8],
    mode: archon_core::config::GateMode,
) -> Result<crate::command::workflow_gate::GateEvaluation> {
    let path = super::absolute(cwd, path);
    let _read = super::chain_read(cwd, &super::LintSource::TaskFile(path.clone()))?;
    super::preflight::task_file_freeze(cwd, &path)?;
    let raw = std::str::from_utf8(candidate)
        .context("candidate TASK body is not UTF-8; return one complete UTF-8 TASK file")?;
    let mut lint = super::task_file::inspect_raw(cwd, &path, raw, mode);
    let subject = format!("task file {}", path.display());
    let mut findings: Vec<_> = lint
        .blockers
        .into_iter()
        .map(|text| {
            let remediation_scope = if lint.inherited_blockers.contains(&text) {
                archon_workflow::RemediationScope::InheritedPredecessor
            } else {
                archon_workflow::RemediationScope::Body
            };
            let mut finding = crate::command::workflow_gate::GateFinding::new(
                crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                text.clone(),
                crate::command::workflow_gate::finding_subject(&text, &subject),
                Some(path.clone()),
                remediation_scope,
            );
            finding.deterministic_defect = lint
                .deterministic
                .get_mut(&text)
                .and_then(|identities| identities.pop_front());
            finding
        })
        .collect();
    let mut report = lint.report;
    // Issue 248: the log-redaction marker as a word is a redacted copy read
    // back as data; such a body never lands.
    // Issue 261: a host finding, so it carries a staged host identity.
    for (index, text) in redaction_marker_lines(raw).into_iter().enumerate() {
        report.push_str(&format!("  BLOCKING {text}\n"));
        let identity = archon_workflow::defect::DeterministicDefect::new(
            "redacted_executable_value",
            "task_file",
            format!("redaction/{index}"),
        );
        findings.push(
            crate::command::workflow_gate::GateFinding::new(
                crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                text.clone(),
                crate::command::workflow_gate::finding_subject(&text, &subject),
                Some(path.clone()),
                archon_workflow::RemediationScope::Body,
            )
            .with_defect(identity),
        );
    }
    // Issue-55: the body's claims about repository paths, checked against the
    // repository recorded beside the task set. Deterministic and cheap, so it
    // runs before the critic and sends a false claim back while the author
    // still has attempts. A record that cannot be read is operational.
    let mut evaluation = None;
    if let Some(tasks_root) = path.parent() {
        let task_id = archon_workflow::task_universe::parsing::parse_task_file(&path, raw)
            .map(|task| task.canonical_task_id)
            .unwrap_or_else(|_| subject.clone());
        match super::repository_claims::inspect(cwd, tasks_root, &task_id, &path, raw) {
            Ok(claims) => {
                report.push_str("\n## repository claims\n");
                if claims.is_empty() {
                    report.push_str("  no backticked repository path is said to exist or not exist against the recorded repository\n");
                }
                for (index, text) in claims.into_iter().enumerate() {
                    report.push_str(&format!("  BLOCKING {text}\n"));
                    // Deterministic: checked against the recorded repository.
                    let identity = archon_workflow::defect::DeterministicDefect::new(
                        "repository_claim",
                        &task_id,
                        format!("claims/{index}"),
                    );
                    findings.push(
                        crate::command::workflow_gate::GateFinding::new(
                            crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                            text,
                            task_id.clone(),
                            Some(path.clone()),
                            archon_workflow::RemediationScope::Body,
                        )
                        .with_defect(identity),
                    );
                }
            }
            Err(error) => {
                evaluation = Some(
                    crate::command::workflow_gate::GateEvaluation::new(report.clone(), Vec::new())
                        .with_operational_error(format!(
                            "repository claim check failed: {error:#}"
                        )),
                );
            }
        }
    }
    Ok(match evaluation {
        Some(operational) => operational,
        None => crate::command::workflow_gate::GateEvaluation::new(report, findings),
    })
}

/// One finding per body line that holds the log-redaction marker as a
/// standalone word, the shape `events::redaction_marker_path` detects.
fn redaction_marker_lines(raw: &str) -> Vec<String> {
    let marker = archon_workflow::events::REDACTION_MARKER;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| {
            archon_workflow::events::redaction_marker_path(&serde_json::Value::String(
                (*line).to_string(),
            ))
            .is_some()
        })
        .map(|(index, _)| {
            format!(
                "line {}: holds the log-redaction marker `{marker}` as a standalone word; restore the original value (log redaction replaced it); if the body must name that text, quote it or build it (e.g. '<'+'redacted>')",
                index + 1
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "candidate_redaction_tests.rs"]
mod redaction_tests;

#[cfg(test)]
mod tests {
    const RAW: &str = "# Body\n\n```yaml\ntask_id: TASK-X-001\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: [sh]\ndeliverable_contracts:\n  - kind: first\n    artifact_path: out.json\n    typed_verifier_command: 'true'\n  - kind: second\n    artifact_path: out.json\n    typed_verifier_command: 'true'\n```\n\n## Focused Tests\n- `sh -c 'exit 1'`\n";

    // Guard: distinct structural slots can have byte-identical diagnostics.
    #[test]
    fn workflow_host_command_distinct_contract_defects_survive_identical_diagnostics() {
        let temp = tempfile::tempdir().expect("fixture");
        let root = temp.path().join("tasks");
        std::fs::create_dir(&root).expect("tasks");
        let path = root.join("TASK-X-001.md");
        let raw = RAW;
        let envelope = super::evaluate_task_file_candidate(
            temp.path(),
            &path,
            raw.as_bytes(),
            archon_core::config::GateMode::Enforce,
        )
        .expect("lint")
        .into_envelope()
        .expect("envelope");
        let identities: std::collections::BTreeSet<_> = envelope
            .policy_findings
            .iter()
            .filter_map(|finding| finding.deterministic_defect.clone())
            .filter(|identity| identity.code == "invalid_verifier")
            .collect();
        assert_eq!(identities.len(), 2, "{envelope:?}");
        // Guard: the same validator identities must reach the set gate too.
        std::fs::write(&path, RAW).expect("land task");
        let envelope = super::super::evaluate_lint(
            temp.path(),
            &super::super::LintSource::Tasks(root),
            archon_core::config::GateMode::Observe,
        )
        .expect("set lint")
        .into_envelope()
        .expect("set envelope");
        let identities: std::collections::BTreeSet<_> = envelope
            .policy_findings
            .iter()
            .filter_map(|finding| finding.deterministic_defect.clone())
            .filter(|identity| identity.code == "invalid_verifier")
            .collect();
        assert_eq!(identities.len(), 2, "{envelope:?}");
        // Guard: structured dependency identities also survive set lint.
        std::fs::write(
            &path,
            RAW.replace(
                "depends_on: []",
                "depends_on:\n  - task_id: TASK-X-002\n    ordering_only: false\n    consumes: []",
            ),
        )
        .expect("consumer");
        std::fs::write(
            path.parent().expect("tasks").join("TASK-X-002.md"),
            RAW.replace("TASK-X-001", "TASK-X-002"),
        )
        .expect("producer");
        let envelope = super::super::evaluate_lint(
            temp.path(),
            &super::super::LintSource::Tasks(path.parent().expect("tasks").to_path_buf()),
            archon_core::config::GateMode::Observe,
        )
        .expect("edge lint")
        .into_envelope()
        .expect("edge envelope");
        assert!(
            envelope.policy_findings.iter().any(|finding| finding
                .deterministic_defect
                .as_ref()
                .is_some_and(|identity| identity.code == "invalid_edge_declaration")),
            "{envelope:?}"
        );
    }
    /// Issue 261 round 7: every deterministic body-lint finding carries a
    /// host identity with its stage, so none is mistaken for a judge finding.
    #[test]
    fn workflow_host_command_every_body_lint_finding_carries_a_staged_identity() {
        let temp = tempfile::tempdir().expect("fixture");
        let root = temp.path().join("tasks");
        std::fs::create_dir(&root).expect("tasks");
        let path = root.join("TASK-X-001.md");
        let stages = |raw: &str| -> Vec<(String, String)> {
            let envelope = super::evaluate_task_file_candidate(
                temp.path(),
                &path,
                raw.as_bytes(),
                archon_core::config::GateMode::Enforce,
            )
            .expect("lint")
            .into_envelope()
            .expect("envelope");
            assert!(!envelope.policy_findings.is_empty(), "{envelope:?}");
            envelope
                .policy_findings
                .iter()
                .map(|finding| {
                    let defect = finding
                        .deterministic_defect
                        .as_ref()
                        .unwrap_or_else(|| panic!("no identity: {}", finding.text));
                    let stage = serde_json::to_value(defect).expect("json")["stage"].clone();
                    (
                        defect.code.clone(),
                        stage.as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        };
        assert_eq!(
            stages("broken task"),
            [("unparseable_task_file".to_string(), "parse".to_string())]
        );
        let shaped = RAW
            .replace("## Focused Tests\n- `sh -c 'exit 1'`\n", "")
            .replace("required_tools: [sh]", "required_tools: [sh, mcp__nope__x]");
        let found = stages(&shaped);
        for stage in ["shape", "tools", "contracts"] {
            assert!(found.iter().any(|(_, s)| s == stage), "{stage}: {found:?}");
        }
    }

    #[test]
    fn workflow_host_command_unparseable_task_does_not_hide_other_contract_defects() {
        let temp = tempfile::tempdir().expect("fixture");
        std::fs::write(temp.path().join("TASK-X-001.md"), RAW).expect("valid task");
        std::fs::write(temp.path().join("TASK-X-002.md"), "broken task").expect("invalid task");
        let findings = super::super::contracts::blocking_findings(Some(temp.path()));
        assert_eq!(findings.len(), 3, "{findings:?}");
    }
}
