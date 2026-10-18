//! #428 follow-up: a bound body the host cannot stamp is refused back to the
//! author with a finding, never a failed call; the host's own faults still
//! fail it. The refusal is the outcome the fixed script's author loop reads.
use archon_workflow::{
    HostCommandRequest, HostCommandResult, RemediationScope, WorkflowError, WorkflowResult,
};

use crate::command::workflow_host_command_catalog::{
    HostCommandResolutionContext, fixed_decomposition_catalog,
};
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, WorkflowHostCommandExecutor,
};
use crate::command::workflow_host_command_exec_tests::{context, seed_frozen_chain};

const ID: &str = "TASK-X-010";

/// Frontmatter lines the task parser accepts for TASK-X-010.
const KEYS: &str = "task_id: TASK-X-010\ntitle: T\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n";

fn whole() -> String {
    format!("```yaml\n{KEYS}```\n\n# {ID}\n")
}

/// The live R8 shape: the answer opens with its ```yaml block, but the
/// fields were nested under `depends_on` and then repeated at the top, so
/// every top-level key appears twice. The host binds it (a JSON mapping keeps
/// the last value of each key), but no anchor is found (a YAML mapping
/// refuses a repeated key).
fn duplicated_keys() -> String {
    format!(
        "```yaml\ntask_id: {ID}\ntitle: T\ncomplexity: small\nstatus: ready\ndepends_on:\n  blocks: []\n  complexity: small\n  implements: []\n{KEYS}```\n\n# {ID}\n"
    )
}

/// The task file inside another fence: never a top-level frontmatter.
fn nested_in_a_fence() -> String {
    format!(
        "My answer:\n```markdown\nThe task file:\n\n{}```\n",
        whole()
    )
}

/// A chain freezing TASK-X-010, and its context: bound in advance when
/// `prebound`, so the request skips the binding and reaches the stamp alone.
fn chain(root: &std::path::Path, prebound: bool) -> HostCommandResolutionContext {
    let mut context = context(root);
    let task_file = context.task_root.join(format!("{ID}.md"));
    seed_frozen_chain(&context, &task_file);
    if prebound {
        context.frozen_task_id = Some(ID.into());
        context.frozen_task_file = Some(task_file);
    }
    context
}

async fn execute(answer: &str, prebound: bool) -> WorkflowResult<HostCommandResult> {
    let temp = tempfile::tempdir().unwrap();
    let context = chain(temp.path(), prebound);
    let executor = FixedHostCommandExecutor::new(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        temp.path().join("run"),
    );
    let request = HostCommandRequest::new("land-task-body", Some(answer.to_string())).unwrap();
    executor.execute(request, Some(1)).await
}

/// The one finding of a candidate refused before staging, as the body gate's
/// shape refusal: a `Body` finding on the task file, `invalid_candidate_shape`.
fn refused(outcome: &HostCommandResult) -> archon_workflow::GatePolicyFinding {
    assert!(outcome.publication_receipt.is_none(), "nothing lands");
    assert_eq!(
        outcome.postcondition.as_ref().map(|p| p.summary.as_str()),
        Some("candidate refused before staging")
    );
    let envelope = outcome.gate_envelope.as_ref().expect("a refusal");
    assert!(envelope.operational_error.is_none());
    assert_eq!(envelope.policy_findings.len(), 1, "{envelope:?}");
    let finding = envelope.policy_findings[0].clone();
    assert_eq!(finding.remediation_scope, RemediationScope::Body);
    let defect = finding
        .deterministic_defect
        .as_ref()
        .expect("host identity");
    assert_eq!(defect.code, "invalid_candidate_shape");
    assert!(finding.subject.ends_with("TASK-X-010.md"), "{finding:?}");
    finding
}

#[test]
fn the_live_shape_and_a_fenced_task_file_still_bind_so_the_stamp_decides() {
    for answer in [duplicated_keys(), nested_in_a_fence()] {
        let temp = tempfile::tempdir().unwrap();
        let context = chain(temp.path(), false);
        let request = HostCommandRequest::new("land-task-body", Some(answer)).unwrap();
        let bound = crate::command::workflow_host_command_binding::context_for_request(
            &context,
            &temp.path().join("run"),
            &request,
        )
        .expect("binds");
        assert_eq!(bound.frozen_task_id.as_deref(), Some(ID));
    }
}

