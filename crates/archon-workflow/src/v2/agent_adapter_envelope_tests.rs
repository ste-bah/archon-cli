// Envelope-parse tolerance: providers wrap valid result envelopes in fences
// or prose. Parsing must locate the envelope (never invent content) and parse
// failures must carry an excerpt of what the agent actually wrote — a blind
// "expected value at line 1 column 1" starves the repair loop and the
// authoring retry of anything to correct (run-8 failure class).

use super::*;
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Status};

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
fn a_non_envelope_decoy_object_does_not_poison_the_reply() {
    let adapter = WorkflowV2AgentAdapter::new();
    // Echoed non-envelope fragment + real envelope: the fragment carries no
    // `status`, so it cannot be the reply and must not make the reply
    // unparseable either (previously any second object binned the whole
    // branch).
    let output = format!("{{\"note\": \"decoy\"}}\n{}", envelope_json());
    let result = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect("the sole envelope parses despite the decoy");
    assert_eq!(result.summary, "authored workflow script");
}

#[test]
fn draft_then_final_envelope_takes_the_final_one() {
    let adapter = WorkflowV2AgentAdapter::new();
    // Agent output is sequential: when two envelopes appear, the later one is
    // the agent's final word. Taking the draft here would surface a
    // needs_review result over the failed verdict the agent actually reached.
    let output = format!(
        "{}\n{{\"status\": \"failed\", \"summary\": \"final verdict: regression found\"}}",
        envelope_json()
    );
    let result = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect("the final envelope parses");
    assert_eq!(result.status, WorkflowV2Status::Failed);
    assert_eq!(result.summary, "final verdict: regression found");
}

#[test]
fn fenced_draft_then_fenced_final_takes_the_final_one() {
    let adapter = WorkflowV2AgentAdapter::new();
    // The live failure shape: a verify branch fenced its draft, said "now as
    // pure JSON", fenced the identical envelope again, and the branch was
    // failed with "expected value at line 1 column 1" after doing all the
    // work.
    let final_envelope = r#"{"status": "failed", "summary": "final: one check failed"}"#;
    let output = format!(
        "Draft first:\n```json\n{}\n```\nNow I need to return this as pure JSON:\n```json\n{final_envelope}\n```",
        envelope_json()
    );
    let result = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect("the final fenced envelope parses");
    assert_eq!(result.summary, "final: one check failed");
}

#[test]
fn truncation_after_a_complete_envelope_still_fails_loudly() {
    let adapter = WorkflowV2AgentAdapter::new();
    // A complete envelope followed by a truncated one: the truncated object is
    // the agent's actual last word, so promoting the earlier complete envelope
    // would report a verdict the agent superseded. Truncation always re-asks.
    let output = format!(
        "{}\n{{\"status\": \"failed\", \"summary\": \"the real final verdi",
        envelope_json()
    );
    adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect_err("truncated final envelope refuses extraction entirely");
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

/// The live 41k-char refusal: a complete, balanced, fenced envelope whose
/// only fault was an unescaped quote inside a shell command. The old guard
/// called it truncation and reported "expected value at line 1 column 1",
/// which named nothing the agent could fix, so the repair re-emitted the same
/// bad escape. The refusal is right; the error has to identify the fault.
#[test]
fn a_malformed_but_complete_envelope_reports_the_real_parse_fault() {
    let adapter = WorkflowV2AgentAdapter::new();
    // Balanced braces; invalid because the inner quotes are not escaped.
    let output = concat!(
        "Now I have all the information. Let me construct the output.\n\n",
        "```json\n",
        r#"{"status": "accepted", "summary": "did the work", "#,
        r#""commands_run": [{"command": "bash -lc 'grep -Fq "Checkout Identity" file'"}]}"#,
        "\n```"
    );
    let error = adapter
        .parse_agent_output(&read_only_request(), output)
        .expect_err("a malformed envelope is still refused");
    let message = error.to_string();
    assert!(
        !message.contains("expected value at line 1 column 1"),
        "must not report the prose preamble as the fault: {message}"
    );
    assert!(
        message.contains("line") && message.contains("column"),
        "must carry the position of the real fault: {message}"
    );
}

/// Truncation must still be refused with the root error — a reply that ran
/// out of tokens has no recoverable fault to point at, and extracting from it
/// is how a cut-off verdict gets recorded as a real one.
#[test]
fn a_truncated_envelope_is_still_refused_as_before() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = concat!(
        "Now I have the picture.\n\n```json\n",
        r#"{"status": "accepted", "summary": "did the work", "data": {"items": [{"id": "a"#
    );
    adapter
        .parse_agent_output(&read_only_request(), output)
        .expect_err("truncated replies stay refused");
}

