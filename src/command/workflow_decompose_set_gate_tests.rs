//! Issue-46: the set gate sends a body-scope finding back to the body it
//! names instead of ending the run, and a frozen chain on disk is verified
//! rather than re-authored.
//!
//! The whole embedded script runs under `node` against a scripted host: the
//! driver decides what each `land-task-body`, `task-set-lint` and
//! `requirements-trace` call answers, and the test reads back which calls
//! were made, in what order, and what the final evidence held.

use super::FIXED_SCRIPT_SOURCE;

const TASK_ROOT: &str = "/p/tasks";

/// A scripted host: `lintRounds` is the list of finding arrays task-set-lint
/// answers with, one per call (the last one repeats); requirements-trace is
/// always clean. Every host outcome is committed with a receipt whose id is
/// the capability plus the call ordinal.
fn driver(args_json: &str, lint_rounds: &str, tail: &str) -> String {
    format!(
        r##"{FIXED_SCRIPT_SOURCE}
globalThis.args = Object.assign({{
  projectRoot: "/p", prdPath: "/p/prd.md", prdDigest: "d", taskRoot: "{TASK_ROOT}",
  gateMode: "enforce", acceptanceCriteria: {{ "AC-X-001": "criterion" }},
}}, {args_json});
const subjects = [
  {{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }},
  {{ taskId: "TASK-X-020", fileName: "TASK-X-020.md" }},
];
const lintRounds = {lint_rounds};
const agentCalls = [];
const hostCalls = [];
const prompts = {{}};
let lintCalls = 0;
let finalInputs = null;
const w = {{
  agent: async (id, options) => {{
    agentCalls.push(id);
    (prompts[id] = prompts[id] || []).push(options.task);
    const content = id.startsWith("acceptance-author-") ? JSON.stringify({{ id: "AC-X-001" }}) : "# body";
    return {{ status: "accepted", stopReason: "end_turn", content }};
  }},
  hostCommand: async (capability, options) => {{
    hostCalls.push(capability);
    let findings = [];
    if (capability === "task-set-lint") {{
      findings = lintRounds[Math.min(lintCalls, lintRounds.length - 1)];
      lintCalls += 1;
    }}
    const callId = capability + "-" + hostCalls.length;
    return {{
      publicationReceipt: {{ call_id: callId }},
      result: {{ data: {{ publicationReceipt: {{ call_id: callId }} }} }},
      postcondition: {{ satisfied: true }},
      gateEnvelope: {{ policy_findings: findings }},
      subjects: capability.endsWith("skeleton") ? subjects : [],
    }};
  }},
  finalReport: async (_id, options) => {{ finalInputs = options.inputs; return {{}}; }},
}};
workflow(w).then(
  () => console.log(JSON.stringify({{ agentCalls, hostCalls, prompts, evidence: finalInputs.length, {tail} }})),
  (error) => console.log(JSON.stringify({{ agentCalls, hostCalls, error: String(error && error.message) }})),
);
"##
    )
}

