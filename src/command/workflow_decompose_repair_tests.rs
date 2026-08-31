//! The phase loop must consume the findings it routes.
//!
//! Driven under `node` by extracting the loop and its helpers from the embedded
//! script, the same way the v3 prelude tests exercise prelude functions.

use super::FIXED_SCRIPT_SOURCE;

/// Slice one top-level `function NAME(...) { ... }` or `const NAME = ...;` out
/// of the embedded script. Top-level declarations end at a closing brace in
/// column zero, so no brace counting is needed.
fn decl(name: &str) -> String {
    let source = FIXED_SCRIPT_SOURCE;
    for marker in [
        format!("async function {name}("),
        format!("function {name}("),
        format!("const {name} ="),
    ] {
        let Some(start) = source.find(&marker) else {
            continue;
        };
        let rest = &source[start..];
        let end = if marker.contains("function") {
            rest.find("\n}").expect("function must close in column zero") + 2
        } else {
            rest.find(";\n").expect("const must end in a semicolon") + 1
        };
        return rest[..end].to_string();
    }
    panic!("script must declare {name}");
}

fn run_js(driver: &str) -> String {
    let mut script = String::new();
    for name in [
        "OPERATIONAL_ATTEMPTS",
        "routeFindings",
        "requireCommitted",
        "authorPrompt",
        "authorCandidate",
    ] {
        script.push_str(&decl(name));
        script.push('\n');
    }
    script.push_str(driver);
    script.push('\n');
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("phase.mjs");
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A committed artifact carrying a repairable finding must be re-authored.
///
/// The gate publishes in observe mode, so the phase used to return on the first
/// commit and discard the routing it had just computed. That is how a task set
/// with 19 shadow findings — including an acceptance floor the gate reported as
/// not falsifiable — was published and then built on for a week.
const DRIVER: &str = r#"
globalThis.args = { gateMode: "observe" };
let authorCalls = 0;
const committed = (findings) => ({
  publicationReceipt: { id: "r" },
  postcondition: { satisfied: true },
  gateEnvelope: { policy_findings: findings },
});
const w = {
  agent: async () => {
    authorCalls += 1;
    return { status: "accepted", stopReason: "end_turn", content: "{\"candidate\":true}" };
  },
  hostCommand: async () =>
    authorCalls === 1
      ? committed([{ text: "floor is not falsifiable", remediation_scope: "candidate_artifact" }])
      : committed([]),
};
const policy = {
  phase: "acceptance",
  capability: "freeze-acceptance",
  attempts: 4,
  retryScopes: new Set(["candidate_artifact"]),
  prompt: () => "author the acceptance contract",
};
authorCandidate(w, policy).then(
  () => console.log(JSON.stringify({ authorCalls })),
  (error) => console.log(JSON.stringify({ error: String(error && error.message) })),
);
"#;

#[test]
fn a_repairable_finding_on_a_committed_artifact_drives_another_author_attempt() {
    assert_eq!(
        run_js(DRIVER),
        r#"{"authorCalls":2}"#,
        "the phase must feed a candidate_artifact finding back to the author instead of \
         publishing the artifact and discarding the finding"
    );
}

fn driver(first_findings: &str, attempts: u32, always_dirty: bool) -> String {
    format!(
        r#"
globalThis.args = {{ gateMode: "observe" }};
let authorCalls = 0;
const committed = (findings) => ({{
  publicationReceipt: {{ id: "r" }},
  postcondition: {{ satisfied: true }},
  gateEnvelope: {{ policy_findings: findings }},
}});
const w = {{
  agent: async () => {{
    authorCalls += 1;
    return {{ status: "accepted", stopReason: "end_turn", content: "{{}}" }};
  }},
  hostCommand: async () =>
    ({always_dirty} || authorCalls === 1) ? committed({first_findings}) : committed([]),
}};
const policy = {{
  phase: "acceptance",
  capability: "freeze-acceptance",
  attempts: {attempts},
  retryScopes: new Set(["candidate_artifact"]),
  prompt: () => "author",
}};
authorCandidate(w, policy).then(
  (outcome) => console.log(JSON.stringify({{ authorCalls, committed: Boolean(outcome && outcome.publicationReceipt) }})),
  (error) => console.log(JSON.stringify({{ authorCalls, error: String(error && error.message) }})),
);
"#
    )
}

/// A PRD-level finding is not the author's to repair; spinning the budget
/// against an unfixable input and shipping the artifact anyway is strictly
/// worse than stopping with a named defect.
#[test]
fn a_prd_input_finding_stops_the_phase_in_observe_mode() {
    let out = run_js(&driver(
        r#"[{ "text": "duplicate obligation id", "remediation_scope": "prd_input" }]"#,
        4,
        true,
    ));
    assert!(
        out.contains("\"error\"") && out.contains("duplicate obligation id"),
        "a prd_input finding must stop the phase rather than be re-authored: {out}"
    );
    assert!(
        out.contains("\"authorCalls\":1"),
        "it must stop on the first attempt, not consume the budget: {out}"
    );
}

/// Observe mode still never blocks: an exhausted budget falls back to the best
/// committed artifact rather than failing the run.
#[test]
fn observe_falls_back_to_the_last_committed_artifact_when_the_budget_is_spent() {
    let out = run_js(&driver(
        r#"[{ "text": "floor is not falsifiable", "remediation_scope": "candidate_artifact" }]"#,
        2,
        true,
    ));
    assert_eq!(
        out, r#"{"authorCalls":2,"committed":true}"#,
        "observe must spend its attempts repairing and then return the committed artifact: {out}"
    );
}

/// An exhausted budget must keep the best artifact, not the most recent one.
///
/// Attempts do not improve monotonically. Live run wf-6efe3de7 produced
/// findings 2, 1, 2, 1, 1 and then a malformed candidate, so keeping the latest
/// commit froze a vacuous acceptance floor that two earlier attempts had
/// already fixed — the repair loop found better artifacts and discarded them.
#[test]
fn an_exhausted_budget_keeps_the_best_committed_artifact_not_the_latest() {
    let driver = r#"
globalThis.args = { gateMode: "observe" };
let call = 0;
const finding = (n) => Array.from({ length: n }, (_, i) => ({
  text: "defect " + i, remediation_scope: "candidate_artifact",
}));
const w = {
  agent: async () => {
    call += 1;
    return { status: "accepted", stopReason: "end_turn", content: "{}" };
  },
  // Two findings, then one, then two: the middle attempt is the best artifact.
  hostCommand: async () => ({
    publicationReceipt: { id: "commit-" + call },
    postcondition: { satisfied: true },
    gateEnvelope: { policy_findings: finding(call === 2 ? 1 : 2) },
  }),
};
const policy = {
  phase: "acceptance",
  capability: "freeze-acceptance",
  attempts: 3,
  retryScopes: new Set(["candidate_artifact"]),
  prompt: () => "author",
};
authorCandidate(w, policy).then(
  (outcome) => console.log(JSON.stringify({ kept: outcome.publicationReceipt.id })),
  (error) => console.log(JSON.stringify({ error: String(error && error.message) })),
);
"#;
    assert_eq!(
        run_js(driver),
        r#"{"kept":"commit-2"}"#,
        "the phase must freeze the artifact with the fewest findings, not the last one authored"
    );
}

/// The skeleton shape must show what a deliverable contract contains.
///
/// It showed `deliverable_contracts: []` while every neighbouring field had a
/// filled example, so authors copied the empty placeholder. Both frozen
/// skeletons on runs wf-c5243dd1 and wf-3b65c2ed carried empty contracts on
/// every task, which is unclearable: the `consumes` check requires a producer
/// to declare the artifact its dependent reads, and the `graph` check requires
/// at least one positive instance obligation. The author oscillated between the
/// two for its whole budget.
#[test]
fn the_skeleton_shape_shows_a_populated_deliverable_contract() {
    let shape = decl("SKELETON_SHAPE");
    assert!(
        !shape.contains("deliverable_contracts: []"),
        "an empty example teaches an empty answer: {shape}"
    );
    assert!(
        shape.contains("artifact_path") && shape.contains("min_instances"),
        "a producer must be shown declaring the artifact it produces with a \
         positive instance obligation: {shape}"
    );
}
