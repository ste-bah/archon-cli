// A parse error that names only a line and column, next to the first 200
// characters of a 10k reply, gives the repair loop nothing it can act on: the
// fault is at column 7757, and the re-ask quotes column 1. Every shape here
// was recorded from one live run that exhausted its repair budget on
// mistakes a reader could fix in seconds -- once shown the bytes at fault.

use super::*;
use crate::{WorkflowV2HostMethod, WorkflowV2HostOptions};

fn read_only_request() -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "coverage-audit-map-0".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        role: "reviewer".to_string(),
        task: "audit coverage".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: Vec::new(),
        target_ownership_scopes: Vec::new(),
    }
}

fn parse_error(output: &str) -> String {
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(&read_only_request(), output)
        .expect_err("the reply is not a valid envelope")
        .to_string()
}

/// A Python-style literal deep inside an otherwise valid envelope. The error
/// must show the bytes at the fault with a marker, not the reply's head.
#[test]
fn syntax_fault_is_quoted_at_its_position() {
    let output = r#"{ "status": "accepted", "summary": "verified src/beta.json", "evidence": [],
  "data": { "deliverables": { "src/beta.json": { "exists": true, "parsed": {"ready": True}, "ready_type": "bool" } } } }"#;

    let error = parse_error(output);

    assert!(
        error.contains(r#""parsed": {"ready": <HERE>True"#),
        "the fault site must be quoted with the marker on the offending byte: {error}"
    );
    assert!(error.contains("expected value at line 2"), "{error}");
}

/// An array opened where an object member key was expected, thousands of
/// characters into a single-line envelope. The head excerpt cannot reach it.
#[test]
fn fault_far_past_the_head_excerpt_is_still_quoted() {
    let padding = "x".repeat(3_000);
    let output = format!(
        r#"{{"status":"accepted","summary":"{padding}","data":{{"checked":[1],["recommendation":"drop it"]}}}}"#
    );

    let error = parse_error(&output);

    assert!(
        error.contains(r#""checked":[1],<HERE>["recommendation""#),
        "the marker must land on the misplaced bracket: {error}"
    );
    assert!(
        !error.contains(&padding),
        "the window is bounded; it must not replay the whole reply: {}",
        error.len()
    );
}

/// The reply IS the file the task asked for, instead of an envelope carrying
/// it. Quoting a JavaScript token as a JSON syntax error teaches nothing; the
/// error must say what the reply is and where the contents belong.
#[test]
fn source_code_instead_of_an_envelope_is_named_as_such() {
    let output = "\n\n```\nexport const meta = {\n  name: 'alpha-beta-chain',\n  phases: [],\n};\n```\n";

    let error = parse_error(output);

    assert!(
        error.contains("source code, not the result envelope"),
        "the reply shape must be named: {error}"
    );
    assert!(
        error.contains("JSON string value"),
        "the error must say where file contents belong: {error}"
    );
}

/// Plain prose is not source code; the hint must not misfire on it.
#[test]
fn prose_is_not_mistaken_for_source_code() {
    let error = parse_error("markdown only, no envelope here.");
    assert!(
        !error.contains("source code"),
        "prose must keep the plain parse error: {error}"
    );
}

/// A schema violation names the exact location. `missing field \`path\``
/// alone sent the model hunting through every list; the path to the offending
/// element ends the hunt.
#[test]
fn missing_field_names_the_element_that_lacks_it() {
    let output = r#"{"status":"accepted","summary":"audited",
      "commands_run":[{"kind":"test","command":"cargo test","status":"succeeded","output_summary":"ok"},{"kind":"inspection","output_summary":"no command here"}]}"#;

    let error = parse_error(output);

    assert!(
        error.contains("commands_run[1]"),
        "the element lacking the field must be named: {error}"
    );
    assert!(error.contains("missing field `command`"), "{error}");
}

/// A multi-line script pasted into a JSON string as-is. serde stops at the
/// first raw line break and reports it at column 0 of the next line; the
/// error must mark that break and say what the fix is.
#[test]
fn raw_line_breaks_inside_a_string_are_named_and_marked() {
    let output = "{\"status\":\"accepted\",\"summary\":\"authored\",\"data\":{\"workflow_js\":\"export const meta = {\n  name: 'x',\n}\"}}";

    let error = parse_error(output);

    assert!(
        error.contains("raw line break"),
        "the unescaped break must be named as the fault: {error}"
    );
    assert!(
        error.contains(r#"export const meta = {\n<HERE>  name: 'x',"#),
        "the marker must land on the line break: {error}"
    );
}

/// A reply that ends with a value still open: cut short, or never closed. The
/// error must say the document is unterminated and show the tail, since the
/// fault has no interior position.
#[test]
fn unterminated_reply_is_named_and_its_tail_quoted() {
    let output = r#"{"status":"accepted","summary":"authored","data":{"workflow_js":"export const meta = { name: 'x' }"#;

    let error = parse_error(output);

    assert!(
        error.contains("still open"),
        "an unterminated reply must be named as such: {error}"
    );
    assert!(
        error.contains("name: 'x' }<HERE>"),
        "the tail of the reply must be quoted so the open value can be found: {error}"
    );
}

/// The repair prompt is what the model actually sees. The fault window must
/// reach it verbatim, not only the persisted failure record.
#[test]
fn repair_prompt_carries_the_fault_window() {
    let output = r#"{"status":"accepted","summary":"s","data":{"ok":True}}"#;
    let adapter = WorkflowV2AgentAdapter::new();
    let request = read_only_request();
    let error = adapter
        .parse_agent_output(&request, output)
        .expect_err("invalid literal");

    let prompt = adapter.build_repair_prompt(&request, output, &error);

    assert!(
        prompt.contains(r#""ok":<HERE>True"#),
        "the re-ask must quote the fault site: {prompt}"
    );
}