#[tokio::test]
async fn a_frontmatter_with_repeated_keys_is_refused_with_the_yaml_error() {
    let outcome = execute(&duplicated_keys(), false)
        .await
        .expect("a refusal, not a failed call");
    let finding = refused(&outcome);
    let prefix = "candidate TASK body for TASK-X-010 has no task file frontmatter the host can anchor on: the ```yaml frontmatter block opened on line 1 is not valid YAML: ";
    assert!(finding.text.starts_with(prefix), "{}", finding.text);
    assert!(finding.text.contains("duplicate"), "{}", finding.text);
    assert!(
        finding.text.ends_with("; return only the task file, starting with one top-level ```yaml frontmatter block that is a valid YAML mapping (each key once) with task_id: TASK-X-010"),
        "{}",
        finding.text
    );
}

#[tokio::test]
async fn a_task_file_inside_another_fence_is_refused_naming_the_fence() {
    let outcome = execute(&nested_in_a_fence(), false)
        .await
        .expect("a refusal, not a failed call");
    assert_eq!(
        refused(&outcome).text,
        "candidate TASK body for TASK-X-010 has no task file frontmatter the host can anchor on: the ```yaml frontmatter block opened on line 5 is inside another fenced block; return only the task file, starting with one top-level ```yaml frontmatter block that is a valid YAML mapping (each key once) with task_id: TASK-X-010"
    );
}

#[tokio::test]
async fn two_task_files_are_refused_whether_binding_or_the_stamp_sees_them() {
    let two = format!("{}\n{}", whole(), whole());
    for prebound in [false, true] {
        let outcome = execute(&two, prebound)
            .await
            .expect("a refusal, not a failed call");
        assert_eq!(
            refused(&outcome).text,
            "the answer holds 2 task files (TASK-X-010, TASK-X-010); return only one task file, starting with its ```yaml frontmatter block; if TASK-X-010 is an example, put it inside a ```markdown fence",
            "prebound={prebound}"
        );
    }
}

/// Bound in advance the stamp refuses it; unbound the binding does, with the
/// same cause, not the generic "binds 0 frozen subjects".
#[tokio::test]
async fn a_frontmatter_the_stamp_cannot_read_is_refused_with_its_cause() {
    let unknown = format!("```yaml\n{KEYS}author_note: x\n```\n\n# {ID}\n");
    for prebound in [true, false] {
        let outcome = execute(&unknown, prebound)
            .await
            .expect("a refusal, not a failed call");
        assert_eq!(
            refused(&outcome).text,
            "candidate TASK body for TASK-X-010 does not parse: task frontmatter contains unknown field 'author_note'; return the whole task file",
            "prebound={prebound}"
        );
    }
}

#[test]
fn every_missing_anchor_cause_names_its_line_and_the_expected_form() {
    let form = "; return only the task file, starting with one top-level ```yaml frontmatter block that is a valid YAML mapping (each key once) with task_id: TASK-X-010";
    for (answer, cause) in [
        ("", "the answer has no ```yaml frontmatter block"),
        (
            "# just prose\r\n",
            "the answer has no ```yaml frontmatter block",
        ),
        (
            "intro\r\n```yaml\r\ntask_id: TASK-X-010\r\n",
            "the ```yaml frontmatter block opened on line 2 is not closed",
        ),
        (
            "```yaml\ntitle: T\n```\n",
            "the ```yaml frontmatter block opened on line 1 has no task_id",
        ),
        (
            "```yaml\nkey: value\n```\n```yaml\ntask_id: TASK-X-010\ntask_id: TASK-X-010\n```\n",
            "the ```yaml frontmatter block opened on line 4 is not valid YAML: ",
        ),
    ] {
        let finding = super::missing_anchor_finding(answer, ID);
        let prefix = format!(
            "candidate TASK body for TASK-X-010 has no task file frontmatter the host can anchor on: {cause}"
        );
        assert!(finding.starts_with(&prefix), "{answer:?}: {finding}");
        assert!(finding.ends_with(form), "{answer:?}: {finding}");
    }
}

