//! Remediation budget arithmetic: what buys another attempt, what refunds a
//! burned one, and what a script may and may not do to the bound.

/// Runs the REAL prelude JS, not a Rust reimplementation of it.
///
/// Every other prelude guard in this file asserts on source text, which
/// catches drift but cannot catch wrong behaviour. The budget is arithmetic
/// over gap sets, so it is worth executing: a source assertion would have
/// passed on a rule that funded exactly the wrong task.
fn run_budget_js(driver: &str) -> String {
    let prelude = super::super::V3_PRIMITIVES_JS;
    let start = prelude
        .find("  const remediationBudget = (opts = {}) => {")
        .expect("remediationBudget must exist");
    let end = start
        + prelude[start..]
            .find("\n  };\n")
            .expect("remediationBudget end")
        + 5;
    let script = format!("{}\n{driver}\n", &prelude[start..end]);
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("budget.mjs");
    std::fs::write(&path, script).expect("write driver");
    let out = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node must be available; the tests already shell out to zsh and python3");
    assert!(
        out.status.success(),
        "driver failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn envelope(gap_ids: &[&str]) -> String {
    let gaps = gap_ids
        .iter()
        .map(|id| format!(r#"{{"id":"{id}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"result":{{"residual_gaps":[{gaps}]}}}}"#)
}

/// The observed TDL-040 sequence: 5 gaps, one closed, then flat.
/// A raw-count rule keeps funding it; this must stop on the plateau.
#[test]
fn budget_extends_while_the_original_diagnosis_shrinks_and_stops_on_a_plateau() {
    let a1 = envelope(&[
        "registry-missing",
        "paging",
        "mcp-path",
        "tui-alias",
        "filter-mismatch",
    ]);
    let a2 = envelope(&["paging", "mcp-path", "tui-alias", "zero-match"]);
    let a3 = envelope(&["paging", "mcp-path", "tui-alias", "zero-test-noise"]);
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 3 }});
console.log([b.shouldContinue(1, {a1}), b.shouldContinue(2, {a2}), b.shouldContinue(3, {a3}), b.shouldContinue(4, {a3})].join(","));"#
    );
    // 1,2 are inside the base budget; 3 is the first flat attempt, which one
    // noisy verdict may be (round 2); 4 is the second in a row and stops.
    assert_eq!(run_budget_js(&driver), "true,true,true,false");
}

/// Churned gaps were never in the baseline, so they cannot buy budget.
/// This is what removes the need to classify substantive vs incidental.
#[test]
fn gaps_absent_from_the_baseline_cannot_earn_budget() {
    let a1 = envelope(&["real-one"]);
    let a2 = envelope(&["real-one", "churn-a"]);
    let a3 = envelope(&["real-one", "churn-b"]);
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 3 }});
b.shouldContinue(1, {a1}); b.shouldContinue(2, {a2});
console.log(b.shouldContinue(3, {a3}));"#
    );
    assert_eq!(run_budget_js(&driver), "false");
}

/// Branch-suffixed ids cannot match across attempts and would read as
/// perpetual churn, so they are excluded rather than silently normalised.
#[test]
fn branch_suffixed_gap_ids_are_excluded_from_the_baseline() {
    let a1 = envelope(&["invalid_write_branch_output_review-remediate-task-tdl-020-2-63-0"]);
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 3 }});
b.shouldContinue(1, {a1});
// Baseline is empty, so nothing can be "still closing": it must not extend
// past the base budget on the strength of an unmatchable id.
console.log(b.shouldContinue(3, {a1}));"#
    );
    assert_eq!(run_budget_js(&driver), "false");
}

/// The envelope the host produces when schema repair failed but the patch
/// was confirmed landed against the declared baseline.
fn schema_landed_envelope() -> String {
    r#"{"result":{"data":{"schema_repair_patch_landed":true},"residual_gaps":[]}}"#.to_string()
}

