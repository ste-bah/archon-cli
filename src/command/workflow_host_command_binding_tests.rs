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

/// The subject is the one the frontmatter names: a `task_id:` line in chat
/// before it is not the candidate's.
#[tokio::test]
async fn a_task_id_line_in_chat_does_not_name_the_subject() {
    let finding = refusal(
        "I mean this file:\ntask_id: TASK-X-010\n\n```yaml\ntask_id: TASK-X-999\ntitle: T\n",
    )
    .await;
    assert_eq!(
        finding.text,
        "candidate TASK body binds 0 frozen subjects; return exactly one body preserving a frozen task_id and file_name"
    );
}

/// Proof for the gate: binding reads only the first frontmatter, so an
/// answer with two complete task files binds the first subject, and the
/// body gate is what must refuse it.
#[test]
fn binding_alone_accepts_an_answer_with_two_task_files() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    seed_frozen_chain(&context, &task_file);
    let frontmatter = |id: &str| {
        format!(
            "```yaml\ntask_id: {id}\ntitle: T\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# {id}\n"
        )
    };
    let answer = format!(
        "{}\n{}",
        frontmatter("TASK-X-010"),
        frontmatter("TASK-X-011")
    );
    let request = HostCommandRequest::new("land-task-body", Some(answer)).unwrap();
    let bound = super::workflow_host_command_binding::context_for_request(
        &context,
        &temp.path().join("run"),
        &request,
    )
    .expect("binds");
    assert_eq!(bound.frozen_task_id.as_deref(), Some("TASK-X-010"));
}

/// A yaml example in the chat before the task file is not the frontmatter:
/// the subject is the one the task file's own frontmatter names, found by
/// the same anchor the packaging strip uses.
#[tokio::test]
async fn a_yaml_example_in_chat_does_not_hide_the_subject() {
    let answer =
        "Shape I will use:\n```yaml\nkey: value\n```\n\n```yaml\ntask_id: TASK-X-010\ntitle: \"cut";
    let finding = refusal(answer).await;
    assert!(
        finding
            .text
            .starts_with("candidate TASK body for TASK-X-010 does not parse: the ```yaml frontmatter block is not closed"),
        "{}",
        finding.text
    );
}

#[test]
fn a_yaml_example_in_chat_before_a_whole_task_file_still_binds() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    seed_frozen_chain(&context, &task_file);
    let answer = "Shape I will use:\n```yaml\nkey: value\n```\n\n```yaml\ntask_id: TASK-X-010\ntitle: T\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# TASK-X-010\n";
    let request = HostCommandRequest::new("land-task-body", Some(answer.into())).unwrap();
    let bound = super::workflow_host_command_binding::context_for_request(
        &context,
        &temp.path().join("run"),
        &request,
    )
    .expect("binds");
    assert_eq!(bound.frozen_task_id.as_deref(), Some("TASK-X-010"));
}

/// An incomplete yaml example in chat that names another task does not stop
/// the valid task file after it binding to its own subject.
#[test]
fn an_incomplete_named_example_in_chat_does_not_stop_the_valid_file_binding() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    seed_frozen_chain(&context, &task_file);
    let answer = "Shape I will use:\n```yaml\ntask_id: TASK-X-999\nnote: shape only\n```\n\n```yaml\ntask_id: TASK-X-010\ntitle: T\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# TASK-X-010\n";
    let request = HostCommandRequest::new("land-task-body", Some(answer.into())).unwrap();
    let bound = super::workflow_host_command_binding::context_for_request(
        &context,
        &temp.path().join("run"),
        &request,
    )
    .expect("binds");
    assert_eq!(bound.frozen_task_id.as_deref(), Some("TASK-X-010"));
}

/// A complete example task file in chat, for another task, does not stop
/// the real task file after it binding to its own subject.
#[test]
fn a_complete_example_in_chat_does_not_stop_the_real_file_binding() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    seed_frozen_chain(&context, &task_file);
    let file = |id: &str| {
        format!(
            "```yaml\ntask_id: {id}\ntitle: T\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# {id}\n"
        )
    };
    let answer = format!(
        "An example:\n\n{}\n{}",
        file("TASK-X-999"),
        file("TASK-X-010")
    );
    let request = HostCommandRequest::new("land-task-body", Some(answer)).unwrap();
    let bound = super::workflow_host_command_binding::context_for_request(
        &context,
        &temp.path().join("run"),
        &request,
    )
    .expect("binds");
    assert_eq!(bound.frozen_task_id.as_deref(), Some("TASK-X-010"));
}
