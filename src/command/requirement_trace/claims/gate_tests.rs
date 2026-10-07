//! PLAN-2 at the gate: the staged requirement trace tests every claim at
//! decomposition, the host catalog admits the refutation it raises, and the
//! fixed decomposition script sends that refutation back to the body named.

use super::tests::{repository, task, task_set};
use crate::command::requirement_trace::TraceOptions;

const GOOD: &str = "## Files Expected to Change\n\n- `src/lib.rs` — exists (2 lines)\n\n## Scope\n\n- REQ-X-001 lives in `present_symbol`.\n\n## Focused Tests\n\n- `cargo test -p demo --test existing`\n";
const FALSE: &str = "## Files Expected to Change\n\n- `src/lib.rs` — exists (2 lines)\n\n## Scope\n\n- REQ-X-002 is proven by `tests/never_written.rs`.\n";

/// Run the staged gate exactly as the host command does, and read back the
/// envelope it staged.
fn staged_envelope(dir: &std::path::Path) -> archon_workflow::GateEnvelopeV1 {
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let base = repository(&root);
    let good = task("TASK-X-010", "REQ-X-001", GOOD);
    let bad = task("TASK-X-020", "REQ-X-002", FALSE);
    let tasks = task_set(
        dir,
        &root,
        &base,
        &[("TASK-X-010", &good), ("TASK-X-020", &bad)],
    );
    let prd = dir.join("prd.md");
    std::fs::write(&prd, "# PRD\n\n- REQ-X-001: one.\n- REQ-X-002: two.\n").unwrap();
    let envelope = dir.join("staging/gate-envelope.json");
    std::fs::create_dir_all(envelope.parent().unwrap()).unwrap();
    crate::command::requirement_trace::staged::handle(
        dir,
        &TraceOptions::new(prd, tasks),
        Some(&envelope),
        Some("trace-call-1"),
        archon_core::config::GateMode::Enforce,
    )
    .expect("staged trace");
    serde_json::from_slice(&std::fs::read(&envelope).unwrap()).unwrap()
}

#[test]
fn the_staged_gate_falsifies_every_claim_and_raises_the_refutation_on_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let written = staged_envelope(dir.path());
    assert!(written.operational_error.is_none(), "{written:?}");
    let report = written.report.to_string();
    assert!(
        report.contains("2 claim(s): 2 tested, 1 refuted, 0 untestable"),
        "the report carries the falsification plan of every claim: {report}"
    );
    assert!(report.contains("tested TASK-X-010 REQ-X-001"), "{report}");
    assert_eq!(written.policy_findings.len(), 1, "{written:?}");
    let finding = &written.policy_findings[0];
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert_eq!(finding.subject, "TASK-X-020");
    assert!(
        finding
            .source_path
            .as_deref()
            .is_some_and(|path| path.ends_with("TASK-X-020.md")),
        "{finding:?}"
    );
    assert!(
        finding.text.contains("`tests/never_written.rs`"),
        "{finding:?}"
    );

    // The host refuses an envelope whose findings carry a scope outside the
    // capability's catalog entry (workflow_host_command_exec), so the
    // refutation reaches the script only if the catalog admits `Body`.
    let catalog =
        crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev").unwrap();
    let trace = &catalog.capabilities["requirements-trace"];
    for finding in &written.policy_findings {
        assert!(
            trace
                .remediation_scopes
                .contains(&finding.remediation_scope),
            "requirements-trace catalog scopes {:?} refuse {:?}",
            trace.remediation_scopes,
            finding.remediation_scope
        );
    }
}

#[test]
fn the_decomposition_script_re_authors_the_body_whose_claim_the_trace_refuted() {
    let dir = tempfile::tempdir().unwrap();
    let written = staged_envelope(dir.path());
    let findings = serde_json::to_string(&written.policy_findings).unwrap();
    let out = run_script(&findings);
    assert!(out.get("error").is_none(), "{out}");
    let bodies: Vec<&str> = out["agentCalls"]
        .as_array()
        .unwrap()
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
        "only the refuted claim's body is re-authored: {out}"
    );
    let reauthor = out["prompts"]["body-TASK-X-020-author-2"][0]
        .as_str()
        .unwrap();
    assert!(
        reauthor.contains("tests/never_written.rs"),
        "the refutation is the author's feedback: {reauthor}"
    );
    let traces = out["hostCalls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| *c == "requirements-trace")
        .count();
    assert_eq!(
        traces, 2,
        "the trace runs again on the re-authored set: {out}"
    );
}

/// The fixed decomposition script under `node`, with a scripted host whose
/// first requirements-trace answer is `findings` and whose later answers
/// are clean.
fn run_script(findings: &str) -> serde_json::Value {
    let source = crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE;
    let script = format!(
        r##"{source}
globalThis.args = {{
  projectRoot: "/p", repositoryRoot: "/r", prdPath: "/p/prd.md", prdDigest: "d", taskRoot: "/p/tasks",
  gateMode: "enforce", acceptanceCriteria: {{ "AC-X-001": "criterion" }},
}};
// Node has no native entry validator; stub it as the other node-driven script tests do.
globalThis.__archonValidateAcceptanceEntry = () => "[]";
const subjects = [
  {{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }},
  {{ taskId: "TASK-X-020", fileName: "TASK-X-020.md" }},
];
const traceRounds = [{findings}, []];
const agentCalls = [];
const hostCalls = [];
const prompts = {{}};
let traceCalls = 0;
const w = {{
  agent: async (id, options) => {{
    agentCalls.push(id);
    (prompts[id] = prompts[id] || []).push(options.task);
    const content = id.startsWith("acceptance-author-") ? JSON.stringify({{ id: "AC-X-001" }}) : "# body";
    return {{ status: "accepted", stopReason: "end_turn", content }};
  }},
  hostCommand: async (capability) => {{
    hostCalls.push(capability);
    let findings = [];
    if (capability === "requirements-trace") {{
      findings = traceRounds[Math.min(traceCalls, traceRounds.length - 1)];
      traceCalls += 1;
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
  finalReport: async () => ({{}}),
}};
workflow(w).then(
  () => console.log(JSON.stringify({{ agentCalls, hostCalls, prompts }})),
  (error) => console.log(JSON.stringify({{ agentCalls, hostCalls, error: String(error && error.message) }})),
);
"##
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("set-gate.cjs");
    std::fs::write(&path, script).unwrap();
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