fn run(script: &str) -> serde_json::Value {
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("set-gate.cjs");
    std::fs::write(&path, script).expect("write driver");
    let out = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node must be available");
    assert!(
        out.status.success(),
        "driver failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("driver json")
}

fn body_finding(task: &str, source_path: &str) -> String {
    format!(
        r#"{{ "text": "obligation AC-X-001 is claimed by TASK-X-010 but none is obliged to make it true — the other task waives it — task {task}: \"waived\"", "subject": "AC-X-001", "source_path": {source_path}, "remediation_scope": "body" }}"#
    )
}

fn count(value: &serde_json::Value, key: &str, item: &str) -> usize {
    value[key]
        .as_array()
        .expect(key)
        .iter()
        .filter(|entry| entry == &item)
        .count()
}

#[test]
fn a_set_gate_body_finding_re_authors_the_task_it_names_with_the_finding_then_re_runs_both_gates() {
    let finding = body_finding("TASK-X-020", &format!("\"{TASK_ROOT}/TASK-X-020.md\""));
    let out = run(&driver("{}", &format!("[[{finding}], []]"), ""));
    assert!(out.get("error").is_none(), "{out}");
    let agent_calls = out["agentCalls"].as_array().unwrap();
    let bodies: Vec<&str> = agent_calls
        .iter()
        .filter_map(|id| id.as_str())
        .filter(|id| id.starts_with("body-"))
        .collect();
    assert_eq!(
        bodies,
        [
            "body-TASK-X-010-author-1",
            "body-TASK-X-020-author-1",
            "body-TASK-X-020-author-2"
        ],
        "only the named body is re-authored, in the same call family with the next ordinal: {out}"
    );
    assert_eq!(count(&out, "hostCalls", "land-task-body"), 3);
    assert_eq!(count(&out, "hostCalls", "task-set-lint"), 2);
    assert_eq!(count(&out, "hostCalls", "requirements-trace"), 2);
    let host_calls = out["hostCalls"].as_array().unwrap();
    let last_body = host_calls
        .iter()
        .rposition(|c| c == "land-task-body")
        .unwrap();
    let first_lint = host_calls
        .iter()
        .position(|c| c == "task-set-lint")
        .unwrap();
    assert!(
        first_lint < last_body,
        "the re-authoring follows the first set gate: {out}"
    );
    let repair_prompt = out["prompts"]["body-TASK-X-020-author-2"][0]
        .as_str()
        .expect("repair prompt");
    assert!(
        repair_prompt.contains("Repair these exact authoritative findings:")
            && repair_prompt.contains("the other task waives it"),
        "the set-gate finding opens the re-authoring prompt: {repair_prompt}"
    );
    assert!(
        repair_prompt.contains("the set gate, before this body was sent back"),
        "the seeded finding is shown as history too: {repair_prompt}"
    );
    assert_eq!(
        out["evidence"], 6,
        "acceptance + skeleton + one entry per subject + two gates; the re-authored body replaces its entry: {out}"
    );
}

#[test]
fn exhausting_the_set_gate_rounds_stops_with_the_open_findings_listed() {
    let finding = body_finding("TASK-X-020", &format!("\"{TASK_ROOT}/TASK-X-020.md\""));
    let out = run(&driver("{}", &format!("[[{finding}]]"), ""));
    let error = out["error"].as_str().expect("the run must stop");
    assert!(
        error.contains("exhausted 4 repair rounds") && error.contains("the other task waives it"),
        "{error}"
    );
    assert_eq!(count(&out, "hostCalls", "task-set-lint"), 4);
    assert_eq!(count(&out, "hostCalls", "requirements-trace"), 4);
    assert_eq!(
        count(&out, "hostCalls", "land-task-body"),
        2 + 3,
        "one re-authoring per round except the last, whose result no gate would judge: {out}"
    );
}

#[test]
fn observe_mode_falls_back_to_the_last_committed_gate_outcomes_when_rounds_are_spent() {
    let finding = body_finding("TASK-X-020", &format!("\"{TASK_ROOT}/TASK-X-020.md\""));
    let out = run(&driver(
        r#"{ gateMode: "observe" }"#,
        &format!("[[{finding}]]"),
        "",
    ));
    assert!(out.get("error").is_none(), "observe never blocks: {out}");
    assert_eq!(count(&out, "hostCalls", "task-set-lint"), 4);
    assert_eq!(out["evidence"], 6, "{out}");
}

#[test]
fn a_finding_without_a_resolvable_path_falls_back_to_the_task_it_quotes_then_its_subject() {
    // The PRD path is what the gate reports when the weakest task has no text;
    // the quoted `task TASK-…:` still names the body.
    let quoted = body_finding("TASK-X-020", "\"/p/prd.md\"");
    let out = run(&driver("{}", &format!("[[{quoted}], []]"), ""));
    assert!(out.get("error").is_none(), "{out}");
    assert!(
        out["agentCalls"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("body-TASK-X-020-author-2")),
        "{out}"
    );
    let by_subject = r#"{ "text": "no path and no quote", "subject": "TASK-X-010", "remediation_scope": "body" }"#;
    let out = run(&driver("{}", &format!("[[{by_subject}], []]"), ""));
    assert!(out.get("error").is_none(), "{out}");
    assert!(
        out["agentCalls"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("body-TASK-X-010-author-2")),
        "{out}"
    );
}

#[test]
fn a_finding_naming_no_frozen_task_stops_the_run_instead_of_being_dropped() {
    let stray = r#"{ "text": "obligation AC-X-001 is hollow — task TASK-Z-999: \"x\"", "subject": "AC-X-001", "source_path": "/elsewhere/TASK-Z-999.md", "remediation_scope": "body" }"#;
    let out = run(&driver("{}", &format!("[[{stray}], []]"), ""));
    let error = out["error"].as_str().expect("the run must stop");
    assert!(
        error.contains("names no frozen task") && error.contains("TASK-Z-999"),
        "{error}"
    );
    assert_eq!(
        count(&out, "hostCalls", "land-task-body"),
        2,
        "nothing is re-authored: {out}"
    );
}

const FROZEN_SUBJECTS: &str = r#"[{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }, { taskId: "TASK-X-020", fileName: "TASK-X-020.md" }]"#;

#[test]
fn a_fully_frozen_chain_is_verified_not_authored_before_the_set_gate() {
    let args = format!(
        r#"{{ frozenChain: {{ acceptance: true, skeleton: true, subjects: {FROZEN_SUBJECTS}, bodies: ["TASK-X-010.md", "TASK-X-020.md"] }} }}"#
    );
    let out = run(&driver(&args, "[[]]", ""));
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!([]),
        "no author call at all: {out}"
    );
    assert_eq!(
        out["hostCalls"],
        serde_json::json!([
            "verify-frozen-acceptance",
            "verify-frozen-skeleton",
            "task-set-lint",
            "requirements-trace"
        ]),
        "{out}"
    );
    assert_eq!(
        out["evidence"], 4,
        "acceptance + skeleton + the two gates; frozen bodies carry no author outcome: {out}"
    );
}

