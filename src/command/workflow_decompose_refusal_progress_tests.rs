//! Issue 261: refusals the host decides without a judge, under the progress
//! rule. Driven like the other phase-loop tests, by extracting the loop and
//! its helpers from the embedded script.

use super::run_js;

/// A refusal the host could not even parse is packaging. Issue 261: it is
/// ranked below every parsed candidate and one packaging refusal is the same
/// defect as the next, whatever the parser said, so repeated packaging makes
/// no progress and closes the stall window (observe then keeps the best
/// committed artifact). It used to be refunded against a fixed budget.
#[test]
fn repeated_packaging_refusals_make_no_progress_and_close_the_window() {
    let driver = r#"
globalThis.args = { gateMode: "observe" };
let call = 0;
const w = {
  agent: async () => { call += 1; return { status: "accepted", stopReason: "end_turn", content: "{}" }; },
  hostCommand: async () => ({
    publicationReceipt: { id: "c" + call },
    postcondition: { satisfied: true },
    gateEnvelope: { policy_findings: [{
      text: "candidate artifact was refused: the reply is not a JSON document (key must be a string at line " + call + " column 7)",
      remediation_scope: "candidate_artifact",
    }] },
  }),
};
const policy = {
  phase: "acceptance", capability: "freeze-acceptance",
  retryScopes: new Set(["candidate_artifact"]), prompt: () => "author",
};
authorCandidate(w, policy).then(() => {
  console.log(JSON.stringify({ calls: call }));
}, (e) => console.log(JSON.stringify({ calls: call, error: String(e && e.message) })));
"#;
    let out = run_js(driver);
    assert!(
        out.contains("\"calls\":4") && !out.contains("error"),
        "a baseline and three packaging repeats, then the best committed artifact: {out}"
    );
}

/// Distinct refusal text at the same count must pause, preserving repair history.
#[test]
fn distinct_deterministic_acceptance_refusals_at_the_same_count_pause() {
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
w.pause = async () => { throw Error("paused"); };
const policy = {phase:"acceptance",capability:"freeze-acceptance",retryScopes:new Set(["candidate_artifact"]),prompt:()=>"author"};
authorCandidate(w,policy).then(() => { throw Error("must pause"); }).catch(e => {
  if (e.message !== "paused" || !prompts[3].includes("command_semantics")) { console.error(e); process.exitCode=1; }
  else console.log(calls);
});
"#;
    assert_eq!(run_js(script), "4");
}

/// The same mechanical refusal forever is a broken prompt: the loop pauses
/// the run with the exact defect as its evidence (Issue 261: it used to fail
/// after a flat allowance of twelve), never falls back to a dirty artifact.
#[test]
fn endless_deterministic_refusals_pause_with_the_exact_defect_not_a_dirty_fallback() {
    let script = r#"
globalThis.args = {gateMode:"observe"};
let calls = 0;
const pauses = [];
const w = {
  agent:async()=>{calls++; return {status:"accepted",stopReason:"end_turn",content:"{}"};},
  hostCommand:async()=>({gateEnvelope:{policy_findings:[{text:"candidate artifact was refused: unknown field `invented`",remediation_scope:"candidate_artifact"}]}}),
  pause:async(id, evidence)=>{pauses.push({id, evidence}); throw new Error("workflow paused by run control: " + id);},
};
authorCandidate(w,{phase:"acceptance",retryScopes:new Set(["candidate_artifact"]),prompt:()=>"author"}).then(
  ()=>{throw Error("must stop");},
  e=>console.log(JSON.stringify({calls,error:e.message,pauses})),
);
"#;
    let output: serde_json::Value = serde_json::from_str(&run_js(script)).unwrap();
    assert_eq!(
        output["calls"], 4,
        "a baseline and the stall window: {output}"
    );
    assert!(
        output["error"]
            .as_str()
            .unwrap()
            .contains("workflow paused by run control"),
        "{output}"
    );
    assert_eq!(output["pauses"][0]["id"], "pause-acceptance-1");
    assert!(
        output["pauses"][0]["evidence"]["last_findings"][0]
            .as_str()
            .unwrap()
            .contains("unknown field `invented`"),
        "{output}"
    );
}