#[test]
fn host_faults_still_fail_the_call() {
    let temp = tempfile::tempdir().unwrap();
    // No pin, skeleton or chain on disk: the pin read fails the call.
    let mut unpinned = context(temp.path());
    unpinned.frozen_task_id = Some(ID.into());
    unpinned.frozen_task_file = Some(unpinned.task_root.join(format!("{ID}.md")));
    let mut request = HostCommandRequest::new("land-task-body", Some(whole())).unwrap();
    let error = super::stamp_request(&mut request, &unpinned).unwrap_err();
    assert!(matches!(&error, WorkflowError::Io { .. }), "{error}");
    let pinned = chain(temp.path(), true);
    let skeleton = pinned
        .task_root
        .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
    let frozen = std::fs::read(&skeleton).unwrap();
    std::fs::write(&skeleton, b"{}").unwrap();
    let error = super::stamp_request(&mut request, &pinned).unwrap_err();
    assert!(matches!(&error, WorkflowError::SpecInvalid(_)), "{error}");
    std::fs::write(&skeleton, frozen).unwrap();
    let mut unbound = pinned.clone();
    unbound.frozen_task_id = None;
    let error = super::stamp_request(&mut request, &unbound).unwrap_err();
    assert!(
        matches!(&error, WorkflowError::SpecInvalid(text) if text.contains("no host-bound frozen task id")),
        "{error}"
    );
    let mut no_stdin = HostCommandRequest::new("land-task-body", None).unwrap();
    let error = super::stamp_request(&mut no_stdin, &pinned).unwrap_err();
    assert!(
        matches!(&error, WorkflowError::SpecInvalid(text) if text.contains("requires candidate stdin")),
        "{error}"
    );
    // A clean candidate is stamped, its stdin rewritten in place.
    assert_eq!(
        super::stamp_request(&mut request, &pinned)
            .unwrap()
            .unwrap(),
        Vec::<String>::new()
    );
    assert!(request.stdin.unwrap().starts_with("```yaml\n"));
}

/// The fixed script's body loop receives the host's refusal exactly as the
/// host returns it: the next author prompt carries the finding.
#[tokio::test]
async fn the_body_author_loop_gets_the_refusal_as_its_next_attempts_finding() {
    let outcome = execute(&duplicated_keys(), false).await.unwrap();
    let text = refused(&outcome).text;
    let refusal = serde_json::to_string(&outcome).unwrap();
    let driver = format!(
        r##"{script}
globalThis.__archonValidateAcceptanceEntry = () => "[]";
// Explicit Node fixture binding; host redaction parity is covered in QuickJS.
globalThis.__archonRedactPublicText = text => String(text);
const stubDigest = (text) => [...String(text)].reduce((h, c) => Math.imul(h ^ c.codePointAt(0), 16777619) >>> 0, 2166136261).toString(16).padStart(8, "0").repeat(8);
globalThis.__archonAuthorContext = (ext, text) => JSON.stringify({{ path: "/run/author-context/" + stubDigest(text) + "." + ext, sha256: stubDigest(text) }});
globalThis.args = {{ projectRoot: "/p", repositoryRoot: "/r", prdPath: "/p/prd.md", prdDigest: "d", taskRoot: "/p/tasks", gateMode: "enforce", acceptanceCriteria: {{ "AC-X-001": "c" }} }};
const prompts = [];
let lands = 0;
const w = {{
  agent: async (_id, options) => {{ prompts.push(options.task); return {{ status: "accepted", stopReason: "end_turn", content: "# body " + prompts.length }}; }},
  hostCommand: async () => {{
    lands += 1;
    if (lands === 1) return {refusal};
    return {{ publicationReceipt: {{ call_id: "land-2" }}, postcondition: {{ satisfied: true }}, gateEnvelope: {{ policy_findings: [] }} }};
  }},
  pause: async (id) => {{ throw new Error("unexpected pause " + id); }},
  finalReport: async () => ({{}}),
}};
authorCandidate(w, bodyPolicy({{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }}, [])).then(
  (outcome) => console.log(JSON.stringify({{ landed: outcome.publicationReceipt.call_id, lands, prompts }})),
  (error) => console.log(JSON.stringify({{ error: String(error && error.message), lands, prompts }})),
);
"##,
        script = crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE,
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stamp-refusal.cjs");
    std::fs::write(&path, driver).unwrap();
    let output = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node must be available");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(out["landed"], "land-2", "{out}");
    assert_eq!(out["lands"], 2, "{out}");
    let prompts = out["prompts"].as_array().unwrap();
    assert_eq!(prompts.len(), 2, "{out}");
    let second = prompts[1].as_str().unwrap();
    assert!(second.contains("Logical attempt: 2."), "{second}");
    assert!(
        second.contains(&text),
        "the finding reaches the author: {second}"
    );
}