#[test]
fn a_frozen_chain_with_one_body_missing_authors_only_that_body() {
    let args = format!(
        r#"{{ frozenChain: {{ acceptance: true, skeleton: true, subjects: {FROZEN_SUBJECTS}, bodies: ["TASK-X-010.md"] }} }}"#
    );
    let out = run(&driver(&args, "[[]]", ""));
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!(["body-TASK-X-020-author-1"]),
        "{out}"
    );
    assert_eq!(
        out["hostCalls"],
        serde_json::json!([
            "verify-frozen-acceptance",
            "verify-frozen-skeleton",
            "land-task-body",
            "task-set-lint",
            "requirements-trace"
        ]),
        "{out}"
    );
    assert_eq!(out["evidence"], 5, "{out}");
}

#[test]
fn a_frozen_body_the_set_gate_sends_back_is_re_authored_like_any_other() {
    let args = format!(
        r#"{{ frozenChain: {{ acceptance: true, skeleton: true, subjects: {FROZEN_SUBJECTS}, bodies: ["TASK-X-010.md", "TASK-X-020.md"] }} }}"#
    );
    let finding = body_finding("TASK-X-010", &format!("\"{TASK_ROOT}/TASK-X-010.md\""));
    let out = run(&driver(&args, &format!("[[{finding}], []]"), ""));
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!(["body-TASK-X-010-author-1"]),
        "{out}"
    );
    assert_eq!(count(&out, "hostCalls", "task-set-lint"), 2);
    assert_eq!(
        out["evidence"], 5,
        "the re-authored frozen body now carries an author outcome: {out}"
    );
}

#[test]
fn a_frozen_acceptance_contract_alone_skips_only_the_acceptance_author() {
    let out = run(&driver(
        r#"{ frozenChain: { acceptance: true, skeleton: false, subjects: [], bodies: [] } }"#,
        "[[]]",
        "",
    ));
    assert!(out.get("error").is_none(), "{out}");
    let agent_calls = out["agentCalls"].as_array().unwrap();
    assert!(
        agent_calls
            .iter()
            .all(|id| !id.as_str().unwrap().starts_with("acceptance-author-")),
        "{out}"
    );
    assert_eq!(out["hostCalls"][0], "verify-frozen-acceptance", "{out}");
    assert_eq!(out["hostCalls"][1], "freeze-skeleton", "{out}");
    assert_eq!(out["evidence"], 6, "{out}");
}

#[test]
fn a_frozen_chain_whose_subjects_differ_at_verification_stops_the_run() {
    let args = r#"{ frozenChain: { acceptance: true, skeleton: true, subjects: [{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }], bodies: ["TASK-X-010.md"] } }"#;
    let out = run(&driver(args, "[[]]", ""));
    let error = out["error"].as_str().expect("the run must stop");
    assert!(error.contains("changed underneath the run"), "{error}");
    assert_eq!(out["agentCalls"], serde_json::json!([]), "{out}");
}