/// The live rejection: a verification branch recorded two commands, the
/// second with an exit code but no `status`, and the whole envelope was
/// discarded with "missing field `status`" after the work was done.
#[test]
fn a_command_status_is_derived_from_its_exit_code() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = r#"{"status":"accepted","summary":"checked",
      "commands_run":[
        {"kind":"inspect","command":"ls a","exit_code":0,"output_summary":"ok"},
        {"kind":"test","command":"run b","exit_code":1,"output_summary":"failed"}
      ]}"#;

    let result = adapter
        .parse_agent_output(&read_only_request(), output)
        .expect("statusless commands are recoverable from their exit codes");

    let statuses: Vec<_> = result
        .commands_run
        .iter()
        .map(|command| command.status)
        .collect();
    assert_eq!(
        statuses,
        vec![
            crate::v2::result::WorkflowV2CommandStatus::Succeeded,
            crate::v2::result::WorkflowV2CommandStatus::Failed,
        ],
        "a non-zero exit must never be read as a pass"
    );
}

/// No status AND no exit code is nothing to infer from. Inventing a pass
/// there is exactly the false-success this enum refuses tolerant coercion to
/// avoid, so the reply is still rejected.
#[test]
fn a_command_with_neither_status_nor_exit_code_is_still_rejected() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = r#"{"status":"accepted","summary":"checked",
      "commands_run":[{"kind":"test","command":"run b","output_summary":"ran"}]}"#;

    adapter
        .parse_agent_output(&read_only_request(), output)
        .expect_err("nothing to derive from; must not invent a status");
}

/// An explicit status is the agent's own verdict and is never overwritten by
/// an exit code that disagrees with it.
#[test]
fn an_explicit_command_status_survives_a_conflicting_exit_code() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = r#"{"status":"accepted","summary":"checked",
      "commands_run":[{"kind":"test","command":"b","exit_code":0,"status":"failed","output_summary":"x"}]}"#;

    let result = adapter
        .parse_agent_output(&read_only_request(), output)
        .expect("explicit status parses");

    assert_eq!(
        result.commands_run[0].status,
        crate::v2::result::WorkflowV2CommandStatus::Failed
    );
}

/// The live loss: an agent wrote a bracketed list in its prose, then the real
/// envelope. The array arm aborted the parse before the envelope was reached.
#[test]
fn an_array_in_the_prose_does_not_kill_the_envelope_after_it() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = format!(
        "Now I have the data. Routes checked: [\"retry\", \"supersede\"]\n\n{}",
        envelope_json()
    );

    let result = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect("an array before the envelope must not abort the parse");

    assert_eq!(result.summary, "authored workflow script");
}

/// The rule the array arm exists for still holds: the array is consumed whole,
/// so an envelope inside it is never promoted to top level.
#[test]
fn an_envelope_inside_an_array_is_still_not_the_reply() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = format!("Here is the result list:\n[{}]", envelope_json());

    adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect_err("a nested envelope is not the reply envelope");
}

/// The live loss, second form. The array above is VALID JSON, so it exercises
/// only the branch that consumes a complete array. A bracket in prose that is
/// NOT valid JSON fails to parse instead, and that arm went on aborting the
/// whole reply: a verification branch wrote "7 `#[test]` annotations" in its
/// preamble, `[test]` failed as an array at line 1 column 3, and the complete
/// envelope 40 lines later was never reached.
#[test]
fn a_malformed_bracket_in_prose_does_not_kill_the_envelope_after_it() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = format!(
        "File contains 7 `#[test]` annotations matching the claim.\n\n{}",
        envelope_json()
    );

    let result = adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect("a prose bracket must not abort the parse");

    assert_eq!(result.summary, "authored workflow script");
}

/// Rust prose is full of brackets. None of them may cost a finished result.
#[test]
fn rust_flavoured_prose_brackets_are_all_survivable() {
    let adapter = WorkflowV2AgentAdapter::new();
    for prose in [
        "ran the `[cfg(test)]` module",
        "added a `[dependencies]` entry",
        "matched [unresolved import] in the log",
    ] {
        let output = format!("{prose}\n\n{}", envelope_json());
        adapter
            .parse_agent_output(&read_only_request(), &output)
            .unwrap_or_else(|error| panic!("prose {prose:?} aborted the parse: {error}"));
    }
}

/// The shape the malformed-container arm actually guards. An array that opens
/// with an object IS a JSON container, so when it fails to parse its contents
/// stay unreachable — a complete envelope inside a broken wrapper must never
/// be lifted out and returned as the reply.
#[test]
fn a_malformed_array_of_objects_still_refuses_to_promote_its_contents() {
    let adapter = WorkflowV2AgentAdapter::new();
    let output = format!("Results:\n[{}, not-json]", envelope_json());

    adapter
        .parse_agent_output(&read_only_request(), &output)
        .expect_err("a broken array wrapper must not yield its nested envelope");
}