/// An attempt that died to schema repair WITH a landed patch produced work
/// and no verdict, so it must not be charged to the task. Attempt 3 is
/// refused on the flat budget; with the refund it is funded.
#[test]
fn a_schema_failure_with_a_landed_patch_buys_one_extra_attempt() {
    let none = envelope(&[]);
    let landed = schema_landed_envelope();
    let driver = format!(
        r#"const plain = remediationBudget({{ baseAttempts: 3 }});
const refunded = remediationBudget({{ baseAttempts: 3 }});
// Same call on both, except the refunded one also saw a landed-patch impl.
console.log([
  plain.shouldContinue(1, {none}),
  plain.shouldContinue(3, {none}),
  refunded.shouldContinue(1, {none}, {landed}),
  refunded.shouldContinue(3, {none}),
].join(","));"#
    );
    // plain: funded at 1, refused at 3. refunded: funded at 1 AND at 3.
    assert_eq!(run_budget_js(&driver), "true,false,true,true");
}

/// The bound is the entire safety argument. Schema repair already retries
/// under its own cap, so an unbounded exemption turns a burned attempt into
/// a hung task. An agent that lands a patch and emits garbage EVERY time
/// must still run out.
#[test]
fn the_schema_refund_is_bounded_to_once_per_task() {
    let none = envelope(&[]);
    let landed = schema_landed_envelope();
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 3 }});
// Four consecutive landed-patch schema failures. Only the first may pay out,
// so funding reaches base+1 = 4 and no further.
b.shouldContinue(1, {none}, {landed});
console.log([
  b.shouldContinue(3, {none}, {landed}),
  b.shouldContinue(4, {none}, {landed}),
  b.shouldContinue(5, {none}, {landed}),
].join(","));"#
    );
    // 3 < 4 funded; 4 and 5 are not — the refund did not compound.
    assert_eq!(run_budget_js(&driver), "true,false,false");
}

/// Without the marker nothing changes. Guards the refund against becoming a
/// blanket extra attempt for every task — which would quietly raise the
/// budget for the stuck tasks the progress rule exists to cut off.
#[test]
fn an_envelope_without_the_marker_earns_no_refund() {
    let none = envelope(&[]);
    // Shaped like the marker but false, and a look-alike key: neither pays.
    let decoys = r#"{"result":{"data":{"schema_repair_patch_landed":false,"schema_repair_patch_landed_maybe":true}}}"#;
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 3 }});
b.shouldContinue(1, {none}, {decoys});
console.log(b.shouldContinue(3, {none}, {decoys}));"#
    );
    assert_eq!(run_budget_js(&driver), "false");
}

/// A verifier that named nothing gives nothing to measure; fall back to the
/// flat budget rather than inventing progress from an empty set.
#[test]
fn an_empty_first_verdict_falls_back_to_the_flat_budget() {
    let none = envelope(&[]);
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 3 }});
console.log([b.shouldContinue(1, {none}), b.shouldContinue(3, {none})].join(","));"#
    );
    assert_eq!(run_budget_js(&driver), "true,false");
}

/// The exact call a generated script emitted, against an explicit
/// instruction not to. `hardCap: 3` makes `ceiling === funded`, so the
/// progress check never runs and a converging task is cut at three.
/// Enforced here because prompt text demonstrably did not prevent it.
#[test]
fn a_script_supplied_fixed_bound_cannot_disable_the_progress_check() {
    // Gap set shrinks every attempt: 3 -> 2 -> 1 of the baseline remain.
    let a1 = envelope(&["gap-a", "gap-b", "gap-c"]);
    let a2 = envelope(&["gap-a", "gap-b"]);
    let a3 = envelope(&["gap-a"]);
    let a4 = envelope(&[]);
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 3, hardCap: 3, maxSchemaRefunds: 0 }});
b.shouldContinue(1, {a1}); b.shouldContinue(2, {a2}); b.shouldContinue(3, {a3});
console.log(b.shouldContinue(4, {a4}));"#
    );
    // Attempt 4 must be funded: the diagnosis is still closing, and a
    // requested `hardCap: 3` is ignored (there is no total ceiling, Issue 298).
    assert_eq!(run_budget_js(&driver), "true");
}

