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
            rest.find("\n}")
                .expect("function must close in column zero")
                + 2
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
        "PACKAGING_REFUNDS",
        "ACCEPTANCE_REFUSAL_REFUNDS",
        "PACKAGING_REFUSAL",
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

/// Repair feedback must carry the attempt history, not just the latest findings.
///
/// Two gates can be individually satisfiable and jointly hard: on decomposition
/// wf-94fe1896 the acceptance author alternated between a floor the judge could
/// refute and a floor that was not falsifiable, because each attempt saw only
/// the current findings and never learned it had already been in the other
/// state. It spent all six attempts oscillating. An earlier run escaped the same
/// loop by chance. Chance is not a mechanism.
#[test]
fn repair_feedback_carries_what_earlier_attempts_already_tried() {
    let driver = r#"
globalThis.args = { gateMode: "observe" };
let prompts = [];
let call = 0;
const w = {
  agent: async (_id, opts) => {
    prompts.push(opts.task);
    call += 1;
    return { status: "accepted", stopReason: "end_turn", content: "{}" };
  },
  // Alternates between two findings, exactly like the live acceptance gate.
  hostCommand: async () => ({
    publicationReceipt: { id: "c" + call },
    postcondition: { satisfied: true },
    gateEnvelope: { policy_findings: [{
      text: call % 2 === 1 ? "floor is not falsifiable" : "refuted by the host judge",
      remediation_scope: "candidate_artifact",
    }] },
  }),
};
const policy = {
  phase: "acceptance", capability: "freeze-acceptance", attempts: 3,
  retryScopes: new Set(["candidate_artifact"]), prompt: () => "author",
};
authorCandidate(w, policy).then(() => {
  const third = prompts[2] || "";
  console.log(JSON.stringify({
    sees_both: third.includes("not falsifiable") && third.includes("refuted by the host judge"),
  }));
}, (e) => console.log(JSON.stringify({ error: String(e && e.message) })));
"#;
    assert_eq!(
        run_js(driver),
        r#"{"sees_both":true}"#,
        "by the third attempt the author must see both findings it has already \
         triggered, or it will keep alternating between them"
    );
}

/// A refusal the host could not even parse is packaging: it is refunded
/// (bounded) rather than charged to the candidate budget, so quote slips
/// inside embedded commands cannot spend a phase's attempts on nothing.
#[test]
fn a_packaging_refusal_is_refunded_and_the_refund_is_bounded() {
    let driver = r#"
globalThis.args = { gateMode: "observe" };
let call = 0;
const w = {
  agent: async () => { call += 1; return { status: "accepted", stopReason: "end_turn", content: "{}" }; },
  hostCommand: async () => ({
    publicationReceipt: { id: "c" + call },
    postcondition: { satisfied: true },
    gateEnvelope: { policy_findings: [{
      text: "candidate artifact was refused: the reply is not a JSON document (key must be a string at line 1 column 7)",
      remediation_scope: "candidate_artifact",
    }] },
  }),
};
const policy = {
  phase: "acceptance", capability: "freeze-acceptance", attempts: 2,
  retryScopes: new Set(["candidate_artifact"]), prompt: () => "author",
};
authorCandidate(w, policy).then(() => {
  console.log(JSON.stringify({ calls: call }));
}, (e) => console.log(JSON.stringify({ calls: call, error: String(e && e.message) })));
"#;
    let out = run_js(driver);
    assert!(
        out.contains("\"calls\":5"),
        "2 attempts + 3 refunds = 5 author calls before the budget is spent: {out}"
    );
}

#[test]
fn deterministic_acceptance_refusals_do_not_spend_judged_attempts() {
    let script = r#"
globalThis.args = { gateMode: "observe" };
let calls = 0;
const prompts = [];
const reasons = ["unknown field `command_semantics`", "missing checks for AC-X-002", "floor needs typed_verifier_command", "example id is not defined"];
const w = {
  agent: async (_, input) => { prompts.push(input.task); calls++; return {status:"accepted",stopReason:"end_turn",content:"{}"}; },
  hostCommand: async () => calls <= reasons.length
    ? {gateEnvelope:{policy_findings:[{text:`candidate artifact was refused: ${reasons[calls-1]}`,remediation_scope:"candidate_artifact"}]}}
    : {publicationReceipt:{id:"r"},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}},
};
const policy = {phase:"acceptance",capability:"freeze-acceptance",attempts:1,retryScopes:new Set(["candidate_artifact"]),prompt:()=>"author"};
authorCandidate(w,policy).then(() => {
  if (!prompts.every(p => p.includes("Logical attempt: 1."))) throw Error("spent a judged attempt");
  if (!prompts[4].includes("command_semantics")) throw Error("lost repair history");
  console.log(calls);
}).catch(e => { console.error(e); process.exitCode=1; });
"#;
    assert_eq!(run_js(script), "5");
}

