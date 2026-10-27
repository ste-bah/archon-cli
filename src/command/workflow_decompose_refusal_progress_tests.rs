//! Issue 261: refusals the host decides without a judge, under the progress
//! rule. Driven like the other phase-loop tests, by extracting the loop and
//! its helpers from the embedded script.

use super::run_js;

/// A refusal the host could not even parse is packaging. Issue 261: it is
/// ranked below every parsed candidate and one packaging refusal is the same
/// defect as the next, whatever the parser said, so repeated packaging makes
/// no progress and closes the stall window, which pauses observe too (Issue
/// 288: it used to keep the best committed artifact). It used to be refunded
/// against a fixed budget.
#[test]
fn repeated_packaging_refusals_advance_the_ladder_without_a_pause() {
    let driver = r#"
globalThis.args = { gateMode: "observe" };
let call = 0;
const w = { checkpoint: async () => {},
  agent: async () => { call += 1; return { status: "accepted", stopReason: "end_turn", content: "{}" }; },
  hostCommand: async () => call <= 13 ? ({
    publicationReceipt: { id: "c" + call },
    postcondition: { satisfied: true },
    gateEnvelope: { policy_findings: [{
      text: "candidate artifact was refused: the reply is not a JSON document (key must be a string at line " + call + " column 7)",
      remediation_scope: "candidate_artifact",
    }] },
  }) : ({ publicationReceipt: { id: "clean" }, postcondition: { satisfied: true }, gateEnvelope: { policy_findings: [] } }),
};
w.pause = async (_, evidence) => { throw new Error("paused " + evidence.reason); };
const policy = {
  phase: "acceptance", capability: "freeze-acceptance",
  retryScopes: new Set(["candidate_artifact"]), prompt: () => "author",
};
authorCandidate(w, policy).then(() => {
  console.log(JSON.stringify({ calls: call }));
}, (e) => console.log(JSON.stringify({ calls: call, error: String(e && e.message) })));
"#;
    let out = run_js(driver);
    assert_eq!(
        out, r#"{"calls":14}"#,
        "packaging refusals reach a clean candidate after the ladder: {out}"
    );
}

/// Distinct refusal text at the same count must pause, preserving repair history.
#[test]
fn distinct_deterministic_acceptance_refusals_reach_a_clean_candidate() {
    let script = r#"
globalThis.args = { gateMode: "observe" };
let calls = 0;
const prompts = [];
const reasons = ["unknown field `command_semantics`", "missing checks for AC-X-002", "floor needs typed_verifier_command", "example id is not defined"];
const w = { checkpoint: async () => {},
  agent: async (_, input) => { prompts.push(input.task); calls++; return {status:"accepted",stopReason:"end_turn",content:"{}"}; },
  hostCommand: async () => calls <= reasons.length
    ? {gateEnvelope:{policy_findings:[{text:`candidate artifact was refused: ${reasons[calls-1]}`,remediation_scope:"candidate_artifact"}]}}
    : {publicationReceipt:{id:"r"},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}},
};
w.pause = async () => { throw Error("paused"); };
const policy = {phase:"acceptance",capability:"freeze-acceptance",retryScopes:new Set(["candidate_artifact"]),prompt:()=>"author"};
authorCandidate(w,policy).then(() => {
  if (!prompts[3].includes("command_semantics")) { console.error("earlier finding was lost"); process.exitCode=1; }
  else console.log(calls);
});
"#;
    assert_eq!(run_js(script), "5");
}

/// The same mechanical refusal forever is a broken prompt: the loop pauses
/// the run with the exact defect as its evidence (Issue 261: it used to fail
/// after a flat allowance of twelve), never falls back to a dirty artifact.
#[test]
fn repeated_deterministic_refusals_reach_clean_authoring_without_a_dirty_fallback() {
    let script = r#"
globalThis.args = {gateMode:"observe"};
let calls = 0;
const pauses = [];
const w = { checkpoint: async () => {},
  agent:async()=>{calls++; return {status:"accepted",stopReason:"end_turn",content:"{}"};},
  hostCommand:async()=>calls <= 13 ? ({gateEnvelope:{policy_findings:[{text:"candidate artifact was refused: unknown field `invented`",remediation_scope:"candidate_artifact"}]}}) : ({publicationReceipt:{id:"r"},postcondition:{satisfied:true},gateEnvelope:{policy_findings:[]}}),
  pause:async(id, evidence)=>{pauses.push({id, evidence}); throw new Error("workflow paused by run control: " + id);},
};
authorCandidate(w,{phase:"acceptance",retryScopes:new Set(["candidate_artifact"]),prompt:()=>"author"}).then(
  ()=>console.log(JSON.stringify({calls,pauses})),
  e=>console.log(JSON.stringify({calls,error:e.message,pauses})),
);
"#;
    let output: serde_json::Value = serde_json::from_str(&run_js(script)).unwrap();
    assert_eq!(
        output["calls"], 14,
        "the repair ladder reaches clean authoring: {output}"
    );
    assert!(output["pauses"].as_array().unwrap().is_empty(), "{output}");
}
