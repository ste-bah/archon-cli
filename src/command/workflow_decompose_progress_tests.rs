//! Issue 261: no-progress windows advance authoring strategy and retain a
//! resumable pause when the existing repair rungs are exhausted, so repeated
//! host findings cannot keep an unattended loop running indefinitely.
//!
//! The whole embedded script runs under `node` against a scripted host, the
//! way the set-gate tests drive it. The scripted host records author calls,
//! gate outcomes and rung events.

use super::repair_tests::script_source;

/// `scenario` is a JS object literal with optional members:
/// - `answer(n)`: the agent reply for author call `n` (1-based); defaults to a
///   complete reply.
/// - `findings(n)`: the finding texts (or finding objects) the gate returns
///   for its `n`-th call; defaults to none.
/// - `operational(n, capability)`: a gate operational error text for the
///   gate's `n`-th call, or null; defaults to none.
/// - `resumed`: pause ids already credited by the scripted host, including
///   authored repair windows explicitly resumed by the scenario.
///
/// `entry` is the expression the driver awaits; `subject` and `w` are in scope.
pub(crate) fn run(gate_mode: &str, scenario: &str, entry: &str) -> serde_json::Value {
    let source = script_source();
    let driver = format!(
        r##"{source}
// Scheduling fixture: the real native validator is tested by shape_tests.
globalThis.__archonValidateAcceptanceEntry = () => "[]";
// Explicit Node fixture binding; host redaction parity is covered in QuickJS.
globalThis.__archonRedactPublicText = text => String(text).replace(/held-[^\s]+/g, "[REDACTED]");
// Issue 288: the host's author-context binding, its files kept nowhere.
const stubDigest = (text) => [...String(text)].reduce((h, c) => Math.imul(h ^ c.codePointAt(0), 16777619) >>> 0, 2166136261).toString(16).padStart(8, "0").repeat(8);
globalThis.__archonAuthorContext = (ext, text) => JSON.stringify({{ path: "/run/author-context/" + stubDigest(text) + "." + ext, sha256: stubDigest(text) }});
const scenario = {scenario};
globalThis.args = Object.assign({{
  projectRoot: "/p", repositoryRoot: "/r", prdPath: "/p/prd.md", prdDigest: "d", taskRoot: "/p/tasks",
  gateMode: "{gate_mode}", acceptanceCriteria: {{ "AC-X-001": "criterion" }},
}}, scenario.args || {{}});
const calls = [];
const prompts = [];
const pauses = [];
const events = [];
const terminalStops = [];
let lands = 0;
const work = [
  [{{ taskId: "TASK-X-001", fileName: "TASK-X-001.md" }}, []],
  [{{ taskId: "TASK-X-002", fileName: "TASK-X-002.md" }}, []],
];
const bodies = new Map();
const w = {{ checkpoint: async (_id, options) => {{ console.log(options.task); }},
  agent: async (id, options) => {{
    calls.push(id);
    prompts.push(options.task);
    events.push("author:" + id);
    if (scenario.answer) return scenario.answer(calls.length, id);
    const content = id.startsWith("acceptance-author-") ? JSON.stringify({{ id: "AC-X-001" }}) : "# body";
    return {{ status: "accepted", stopReason: "end_turn", content }};
  }},
  hostCommand: async (capability, options) => {{
    lands += 1;
    events.push("gate:" + (options?.target?.taskId || capability));
    const texts = scenario.findings ? scenario.findings(lands, capability, options) : [];
    const callId = capability + "-" + lands;
    const operational = scenario.operational ? scenario.operational(lands, capability) : null;
    if (operational) return {{ gateEnvelope: {{ policy_findings: [], operational_error: {{ kind: "operational", text: operational }} }} }};
    return {{
      publicationReceipt: {{ call_id: callId }},
      result: {{ data: {{ publicationReceipt: {{ call_id: callId }} }} }},
      postcondition: {{ satisfied: true }},
      gateEnvelope: {{ policy_findings: texts.map((text) => typeof text === "string" ? {{ text, remediation_scope: "body" }} : text) }},
      subjects: capability.endsWith("skeleton") ? [{{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }}] : [],
    }};
  }},
  pause: async (id, evidence) => {{
    pauses.push({{ id, evidence }});
    events.push("pause:" + id);
    if ((scenario.resumed || []).includes(id)) return {{ resumed: true, pause_id: id }};
    throw new Error("workflow paused by run control: script requested pause " + id);
  }},
  finalReport: async () => ({{}}),
}};
globalThis.__archonHost = async (method, payload) => {{
  const stop = JSON.parse(payload);
  terminalStops.push({{ method, stop }});
  throw new Error(stop.reason);
}};
const subject = {{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }};
Promise.resolve().then(() => {entry}).then(
  () => console.log(JSON.stringify({{ accepted: true, calls: calls.length, lands, pauses, events, prompts, terminalStops }})),
  (error) => console.log(JSON.stringify({{ error: String(error && error.message), calls: calls.length, lands, pauses, events, prompts, terminalStops }})),
);
"##
    );
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("progress.cjs");
    std::fs::write(&path, driver).expect("write driver");
    let out = std::process::Command::new("node")
        .arg("-e")
        .arg("const r = require('node:child_process').spawnSync(process.execPath, [process.argv[1]], {timeout:30000,stdio:'inherit'}); if (r.error) console.error(r.error); process.exitCode = r.status ?? 1;")
        .arg(&path)
        .output()
        .expect("node must be available");
    assert!(
        out.status.success(),
        "driver failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<_> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    let mut value: serde_json::Value = lines
        .iter()
        .rev()
        .find_map(|line| serde_json::from_str(line).ok())
        .expect("driver json after progress events");
    value["progress_events"] = serde_json::Value::Array(
        lines
            .iter()
            .filter(|line| !line.starts_with('{'))
            .map(|line| serde_json::Value::String(line.clone()))
            .collect(),
    );
    value
}

pub(crate) const BODY: &str = "authorCandidate(w, bodyPolicy(subject, []))";

pub(super) fn body(scenario: &str) -> serde_json::Value {
    run("enforce", scenario, BODY)
}

pub(super) fn pause_ids(out: &serde_json::Value) -> Vec<String> {
    out["pauses"]
        .as_array()
        .expect("pauses")
        .iter()
        .map(|pause| pause["id"].as_str().expect("pause id").to_string())
        .collect()
}

// --- progress keeps a subject going past the old fixed budget ---------------

#[test]
fn a_subject_that_keeps_reducing_its_findings_is_retried_past_the_old_body_budget() {
    // Twelve refused attempts, each with one host defect fewer: the old
    // budget (BODY_ATTEMPTS = 10) failed the run on the tenth.
    let out = body(
        r#"{ findings: (n) => n <= 12 ? Array.from({ length: 13 - n }, (_, i) => "candidate artifact was refused: defect " + String.fromCharCode(97 + i)) : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 13, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn an_authored_stall_walks_the_repair_ladder_and_cycles_without_pausing() {
    let out = body(r#"{ findings: (n) => n <= 13 ? ["defect alpha"] : [] }"#);
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
    let prompts = out["prompts"].as_array().unwrap();
    assert!(
        prompts.iter().any(|prompt| prompt
            .as_str()
            .unwrap()
            .contains("Re-author a complete replacement from fresh context")),
        "{prompts:?}"
    );
    assert!(
        prompts.iter().any(|prompt| prompt
            .as_str()
            .unwrap()
            .contains("smaller groups while preserving every finding")),
        "{prompts:?}"
    );
    assert!(
        prompts.iter().any(|prompt| prompt
            .as_str()
            .unwrap()
            .contains("deferred while independent subjects settle")),
        "{prompts:?}"
    );
    assert!(
        prompts.iter().any(|prompt| prompt
            .as_str()
            .unwrap()
            .contains("Re-author a complete replacement from fresh context")
            && prompt.as_str().unwrap().contains("Logical attempt: 14.")),
        "cycle returns to fresh context with a new deterministic call: {prompts:?}"
    );
    let transitions: Vec<_> = out["progress_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|line| line.as_str().filter(|line| line.contains("repair rung")))
        .collect();
    assert!(
        transitions[0].contains("r2")
            && transitions[1].contains("r3")
            && transitions[2].contains("r4")
            && transitions[3].contains("r2"),
        "{transitions:?}"
    );
}

#[test]
fn findings_that_are_all_new_at_the_same_count_advance_the_ladder() {
    // A new set of the same size does not establish a new minimum.
    let out = body(
        r#"{ findings: (n) => n <= 13 ? ["defect " + String.fromCharCode(96 + n) + "x", "defect " + String.fromCharCode(96 + n) + "y"] : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn a_new_minimum_between_regressions_keeps_the_window_open() {
    // 5, 6, 4, 5, 3, 4, 2, 3, 1, 2, then clean: attempts do not improve
    // monotonically, but every second one sets a new minimum.
    let out = body(
        r#"{ findings: (n) => { const counts = [5, 6, 4, 5, 3, 4, 2, 3, 1, 2]; return n <= counts.length ? ["a", "b", "c", "d", "e", "f"].slice(0, counts[n - 1]).map((x) => "candidate artifact was refused: defect " + x) : []; } }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 11, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

// --- no progress pauses the run with evidence -------------------------------

#[test]
fn repeating_the_same_findings_advances_the_authoring_ladder() {
    let out = body(r#"{ findings: (n) => n <= 13 ? ["defect alpha"] : [] }"#);
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
    assert!(
        out["progress_events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line.as_str().unwrap().contains("repair rung r4")),
        "{out}"
    );
}

#[test]
fn an_author_alternating_between_two_findings_advances_the_ladder() {
    let out = body(
        r#"{ findings: (n) => n <= 13 ? [n % 2 ? "floor is not falsifiable" : "refuted by the judge"] : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn changing_numbers_inside_identifiers_still_advance_the_ladder() {
    // Different names at the same count are not measured progress.
    let out = body(
        r#"{ findings: (n) => n <= 13 ? ["check 'AC-X-00" + n + "' was refuted by the host judge at line " + (10 + n)] : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn repeated_packaging_refusals_advance_the_ladder() {
    let out = body(
        r#"{ findings: (n) => n <= 13 ? [{ text: "candidate artifact was refused: the reply is not a JSON document (" + ["key must be a string", "expected value", "trailing comma", "eof"][n % 4] + ")", remediation_scope: "candidate_artifact" }] : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn incomplete_provider_outcomes_advance_the_ladder_then_accept() {
    let out = body(
        r##"{ answer: (n) => n <= 12 ? ({ status: "accepted", stopReason: "max_tokens", content: "cut" }) : ({ status: "accepted", stopReason: "end_turn", content: "# body" }) }"##,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

/// Issue 288: a stall pauses observe too (resumable); a committed artifact
/// with open findings is never returned.
#[test]
fn observe_mode_reauthors_a_stalled_candidate_instead_of_returning_it() {
    let out = run(
        "observe",
        r#"{ findings: (n) => n <= 13 ? ["defect alpha"] : [] }"#,
        BODY,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

// --- resume -----------------------------------------------------------------

#[test]
fn a_replayed_rung_call_sequence_completes_without_operator_resume() {
    let out = body(r#"{ findings: (n) => n <= 13 ? ["defect alpha"] : [] }"#);
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
    let rung_events: Vec<_> = out["progress_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event.as_str().filter(|event| event.contains("repair rung")))
        .collect();
    assert_eq!(
        rung_events.len(),
        4,
        "the same call sequence reconstructs the ladder: {rung_events:?}"
    );
}

// --- operational attempts follow the same rule ------------------------------

#[test]
fn consecutive_operational_failures_record_the_host_retry_window_without_resume_advice() {
    let out = body(
        r#"{ answer: () => ({ status: "failed", summary: "agent transport failed: stream ended" }) }"#,
    );
    assert_eq!(pause_ids(&out).len(), 1, "{out}");
    assert_eq!(
        out["pauses"][0]["evidence"]["reason"],
        "operational_no_progress"
    );
    let evidence = out["pauses"][0]["evidence"].to_string();
    assert!(
        evidence.contains("agent transport failed: stream ended"),
        "{out}"
    );
    assert!(!evidence.contains("resume"), "{out}");
    assert!(out["terminalStops"].as_array().unwrap().is_empty(), "{out}");
}

#[test]
fn operational_failures_between_answers_do_not_pause() {
    let out = body(
        r#"{ answer: (n) => [1, 2, 4, 5].includes(n) ? { status: "failed", summary: "blip" } : { status: "accepted", stopReason: "end_turn", content: "body text" }, findings: (n) => n === 1 ? ["defect alpha"] : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 6, "{out}");
}

#[test]
fn operational_replay_retains_evidence_and_retries_until_recovery() {
    let out = body(
        r#"{ resumed: ["pause-body-TASK-X-010-1"], answer: (n) => n <= 5 ? {status:"failed",summary:"down"} : {status:"accepted",stopReason:"end_turn",content:"complete"} }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 6, "{out}");
    assert_eq!(pause_ids(&out).len(), 1, "{out}");
    let recovery = out["pauses"][0]["evidence"]["recovery"].as_str().unwrap();
    assert!(
        recovery.contains("down") && recovery.contains("retries"),
        "{out}"
    );
    assert!(!recovery.contains("resume"), "{out}");
}

#[test]
fn an_acceptance_entry_that_never_parses_advances_the_ladder() {
    let out = run(
        "enforce",
        r##"{ answer: (n, id) => id.startsWith("acceptance-author-") && n <= 13 ? ({ status: "accepted", stopReason: "end_turn", content: "I could not produce an entry." }) : ({ status: "accepted", stopReason: "end_turn", content: id.startsWith("acceptance-author-") ? JSON.stringify({ id: "AC-X-001" }) : "# body" }) }"##,
        "workflow(w)",
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn a_stalled_body_waits_for_independent_bodies_then_reenters_without_pause() {
    let out = run(
        "enforce",
        r##"{
          answer: (_n, id) => ({ status: "accepted", stopReason: "end_turn", content: "# body " + id }),
          findings: (_n, _capability, options) => options?.target?.taskId === "TASK-X-001" && calls.filter(id => id.includes("TASK-X-001")).length <= 10
            ? ["candidate artifact was refused: repeated body defect"] : []
        }"##,
        "authorBodies(w, work, bodies)",
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
    let events = out["events"].as_array().unwrap();
    let sibling_gate = events
        .iter()
        .position(|event| event == "gate:TASK-X-002")
        .unwrap();
    let pending = events
        .iter()
        .position(|event| event == "author:body-TASK-X-001-author-11")
        .unwrap();
    assert!(
        sibling_gate < pending,
        "independent body must settle before the deferred subject re-enters: {events:?}"
    );
    assert_eq!(
        out["calls"], 12,
        "the deferred subject retries only after its sibling settles: {out}"
    );
}

#[path = "workflow_decompose_progress_rule_tests.rs"]
mod rule;

#[path = "workflow_decompose_progress_bound_tests.rs"]
mod bound;

#[path = "workflow_decompose_progress_stage_tests.rs"]
mod stage;