#[test]
fn endless_deterministic_refusals_stop_with_the_exact_defect_not_a_dirty_fallback() {
    let script = r#"
globalThis.args = {gateMode:"observe"};
let calls = 0;
const w = {
  agent:async()=>{calls++; return {status:"accepted",stopReason:"end_turn",content:"{}"};},
  hostCommand:async()=>({gateEnvelope:{policy_findings:[{text:"candidate artifact was refused: unknown field `invented`",remediation_scope:"candidate_artifact"}]}}),
};
authorCandidate(w,{phase:"acceptance",attempts:1,retryScopes:new Set(["candidate_artifact"]),prompt:()=>"author"}).then(
  ()=>{throw Error("must stop");},
  e=>console.log(JSON.stringify({calls,error:e.message})),
);
"#;
    let output: serde_json::Value = serde_json::from_str(&run_js(script)).unwrap();
    assert_eq!(
        output["calls"], 12,
        "the mechanical allowance is flat, not per attempt"
    );
    assert!(
        output["error"]
            .as_str()
            .unwrap()
            .contains("unknown field `invented`")
    );
}

#[test]
fn the_first_author_receives_check_alternatives_and_the_falsifiability_standard() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prompt.mjs");
    let script = format!(
        r#"{FIXED_SCRIPT_SOURCE}
globalThis.args = {{projectRoot:"p",prdPath:"p.md",prdDigest:"d",taskRoot:"tasks",gateMode:"observe"}};
workflow({{agent:async(_, input)=>{{console.log(input.task);throw Error("captured");}}}}).catch(e=>{{if(e.message!=="captured") throw e;}});
"#
    );
    std::fs::write(&path, script).unwrap();
    let output = std::process::Command::new("node")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prompt = String::from_utf8(output.stdout).unwrap();
    for required in [
        "not how many entries",
        "every acceptance id",
        "must fail in every state",
        "typed_verifier_command",
        "\"kind\":\"command\"",
        "Only prd, gap_policy, criterion text and every judgment",
    ] {
        assert!(
            prompt.contains(required),
            "missing author guidance: {required}"
        );
    }
}

#[test]
fn mcp_obligation_body_prompt_requires_project_specific_declarations() {
    let shape = decl("BODY_SHAPE");
    assert!(!shape.contains("required_tools: []"));
    let workflow = decl("workflow");
    assert!(workflow.contains(".mcp.json"));
    assert!(workflow.contains("every declared tool"));
}

#[test]
fn acceptance_entries_are_separate_calls_and_truncation_retries_only_one() {
    let dir = tempfile::tempdir().unwrap();
    let script = format!("{}\n{}", FIXED_SCRIPT_SOURCE, r#"
globalThis.args = { projectRoot:'/p', prdPath:'/p/prd', prdDigest:'x', taskRoot:'/p/tasks', gateMode:'observe', acceptanceCriteria:{'AC-X-001':'first','AC-X-002':'second'} };
let calls = [], freezes = [], failed = false;
const w = {
 finalReport: async ()=>({}),
 agent: async (id, options) => {
  calls.push(id);
  if(id.includes('AC-X-002') && !failed) { failed=true; return {status:'accepted',stopReason:'max_tokens',content:'cut'}; }
  return {status:'accepted',stopReason:'end_turn',content:JSON.stringify({id:id.includes('AC-X-002')?'AC-X-002':'AC-X-001'})};
 },
 hostCommand: async (cap, options) => {
  if(cap==='freeze-acceptance') freezes.push(JSON.parse(options.stdin));
  return {publicationReceipt:{call_id:cap},result:{data:{publicationReceipt:{call_id:cap}}},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]},subjects:[{taskId:'TASK-X-001',fileName:'TASK-X-001.md'}]};
 }
};
workflow(w).then(()=>{
 const a=calls.filter(x=>x.startsWith('acceptance-author-'));
 if(a.length!==3 || !a[0].includes('AC-X-001') || !a[1].includes('AC-X-002') || !a[2].includes('AC-X-002')) throw Error(JSON.stringify(calls));
 if(freezes[0].entries.length!==2) throw Error('not entry envelope');
}).catch(e=>{console.error(e);process.exitCode=1;});
"#);
    let path = dir.path().join("entry-test.cjs");
    std::fs::write(&path, script).unwrap();
    let out = std::process::Command::new("node").arg(path).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}