/// No ceiling must not turn into "never stop". A plateau still ends the
/// budget: only attempts that are converging are funded past the window.
#[test]
fn an_ignored_hard_cap_still_stops_on_a_plateau() {
    let a1 = envelope(&["gap-a", "gap-b"]);
    let flat = envelope(&["gap-a", "gap-b"]);
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 3, hardCap: 3 }});
b.shouldContinue(1, {a1}); b.shouldContinue(2, {flat}); b.shouldContinue(3, {flat});
console.log(b.shouldContinue(4, {flat}));"#
    );
    assert_eq!(run_budget_js(&driver), "false");
}

/// `maxSchemaRefunds: 0` switched off a refund whose safety argument is the
/// once-per-task bound, not the ability to disable it. Floored at 1.
#[test]
fn a_script_cannot_switch_off_the_schema_refund() {
    let none = envelope(&[]);
    let landed = schema_landed_envelope();
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 2, hardCap: 2, maxSchemaRefunds: 0 }});
b.shouldContinue(1, {none}, {landed});
console.log(b.shouldContinue(2, {none}, {landed}));"#
    );
    // Without the floor this is false at the flat bound; the refund funds it.
    assert_eq!(run_budget_js(&driver), "true");
}

/// Widening is still the caller's to do — the floor is a floor, not a pin.
#[test]
fn a_script_may_still_widen_the_budget() {
    let a1 = envelope(&["gap-a", "gap-b", "gap-c"]);
    let a2 = envelope(&["gap-a", "gap-b"]);
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 8 }});
b.shouldContinue(1, {a1});
console.log(b.shouldContinue(7, {a2}));"#
    );
    // Inside a caller-widened base window, funded without needing progress.
    assert_eq!(run_budget_js(&driver), "true");
}

/// The envelope the host produces when a branch died to a provider or
/// transport failure before any verdict existed.
fn transport_failure_envelope() -> String {
    r#"{"result":{"data":{"transport_failure_no_verdict":true},"residual_gaps":[]}}"#.to_string()
}

/// A three-minute provider outage burned a real remediation round in a live
/// run. It says nothing about the work, so it must not be charged to the
/// task — the same reasoning that already forgives a schema failure whose
/// patch landed.
#[test]
fn a_transport_failure_buys_one_extra_attempt() {
    let none = envelope(&[]);
    let transport = transport_failure_envelope();
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 2, hardCap: 2 }});
b.shouldContinue(1, {none}, {transport});
console.log(b.shouldContinue(2, {none}, {transport}));"#
    );
    assert_eq!(run_budget_js(&driver), "true");
}

/// The two reasons draw from ONE pool. A task that hits both must not
/// collect two refunds — the bound's whole safety argument is that it is a
/// single forgiveness per task, however the attempt was burned.
#[test]
fn schema_and_transport_refunds_share_one_pool() {
    let none = envelope(&[]);
    let landed = schema_landed_envelope();
    let transport = transport_failure_envelope();
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 2, hardCap: 2 }});
b.shouldContinue(1, {none}, {landed});
b.shouldContinue(2, {none}, {transport});
console.log(b.shouldContinue(3, {none}, {transport}));"#
    );
    // Attempt 2 is funded by the single refund; attempt 3 must not be.
    assert_eq!(run_budget_js(&driver), "false");
}

/// An envelope carrying neither marker earns nothing, so an ordinary
/// rejection still costs the attempt it should.
#[test]
fn an_ordinary_failure_earns_no_transport_refund() {
    let none = envelope(&[]);
    let plain = r#"{"result":{"data":{"branch_error_from_runtime":true},"residual_gaps":[]}}"#;
    let driver = format!(
        r#"const b = remediationBudget({{ baseAttempts: 2, hardCap: 2 }});
b.shouldContinue(1, {none}, {plain});
console.log(b.shouldContinue(2, {none}, {plain}));"#
    );
    assert_eq!(run_budget_js(&driver), "false");
}

