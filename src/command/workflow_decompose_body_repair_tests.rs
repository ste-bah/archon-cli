//! Issue-44: a `body` finding from the body gate — an obligation fidelity
//! verdict among them — drives another author attempt, and the repair prompt
//! carries the finding's exact text. Nothing in the script had to change for
//! this: the body phase already retries `body` scope and feeds `feedback`
//! into `authorPrompt`; this pins that the per-body audit rides that path.

use super::run_js;

const FIDELITY_FINDING: &str = "obligation AC-WS-001 is claimed by TASK-WS-002 but none is obliged to make it true — a fixture registry leaves the shared store empty — task TASK-WS-002: \\\"the shared store is not written\\\"";

fn driver(gate_mode: &str) -> String {
    format!(
        r##"
globalThis.args = {{ gateMode: "{gate_mode}" }};
let authorCalls = 0;
const prompts = [];
const committed = (findings) => ({{
  publicationReceipt: {{ id: "r" }},
  postcondition: {{ satisfied: true }},
  gateEnvelope: {{ policy_findings: findings }},
}});
const w = {{
  agent: async (_id, options) => {{
    authorCalls += 1;
    prompts.push(options.task);
    return {{ status: "accepted", stopReason: "end_turn", content: "# TASK-WS-002" }};
  }},
  hostCommand: async (capability) => {{
    if (capability !== "land-task-body") throw new Error("wrong capability " + capability);
    return authorCalls === 1
      ? committed([{{ text: "{FIDELITY_FINDING}", remediation_scope: "body", subject: "TASK-WS-002" }}])
      : committed([]);
  }},
}};
const policy = {{
  phase: "body-TASK-WS-002",
  capability: "land-task-body",
  attempts: 10,
  retryScopes: new Set(["candidate_artifact", "body"]),
  prompt: () => "author the body",
}};
authorCandidate(w, policy).then(
  () => console.log(JSON.stringify({{ authorCalls, repairPrompt: prompts[1] || null }})),
  (error) => console.log(JSON.stringify({{ authorCalls, error: String(error && error.message) }})),
);
"##
    )
}

#[test]
fn a_body_scope_fidelity_finding_drives_another_author_attempt_with_the_finding_text() {
    for gate_mode in ["observe", "enforce"] {
        let out = run_js(&driver(gate_mode));
        let value: serde_json::Value = serde_json::from_str(&out).expect("driver json");
        assert_eq!(
            value["authorCalls"], 2,
            "{gate_mode}: the finding must be repaired, not published or fatal: {out}"
        );
        let prompt = value["repairPrompt"]
            .as_str()
            .expect("second author prompt recorded");
        assert!(
            prompt.contains("Repair these exact authoritative findings:"),
            "{gate_mode}: {prompt}"
        );
        assert!(
            prompt.contains("obligation AC-WS-001 is claimed by TASK-WS-002")
                && prompt.contains("a fixture registry leaves the shared store empty")
                && prompt.contains("the shared store is not written"),
            "{gate_mode}: the author must read the obligation, the reason and the quote: {prompt}"
        );
    }
}

/// A refuted check is never published, so its finding arrives on an
/// envelope-only refusal; it must still name the one entry to re-author
/// instead of sending every entry back.
#[test]
fn an_unpublished_refutation_retries_only_the_refuted_entry() {
    let driver = r#"
const ids = new Set(["AC-A-001", "AC-A-002"]);
const findings = [{
  subject: "AC-A-002",
  text: "check 'AC-A-002' was refuted by the host judge; reason: \"weak\"; counterexample: \"a passing false state\"; replace the check with one that fails in that state",
  remediation_scope: "candidate_artifact",
}];
console.log(JSON.stringify([...acceptanceRepairIds(findings, ids, false)]));
"#;
    assert_eq!(super::run_js(driver), r#"["AC-A-002"]"#);
}
