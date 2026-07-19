// Envelope-parse tolerance: providers wrap valid result envelopes in fences
// or prose. Parsing must locate the envelope (never invent content) and parse
// failures must carry an excerpt of what the agent actually wrote — a blind
// "expected value at line 1 column 1" starves the repair loop and the
// authoring retry of anything to correct (run-8 failure class).

use super::*;
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions};

fn read_only_request() -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "author-workflow-script".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        role: "planner".to_string(),
        task: "author the workflow script".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: Vec::new(),
        target_ownership_scopes: Vec::new(),
    }
}

fn envelope_json() -> String {
    serde_json::json!({
        "status": "needs_review",
        "summary": "authored workflow script",
        "data": {
            "workflow_js": "export const meta = { name: 'x' }\nphase('One')\nconst a = await agent('goal { not json } braces', { label: 'one' })\nreturn { accepted: [] }"
        }
    })
    .to_string()
}

fn parsed_workflow_js(result: &WorkflowV2Result) -> &str {
    result
        .data
        .get("workflow_js")
        .and_then(serde_json::Value::as_str)
        .expect("workflow_js survives parsing")
}

#[test]
fn bare_json_envelope_still_parses() {
    let adapter = WorkflowV2AgentAdapter::new();
    let result = adapter
        .parse_agent_output(&read_only_request(), &envelope_json())
        .expect("bare envelope parses");
    assert!(parsed_workflow_js(&result).contains("phase('One')"));
}

#[test]
fn fenced_envelope_is_located_and_parsed() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = format!("```json\n{}\n```", envelope_json());
    let result = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect("fenced envelope parses");
    assert!(parsed_workflow_js(&result).contains("not json"));
}

#[test]
fn prose_preamble_with_stray_brace_still_locates_the_envelope() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = format!(
        "Here is the result (note: schema uses {{ curly braces.\n\n{}\n\nDone.",
        envelope_json()
    );
    let result = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect("envelope behind prose and a stray brace parses");
    assert_eq!(result.summary, "authored workflow script");
}

#[test]
fn two_complete_objects_are_ambiguous_and_fail_loudly() {
    let adapter = WorkflowV2AgentAdapter::new();
    // Echoed schema example + real envelope: guessing between them is how a
    // failed reply gets recorded as an accepted no-op. Never guess.
    let output = format!("{{\"note\": \"decoy\"}}\n{}", envelope_json());
    let error = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect_err("ambiguous multi-object reply goes to repair");
    assert!(error.to_string().contains("output begins:"), "{error}");
}

#[test]
fn draft_then_final_envelope_is_ambiguous_and_fails_loudly() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = format!(
        "{}\n{{\"status\": \"failed\", \"summary\": \"final verdict: regression found\"}}",
        envelope_json()
    );
    adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect_err("draft plus final is ambiguous; repair must re-ask");
}

#[test]
fn prose_wrapped_single_element_array_cannot_impersonate_the_envelope() {
    let adapter = WorkflowV2AgentAdapter::new();
    // The object is complete and validates, but it is an array element rather
    // than the reply envelope. Scanning only object starts would incorrectly
    // promote it to the top level on a read-only call.
    let output = format!("Here is the result list:\n[{}]", envelope_json());
    let error = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect_err("an envelope nested in an array is not the reply envelope");
    assert!(error.to_string().contains("output begins:"), "{error}");
}

#[test]
fn malformed_outer_object_cannot_promote_a_complete_nested_envelope() {
    let adapter = WorkflowV2AgentAdapter::new();
    // The outer document is invalid at `not-json`. Its nested object is a
    // complete, validating envelope, but accepting it would turn malformed
    // wrapper output into a successful read-only result.
    let output = format!("{{\"wrapper\": not-json, \"result\": {}}}", envelope_json());
    let error = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect_err("a nested envelope cannot escape a malformed outer object");
    assert!(error.to_string().contains("output begins:"), "{error}");
}

#[test]
fn truncated_envelope_with_one_complete_nested_fragment_fails_loudly() {
    let adapter = WorkflowV2AgentAdapter::new();
    // The outer envelope is cut off mid-string; its only complete nested
    // object (an evidence item without `status`) must not impersonate the
    // reply as a default-Pending result.
    let output = r#"{"status": "failed", "summary": "real summary", "evidence": [{"kind": "inspection", "summary": "looked at src/lib.rs"}], "data": {"workflow_js": "export const meta = { name:"#;
    adapter
        .parse_agent_output(&read_only_request(), output)
        .expect_err("nested fragment without status is not an envelope");
}

#[test]
fn truncated_reduce_reply_with_complete_branch_envelope_in_data_fails_loudly() {
    let adapter = WorkflowV2AgentAdapter::new();
    // data.items legitimately carries envelope-shaped branch results; when the
    // outer reply is truncated, the sole complete branch envelope must not be
    // extracted as the reply (a failed reduce would surface as Accepted).
    let output = r#"{"status": "failed", "summary": "reduce failed: branches disagree", "evidence": [], "data": {"items": [{"status": "accepted", "summary": "branch b1 done", "evidence": [{"kind": "implementation", "summary": "branch proof"}]}, {"status": "accepted", "summary": "branch b2 done", "evidence": [{"kind": "impl"#;
    let error = adapter
        .parse_agent_output(&read_only_request(), output)
        .expect_err("truncated reply refuses extraction entirely");
    assert!(error.to_string().contains("output begins:"), "{error}");
}

#[test]
fn lone_task_coverage_fragment_with_status_cannot_impersonate_the_envelope() {
    let adapter = WorkflowV2AgentAdapter::new();
    // A coverage entry carries `status` and even validation-passing evidence;
    // its mandatory `task_id` (never a top-level envelope key) marks it as a
    // fragment. A failed reply must never surface as Accepted.
    let output = r#"{"status": "failed", "summary": "verification failed", "task_coverage": [{"task_id": "T1", "status": "accepted", "summary": "done", "evidence": [{"kind": "implementation", "summary": "proof"}]}], "data": {"workflow_js": "export const meta = { name:"#;
    let error = adapter
        .parse_agent_output(&read_only_request(), output)
        .expect_err("status-bearing coverage fragment is not an envelope");
    assert!(error.to_string().contains("output begins:"), "{error}");
}

#[test]
fn unparseable_output_error_carries_an_excerpt_of_what_was_written() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = "I could not produce the envelope because\nthe task list was ambiguous.";
    let error = adapter
        .parse_agent_output(&read_only_request(), output)
        .expect_err("prose without any JSON object fails");
    let message = error.to_string();
    assert!(message.contains("output begins:"), "{message}");
    assert!(
        message.contains("I could not produce the envelope because the task list"),
        "excerpt is the collapsed head of the reply: {message}"
    );
}

#[test]
fn malformed_envelope_error_carries_the_excerpt_too() {
    let adapter = WorkflowV2AgentAdapter::new();
    // Parses as JSON but violates the schema: evidence must be a sequence.
    let output = r#"{"status": "needs_review", "summary": "s", "evidence": {"kind": "review"}}"#;
    let error = adapter
        .parse_agent_output(&read_only_request(), output)
        .expect_err("map-shaped evidence fails the schema");
    let message = error.to_string();
    assert!(message.contains("output begins:"), "{message}");
    assert!(message.contains("\"evidence\""), "{message}");
}