/// The host half of the same rule, executed end to end: the envelope the host
/// actually builds for a call that never started, handed to the real budget.
///
/// A call whose stage was never dispatched says nothing about the work, so it
/// must not spend an attempt — otherwise a store the host cannot read empties
/// every task's budget in milliseconds, which is exactly what happened. A call
/// that ran and failed still spends one, or a task that genuinely cannot be
/// finished would loop instead of being recorded blocked.
#[test]
fn a_call_that_never_started_does_not_spend_an_attempt_but_a_failed_one_does() {
    use crate::WorkflowError;
    use crate::v2::host_fault::v2_result_for_call_error;
    use crate::v2::script::{ScriptEnvelopeShape, result_view_json_shaped};

    let envelope = |error: WorkflowError| {
        result_view_json_shaped(
            &v2_result_for_call_error("stage-1", &error),
            ScriptEnvelopeShape::Deduped,
        )
        .expect("envelope")
    };
    let never_started = envelope(WorkflowError::StateCorrupt(
        "branches/call/foreign.json: missing field".to_string(),
    ));
    let ran_and_failed = envelope(WorkflowError::StageFailed(
        "the verifier rejected the change".to_string(),
    ));

    let driver = format!(
        r#"const a = remediationBudget({{ baseAttempts: 1 }});
const b = remediationBudget({{ baseAttempts: 1 }});
console.log([a.shouldContinue(1, {never_started}), b.shouldContinue(1, {ran_and_failed})].join(","));"#
    );

    assert_eq!(run_budget_js(&driver), "true,false");
}

/// Gap envelopes built inside the driver, so a long sequence stays readable:
/// `gaps(n, from)` names `gap-<from>` .. `gap-<from+n-1>`.
const GAPS_JS: &str = r#"const gaps = (n, from = 0) => ({ result: { residual_gaps: Array.from({ length: n }, (_, i) => ({ id: "gap-" + (from + i) })) } });"#;

/// Issue 298, the review's reproduction: one original gap closes per attempt.
/// The old total ceiling stopped it at attempt 12 with nine gaps open; a
/// task that keeps closing gaps must keep its budget until they are closed.
#[test]
fn a_task_closing_one_original_gap_per_attempt_is_never_cut_at_a_total() {
    let driver = format!(
        r#"{GAPS_JS}
const b = remediationBudget();
const answers = [];
for (let attempt = 1; attempt <= 21; attempt += 1) answers.push(b.shouldContinue(attempt, gaps(21 - attempt)));
console.log(answers.every(Boolean) + "," + b.shouldContinue(22, gaps(0)));"#
    );
    // Every closing attempt is funded; the verdicts naming no gap measure
    // nothing (round 2), and the second of them in a row stops.
    assert_eq!(run_budget_js(&driver), "true,false");
}

/// New ids may appear while the originals shrink. They never earn budget,
/// and they never cancel the progress the originals show.
#[test]
fn new_gaps_appearing_do_not_hide_originals_that_keep_closing() {
    let driver = format!(
        r#"{GAPS_JS}
const b = remediationBudget();
const answers = [];
for (let attempt = 1; attempt <= 15; attempt += 1) {{
  const env = gaps(16 - attempt);
  env.result.residual_gaps.push(...gaps(attempt, 100).result.residual_gaps);
  answers.push(b.shouldContinue(attempt, env));
}}
console.log(answers.every(Boolean));"#
    );
    assert_eq!(run_budget_js(&driver), "true");
}

/// A refund still adds one attempt to the window, and no longer interacts
/// with a total: a refunded task that keeps closing keeps going.
#[test]
fn a_refunded_task_that_keeps_closing_keeps_its_budget() {
    let landed = schema_landed_envelope();
    let driver = format!(
        r#"{GAPS_JS}
const b = remediationBudget({{ baseAttempts: 2 }});
const answers = [b.shouldContinue(1, gaps(16), {landed})];
for (let attempt = 2; attempt <= 16; attempt += 1) answers.push(b.shouldContinue(attempt, gaps(16 - attempt)));
console.log(answers.every(Boolean) + "," + b.shouldContinue(17, gaps(0)));"#
    );
    assert_eq!(run_budget_js(&driver), "true,false");
}

/// The first verifier named nothing, a later one named gaps: the first
/// named set is the baseline, so closing those is progress.
#[test]
fn a_late_first_diagnosis_becomes_the_baseline() {
    let driver = format!(
        r#"{GAPS_JS}
const b = remediationBudget({{ baseAttempts: 2 }});
const answers = [b.shouldContinue(1, gaps(0)), b.shouldContinue(2, gaps(14))];
for (let attempt = 3; attempt <= 15; attempt += 1) answers.push(b.shouldContinue(attempt, gaps(16 - attempt)));
answers.push(b.shouldContinue(16, gaps(1)));
console.log(answers.every(Boolean) + "," + b.shouldContinue(17, gaps(1)));"#
    );
    assert_eq!(run_budget_js(&driver), "true,false");
}

