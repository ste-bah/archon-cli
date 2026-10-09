//! Issue 261: an author loop is limited by attempts that make no progress,
//! never by a fixed total, and a loop that stops PAUSES the run with evidence
//! instead of failing it.
//!
//! The whole embedded script runs under `node` against a scripted host, the
//! way the set-gate tests drive it. `w.pause` behaves as the live host does:
//! an id it has not seen pauses the run (the call rejects with the host's
//! pause text), and an id an earlier, resumed run already took answers
//! `{ resumed: true }`.

use super::FIXED_SCRIPT_SOURCE;

/// `scenario` is a JS object literal with optional members:
/// - `answer(n)`: the agent reply for author call `n` (1-based); defaults to a
///   complete reply.
/// - `findings(n)`: the finding texts (or finding objects) the gate returns
///   for its `n`-th call; defaults to none.
/// - `operational(n, capability)`: a gate operational error text for the
///   gate's `n`-th call, or null; defaults to none.
/// - `resumed`: pause ids a previous run already took.
///
/// `entry` is the expression the driver awaits; `subject` and `w` are in scope.
pub(crate) fn run(gate_mode: &str, scenario: &str, entry: &str) -> serde_json::Value {
    let driver = format!(
        r##"{FIXED_SCRIPT_SOURCE}
// Scheduling fixture: the real native validator is tested by shape_tests.
globalThis.__archonValidateAcceptanceEntry = () => "[]";
// Issue 288: the host's author-context binding, its files kept nowhere.
const stubDigest = (text) => [...String(text)].reduce((h, c) => Math.imul(h ^ c.codePointAt(0), 16777619) >>> 0, 2166136261).toString(16).padStart(8, "0").repeat(8);
globalThis.__archonAuthorContext = (ext, text) => JSON.stringify({{ path: "/run/author-context/" + stubDigest(text) + "." + ext, sha256: stubDigest(text) }});
const scenario = {scenario};
globalThis.args = Object.assign({{
  projectRoot: "/p", repositoryRoot: "/r", prdPath: "/p/prd.md", prdDigest: "d", taskRoot: "/p/tasks",
  gateMode: "{gate_mode}", acceptanceCriteria: {{ "AC-X-001": "criterion" }},
}}, scenario.args || {{}});
const calls = [];
const pauses = [];
let lands = 0;
const w = {{
  agent: async (id, options) => {{
    calls.push(id);
    if (scenario.answer) return scenario.answer(calls.length, id);
    const content = id.startsWith("acceptance-author-") ? JSON.stringify({{ id: "AC-X-001" }}) : "# body";
    return {{ status: "accepted", stopReason: "end_turn", content }};
  }},
  hostCommand: async (capability) => {{
    lands += 1;
    const texts = scenario.findings ? scenario.findings(lands, capability) : [];
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
    if ((scenario.resumed || []).includes(id)) return {{ resumed: true, pause_id: id }};
    throw new Error("workflow paused by run control: script requested pause " + id);
  }},
  finalReport: async () => ({{}}),
}};
const subject = {{ taskId: "TASK-X-010", fileName: "TASK-X-010.md" }};
Promise.resolve().then(() => {entry}).then(
  () => console.log(JSON.stringify({{ accepted: true, calls: calls.length, lands, pauses }})),
  (error) => console.log(JSON.stringify({{ error: String(error && error.message), calls: calls.length, lands, pauses }})),
);
"##
    );
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("progress.cjs");
    std::fs::write(&path, driver).expect("write driver");
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

pub(super) fn evidence(out: &serde_json::Value, index: usize) -> &serde_json::Value {
    &out["pauses"][index]["evidence"]
}

pub(super) fn progress_flags(evidence: &serde_json::Value) -> Vec<bool> {
    evidence["progress_history"]
        .as_array()
        .expect("progress history")
        .iter()
        .map(|entry| entry["progress"].as_bool().expect("progress flag"))
        .collect()
}

/// Asserts the run stopped on a pause, not on a failure: the last pause the
/// script requested is the one it ended with.
pub(super) fn assert_paused(out: &serde_json::Value) {
    let error = out["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("workflow paused by run control"),
        "the loop must end on a pause, not a failure: {out}"
    );
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
fn findings_that_are_all_new_at_the_same_count_pause() {
    // A new set of the same size does not establish a new minimum.
    let out = body(
        r#"{ findings: (n) => n <= 11 ? ["defect " + String.fromCharCode(96 + n) + "x", "defect " + String.fromCharCode(96 + n) + "y"] : [] }"#,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [true, false, false, false]
    );
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
fn repeating_the_same_findings_pauses_the_run_with_evidence() {
    let out = body(r#"{ findings: () => ["defect alpha"] }"#);
    assert_paused(&out);
    assert_eq!(pause_ids(&out), ["pause-body-TASK-X-010-1"], "{out}");
    assert_eq!(
        out["calls"], 4,
        "one baseline attempt and three without progress: {out}"
    );
    let evidence = evidence(&out, 0);
    assert_eq!(evidence["subject"], "body-TASK-X-010", "{evidence}");
    assert_eq!(evidence["reason"], "no_progress", "{evidence}");
    assert_eq!(evidence["author_calls"], 4, "{evidence}");
    assert_eq!(evidence["stall_window"], 3, "{evidence}");
    assert_eq!(progress_flags(evidence), [true, false, false, false]);
    // A judge finding is not measured: no host defect is outstanding.
    assert_eq!(evidence["progress_history"][3]["findings"], 0, "{evidence}");
    assert_eq!(evidence["last_findings"][0], "defect alpha", "{evidence}");
    assert!(
        evidence["recovery"]
            .as_str()
            .is_some_and(|text| text.contains("resume")),
        "{evidence}"
    );
}

#[test]
fn an_author_alternating_between_two_findings_pauses() {
    let out = body(
        r#"{ findings: (n) => [n % 2 ? "floor is not falsifiable" : "refuted by the judge"] }"#,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [true, false, false, false],
        "a different finding of the same count makes no progress"
    );
}

#[test]
fn changing_numbers_inside_identifiers_does_not_lower_the_defect_count() {
    // Different names at the same count are not measured progress.
    let out = body(
        r#"{ findings: (n) => n <= 6 ? ["check 'AC-X-00" + n + "' was refuted by the host judge at line " + (10 + n)] : [] }"#,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [true, false, false, false]
    );
}

#[test]
fn repeated_packaging_refusals_pause_whatever_the_parse_error_says() {
    let out = body(
        r#"{ findings: (n) => [{ text: "candidate artifact was refused: the reply is not a JSON document (" + ["key must be a string", "expected value", "trailing comma", "eof"][n % 4] + ")", remediation_scope: "candidate_artifact" }] }"#,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
    assert_eq!(
        evidence(&out, 0)["progress_history"][1]["kind"],
        "packaging"
    );
}

#[test]
fn incomplete_provider_outcomes_pause_without_a_baseline() {
    let out = body(
        r#"{ answer: () => ({ status: "accepted", stopReason: "max_tokens", content: "cut" }) }"#,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 3, "{out}");
    assert_eq!(out["lands"], 0, "nothing was judged: {out}");
    assert_eq!(
        evidence(&out, 0)["progress_history"][0]["kind"],
        "incomplete"
    );
}

/// Issue 288: a stall pauses observe too (resumable); a committed artifact
/// with open findings is never returned.
#[test]
fn observe_mode_pauses_on_a_stall_instead_of_returning_the_committed_artifact() {
    let out = run("observe", r#"{ findings: () => ["defect alpha"] }"#, BODY);
    assert_paused(&out);
    assert!(out.get("accepted").is_none(), "{out}");
    assert_eq!(out["calls"], 4, "{out}");
    assert_eq!(pause_ids(&out), ["pause-body-TASK-X-010-1"], "{out}");
    assert_eq!(evidence(&out, 0)["reason"], "no_progress", "{out}");
}

// --- resume -----------------------------------------------------------------

#[test]
fn a_resumed_pause_continues_the_loop_and_can_accept() {
    let out = body(
        r#"{ resumed: ["pause-body-TASK-X-010-1"], findings: (n) => n <= 5 ? ["defect alpha"] : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(pause_ids(&out), ["pause-body-TASK-X-010-1"], "{out}");
    assert_eq!(out["calls"], 6, "{out}");
}

#[test]
fn a_resume_grants_one_fresh_window_not_an_immediate_re_pause() {
    let out = body(r#"{ resumed: ["pause-body-TASK-X-010-1"], findings: () => ["defect alpha"] }"#);
    assert_paused(&out);
    assert_eq!(
        pause_ids(&out),
        ["pause-body-TASK-X-010-1", "pause-body-TASK-X-010-2"],
        "{out}"
    );
    assert_eq!(
        out["calls"], 7,
        "three new attempts after the resume: {out}"
    );
    let second = evidence(&out, 1);
    assert_eq!(second["ordinal"], 2, "{second}");
    assert_eq!(second["author_calls"], 7, "{second}");
}

// --- operational attempts follow the same rule ------------------------------

#[test]
fn consecutive_operational_failures_pause_instead_of_failing() {
    let out = body(
        r#"{ answer: () => ({ status: "failed", summary: "agent transport failed: stream ended" }) }"#,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 3, "{out}");
    let evidence = evidence(&out, 0);
    assert_eq!(evidence["reason"], "operational_no_progress", "{evidence}");
    assert!(
        evidence["last_findings"][0]
            .as_str()
            .is_some_and(|text| text.contains("agent transport failed")),
        "{evidence}"
    );
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
fn an_operational_stall_after_a_resume_gets_a_fresh_window() {
    let out = body(
        r#"{ resumed: ["pause-body-TASK-X-010-1"], answer: () => ({ status: "failed", summary: "down" }) }"#,
    );
    assert_paused(&out);
    assert_eq!(pause_ids(&out).len(), 2, "{out}");
    assert_eq!(out["calls"], 6, "{out}");
}

#[test]
fn an_acceptance_entry_that_never_parses_pauses_the_run() {
    // Every reply is prose: its new malformed-reply class is progress once,
    // then three repeated classes without progress pause the run.
    let out = run(
        "enforce",
        r#"{ answer: () => ({ status: "accepted", stopReason: "end_turn", content: "I could not produce an entry." }) }"#,
        "workflow(w)",
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
    assert_eq!(pause_ids(&out), ["pause-acceptance-1"], "{out}");
    assert_eq!(
        evidence(&out, 0)["reason"],
        "no_progress",
        "the provider answered every reply: not an outage"
    );
}

#[path = "workflow_decompose_progress_rule_tests.rs"]
mod rule;

#[path = "workflow_decompose_progress_bound_tests.rs"]
mod bound;

#[path = "workflow_decompose_progress_stage_tests.rs"]
mod stage;
