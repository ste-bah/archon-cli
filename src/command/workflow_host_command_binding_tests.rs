//! A body that binds no frozen subject but names one by its `task_id` line is
//! refused with that subject's parse cause, through the shape refusal, so the
//! author gets a repair it can act on. A body that names no subject keeps the
//! binding refusal.
use archon_workflow::{HostCommandRequest, RemediationScope};

use super::workflow_host_command_catalog::fixed_decomposition_catalog;
use super::workflow_host_command_exec::{FixedHostCommandExecutor, WorkflowHostCommandExecutor};
use super::workflow_host_command_exec_tests::{context, seed_frozen_chain};

async fn refusal(answer: &str) -> archon_workflow::GatePolicyFinding {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    seed_frozen_chain(&context, &task_file);
    let executor = FixedHostCommandExecutor::new(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        temp.path().join("run"),
    );
    let request = HostCommandRequest::new("land-task-body", Some(answer.to_string())).unwrap();
    let outcome = executor.execute(request, Some(1)).await.unwrap();
    assert!(outcome.publication_receipt.is_none(), "nothing lands");
    assert_eq!(
        outcome.postcondition.as_ref().map(|p| p.summary.as_str()),
        Some("candidate refused before staging")
    );
    let mut findings = outcome.gate_envelope.expect("a refusal").policy_findings;
    assert_eq!(findings.len(), 1, "{findings:?}");
    findings.pop().unwrap()
}

/// The live shape: the author stopped part way through the frontmatter, so
/// the ```yaml block is never closed and the answer ends mid-value.
#[tokio::test]
async fn a_truncated_frontmatter_is_refused_with_its_parse_cause() {
    let answer = "```yaml\ntask_id: TASK-X-010\ntitle: \"Generic engine, freshness layer and tests\"\ncomplexity: medium\nstatus: ready\ndepends_on: [{task_id: \"TASK-X-001\", consumes: [{artifact_path: \"data/registry.j...";
    let finding = refusal(answer).await;
    let ends = answer.chars().count();
    let tail: String = answer.chars().skip(ends - 40).collect();
    assert_eq!(
        finding.text,
        format!(
            "candidate TASK body for TASK-X-010 does not parse: the ```yaml frontmatter block is not closed; the answer ends at char {ends} with '{tail}'; return the whole task file"
        )
    );
    assert_eq!(finding.remediation_scope, RemediationScope::Body);
    let defect = finding.deterministic_defect.expect("host identity");
    assert_eq!(defect.code, "invalid_candidate_shape");
    assert!(
        finding.subject.ends_with("TASK-X-010.md"),
        "{}",
        finding.subject
    );
}

/// A closed frontmatter that still fails to parse carries the parser's own
/// error for the subject it names.
#[tokio::test]
async fn a_frontmatter_missing_keys_is_refused_with_the_parser_error() {
    let finding = refusal("```yaml\ntask_id: TASK-X-010\ntitle: T\n```\n\n# Body\n").await;
    assert!(
        finding
            .text
            .starts_with("candidate TASK body for TASK-X-010 does not parse: "),
        "{}",
        finding.text
    );
    assert!(
        finding.text.contains("missing required key(s)"),
        "{}",
        finding.text
    );
    assert!(finding.text.ends_with("; return the whole task file"));
}

#[tokio::test]
async fn a_body_naming_no_frozen_subject_keeps_the_binding_refusal() {
    for answer in [
        "```yaml\ntitle: no task id here\n",
        "```yaml\ntask_id: TASK-X-999\n",
    ] {
        let finding = refusal(answer).await;
        assert_eq!(
            finding.text,
            "candidate TASK body binds 0 frozen subjects; return exactly one body preserving a frozen task_id and file_name"
        );
        assert_eq!(
            finding.remediation_scope,
            RemediationScope::CandidateArtifact
        );
        assert_eq!(
            finding.deterministic_defect.unwrap().code,
            "unbound_candidate"
        );
    }
}