/// The stop names its evidence: which original gaps are still open, and
/// that the cause is no progress, not a count.
#[test]
fn a_plateau_stop_logs_the_open_original_gaps() {
    let driver = format!(
        r#"{GAPS_JS}
const lines = [];
globalThis.log = (message) => lines.push(message);
const b = remediationBudget({{ baseAttempts: 2 }});
b.shouldContinue(1, gaps(2));
b.shouldContinue(2, gaps(2));
b.shouldContinue(3, gaps(2));
console.log(lines.join("|"));"#
    );
    let logged = run_budget_js(&driver);
    assert!(logged.contains("no progress"), "{logged}");
    assert!(
        logged.contains("gap-0") && logged.contains("gap-1"),
        "{logged}"
    );
}

/// Round 2 (a): a verdict that names no gap measures nothing. Read as
/// "0 open", it made the next real verdict look like 0 -> 11 and stopped a
/// steady closer at attempt 9.
#[test]
fn a_gap_less_verdict_between_closing_attempts_is_not_a_regression() {
    let transport = transport_failure_envelope();
    for between in [r#"{"result":{"residual_gaps":[]}}"#.to_string(), transport] {
        let driver = format!(
            r#"{GAPS_JS}
const b = remediationBudget();
const answers = [];
for (let attempt = 1; attempt <= 12; attempt += 1) answers.push(b.shouldContinue(attempt, attempt === 8 ? {between} : gaps(20 - attempt)));
console.log(answers.join(","));"#
        );
        assert_eq!(
            run_budget_js(&driver),
            vec!["true"; 12].join(","),
            "{between}"
        );
    }
}

/// Round 2 (b): one noisy attempt does not end remediation; two
/// non-shrinking attempts in a row do (as ACCEPTANCE_STALL_LIMIT = 2).
#[test]
fn one_noisy_attempt_does_not_end_remediation_but_two_do() {
    let driver = format!(
        r#"{GAPS_JS}
const b = remediationBudget({{ baseAttempts: 2 }});
const seq = [10, 9, 9, 8, 8, 8];
console.log(seq.map((n, i) => b.shouldContinue(i + 1, gaps(n))).join(","));"#
    );
    assert_eq!(run_budget_js(&driver), "true,true,true,true,true,false");
}

/// Round 3: progress is a new BEST, not a drop from the last attempt.
/// Verdicts alternating 10 and 9 open gaps "closed" one every other attempt
/// against the previous verdict and ran 200+ attempts; against the best (9)
/// nothing after the first 9 is progress, so two attempts later it stops.
#[test]
fn verdicts_alternating_between_two_counts_stop_within_a_small_bound() {
    let driver = format!(
        r#"{GAPS_JS}
const b = remediationBudget({{ baseAttempts: 2 }});
let attempt = 1;
while (attempt <= 200 && b.shouldContinue(attempt, gaps(attempt % 2 === 1 ? 10 : 9))) attempt += 1;
console.log(attempt);"#
    );
    // 1: baseline 10; 2: 9 (new best); 3: 10 and 4: 9 are no new best.
    assert_eq!(run_budget_js(&driver), "4");
}

/// Round 3: the noisy closer from review keeps going: 20 -> 14, a verdict
/// naming no gap, then 13 and 12 are each a new best.
#[test]
fn a_noisy_closer_with_a_gap_less_verdict_keeps_going() {
    let driver = format!(
        r#"{GAPS_JS}
const b = remediationBudget({{ baseAttempts: 2 }});
const empty = {{ result: {{ residual_gaps: [] }} }};
console.log([gaps(20), gaps(14), empty, gaps(13), gaps(12)].map((env, i) => b.shouldContinue(i + 1, env)).join(","));"#
    );
    assert_eq!(run_budget_js(&driver), "true,true,true,true,true");
}
