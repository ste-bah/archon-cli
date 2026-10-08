//! Issue 261: only a higher tier or a smaller distinct defect count is progress.
//! Rewording and swapping defects share the three-attempt no-progress window.

use super::{BODY, assert_paused, body, evidence, pause_ids, progress_flags, run};

/// A provider answer that ends a loop the script never pauses: without it a
/// regression would spin until the test harness timed out.
const SPIN_GUARD: &str =
    r##"if (n > 1500) throw new Error("spin: no pause after 1500 provider calls");"##;

#[test]
fn a_judge_rewording_one_defect_pauses_when_no_attempt_sets_a_new_best() {
    let out = body(&format!(
        r##"{{ answer: (n) => {{ {SPIN_GUARD} return {{ status: "accepted", stopReason: "end_turn", content: "# body " + n }}; }},
          findings: (n) => ["the parser lacks validation (observation " + n + ")"] }}"##
    ));
    assert_paused(&out);
    assert_eq!(
        out["calls"], 4,
        "a baseline, then three attempts without progress: {out}"
    );
    assert_eq!(pause_ids(&out), ["pause-body-TASK-X-010-1"], "{out}");
    let evidence = evidence(&out, 0);
    assert_eq!(evidence["reason"], "no_progress", "{evidence}");
    assert_eq!(progress_flags(evidence), [true, false, false, false]);
}

#[test]
fn a_resume_past_a_no_progress_pause_gets_one_fresh_window() {
    let out = run(
        "enforce",
        &format!(
            r##"{{ resumed: ["pause-body-TASK-X-010-1"],
              answer: (n) => {{ {SPIN_GUARD} return {{ status: "accepted", stopReason: "end_turn", content: "# body " + n }}; }},
              findings: (n) => ["the parser lacks validation (observation " + n + ")"] }}"##
        ),
        BODY,
    );
    assert_paused(&out);
    assert_eq!(
        out["calls"], 7,
        "four calls, then a fresh window of three: {out}"
    );
    assert_eq!(
        pause_ids(&out),
        ["pause-body-TASK-X-010-1", "pause-body-TASK-X-010-2"],
        "{out}"
    );
}

#[test]
fn a_subject_clearing_distinct_defects_is_not_stopped_by_the_short_window() {
    let out = body(
        r##"{ findings: (n) => Array.from({length: Math.max(41 - n, 0)}, (_, i) => "candidate artifact was refused: defect " + i) }"##,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 41, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn set_gate_rounds_whose_judge_rewords_one_defect_pause_without_a_new_best() {
    let out = run(
        "enforce",
        &format!(
            r##"{{ answer: (n, id) => {{ {SPIN_GUARD} return {{ status: "accepted", stopReason: "end_turn", content: id.startsWith("acceptance-author-") ? JSON.stringify({{ id: "AC-X-001" }}) : "# body " + n }}; }},
              findings: (n, capability) => {{
                if (capability !== "task-set-lint") return [];
                const round = globalThis.lintRound = (globalThis.lintRound || 0) + 1;
                return [{{ text: "task TASK-X-010: obligation is unclaimed (pass " + round + ")", subject: "TASK-X-010", source_path: "/p/tasks/TASK-X-010.md", remediation_scope: "body" }}];
              }} }}"##
        ),
        "workflow(w)",
    );
    assert_paused(&out);
    assert_eq!(pause_ids(&out), ["pause-set-gates-1"], "{out}");
    let evidence = evidence(&out, 0);
    assert_eq!(evidence["reason"], "no_progress", "{evidence}");
    assert_eq!(evidence["rounds"], 4, "{evidence}");
}

/// The judge rejects both entries with the same findings every time. Each
/// repair round rewrites AC-X-001 with new content and then fails on
/// AC-X-002's prose, so the rewrite is the round's only retained work. A
/// rewrite the judge rejects again is not a new best: the loop must pause.
#[test]
fn an_acceptance_entry_the_judge_rejects_after_every_rewrite_pauses() {
    let out = run(
        "enforce",
        &format!(
            r##"{{
              args: {{ acceptanceCriteria: {{ "AC-X-001": "a", "AC-X-002": "b" }} }},
              answer: (n, id) => {{
                {SPIN_GUARD}
                const match = /^acceptance-author-(AC-X-00(\d))-(\d+)$/.exec(id);
                if (!match) return {{ status: "accepted", stopReason: "end_turn", content: "# body " + n }};
                const round = Math.floor((Number(match[3]) - 1) / 3);
                const prose = match[2] === "2" && round % 2 === 1;
                return {{ status: "accepted", stopReason: "end_turn", content: prose ? "prose" : JSON.stringify({{ id: match[1], version: n }}) }};
              }},
              findings: (n, capability) => capability === "freeze-acceptance"
                ? ["AC-X-001", "AC-X-002"].map((id) => ({{ text: "check '" + id + "': floor is not falsifiable", subject: id, remediation_scope: "candidate_artifact" }}))
                : [],
            }}"##
        ),
        "workflow(w)",
    );
    assert_paused(&out);
    assert_eq!(pause_ids(&out), ["pause-acceptance-1"], "{out}");
    let evidence = evidence(&out, 0);
    assert_eq!(evidence["reason"], "no_progress", "{evidence}");
}

/// The round-8 measure and observe-fallback suite runs under cargo.
#[test]
fn round8_regression_suite_passes() {
    let output = std::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/command/workflow_decompose_round8_test.cjs"
        ))
        .output()
        .expect("node must be available");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Issue 288: author prompts grow by a bounded amount per completed entry and
/// per attempt (the suite prints the measured bytes), and an inactivity cut in
/// a round that kept new work does not consume the no-progress window.
#[test]
fn author_prompts_stay_bounded_per_entry_and_attempt() {
    let output = std::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/command/workflow_decompose_prompt_size_test.cjs"
        ))
        .output()
        .expect("node must be available");
    println!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The round-3 regression suite runs under cargo like the other node suites.
#[test]
fn round3_regression_suite_passes() {
    let output = std::process::Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/command/workflow_decompose_round3_test.cjs"
        ))
        .output()
        .expect("node must be available");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Runs one node suite of the fixed script; it fails on any failing case.
fn node_suite_passes(file: &str) {
    let path = format!("{}/src/command/{file}", env!("CARGO_MANIFEST_DIR"));
    let output = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node must be available");
    assert!(
        output.status.success(),
        "{path}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Issue 357 rounds 7-8: a pass is progress in every kind of round, each
/// entry's note stays its own, and an outage in a window pauses observe.
#[test]
fn round_credit_regression_suite_passes() {
    node_suite_passes("workflow_decompose_round_credit_test.cjs");
}

/// Issue 362: a freeze outage retries the freeze, never the author.
#[test]
fn freeze_outage_regression_suite_passes() {
    node_suite_passes("workflow_decompose_freeze_outage_test.cjs");
}

/// Issue 357: every refusal of a round is measured against its own best.
#[test]
fn multi_refusal_regression_suite_passes() {
    node_suite_passes("workflow_decompose_multi_refusal_test.cjs");
}

/// Issue 357: shape repairs use the freeze progress measure per episode.
#[test]
fn shape_progress_regression_suite_passes() {
    node_suite_passes("workflow_decompose_shape_progress_test.cjs");
}

/// Issue 366: the author is told where a check runs and that its paths are
/// relative, and the author step gives the entry validator the live roots.
#[test]
fn check_paths_regression_suite_passes() {
    node_suite_passes("workflow_decompose_check_paths_test.cjs");
}

// --- round 5: deterministic refusals and gate operational errors -------------

/// A refusal the host produced (not a model's text) naming the `n`-th task.
fn refusal(n: &str) -> String {
    format!(
        r##""candidate artifact was refused: task 'TASK-X-" + {n} + "' file_name 'bad' must be a direct TASK-*.md filename""##
    )
}

#[test]
fn a_complete_validator_cleared_one_defect_per_attempt_keeps_running() {
    // Guard: complete structured findings shrink from seventy to zero.
    // The real Rust validator's completeness has its own failing-before test.
    let out = body(&format!(
        r##"{{ answer: (n) => {{ {SPIN_GUARD} return {{ status: "accepted", stopReason: "end_turn", content: "# body " + n }}; }},
          findings: (n) => Array.from({{length: Math.max(71 - n, 0)}}, (_, i) => ({{ text: {}, subject: "skeleton", remediation_scope: "candidate_artifact",
            deterministic_defect: {{provenance:"host_validator", code:"invalid_filename", subject:"TASK-X-" + (i + n), location:"file_name"}} }})) }}"##,
        refusal("String(n).padStart(3, \"0\")")
    ));
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 71, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn a_deterministic_refusal_oscillating_between_two_defects_pauses() {
    let out = body(&format!(
        r##"{{ findings: (n) => [{{ text: {}, subject: "skeleton", remediation_scope: "candidate_artifact" }}] }}"##,
        refusal("(n % 2 ? \"001\" : \"002\")")
    ));
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "A, B, A, B: no smaller count: {out}");
    assert_eq!(evidence(&out, 0)["reason"], "no_progress");
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [true, false, false, false]
    );
}

#[test]
fn a_resumed_loop_still_knows_the_refusals_it_saw_before_the_pause() {
    // X four times pauses; after resume Y, X, Y make no smaller count.
    // A fresh window preserves the best even when the finding set changes.
    let out = run(
        "enforce",
        &format!(
            r##"{{ resumed: ["pause-body-TASK-X-010-1"],
              findings: (n) => [{{ text: {}, subject: "skeleton", remediation_scope: "candidate_artifact" }}] }}"##,
            refusal("(n > 4 && n % 2 ? \"002\" : \"001\")")
        ),
        BODY,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 7, "{out}");
    assert_eq!(
        pause_ids(&out),
        ["pause-body-TASK-X-010-1", "pause-body-TASK-X-010-2"]
    );
}

#[test]
fn a_gate_operational_error_pauses_the_author_loop_instead_of_failing() {
    let out = body(
        r##"{ operational: (n, capability) => capability === "land-task-body" ? "judge response was truncated" : null }"##,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 3, "{out}");
    let evidence = evidence(&out, 0);
    assert_eq!(evidence["reason"], "operational_no_progress", "{evidence}");
    assert!(
        evidence["last_findings"][0]
            .as_str()
            .is_some_and(|text| text.contains("judge response was truncated")),
        "{evidence}"
    );
}

#[test]
fn a_set_gate_operational_error_pauses_the_rounds_instead_of_failing() {
    let out = run(
        "enforce",
        r##"{ operational: (n, capability) => capability === "task-set-lint" ? "lint preparation failed" : null }"##,
        "workflow(w)",
    );
    assert_paused(&out);
    assert_eq!(pause_ids(&out), ["pause-set-gates-1"], "{out}");
    assert_eq!(evidence(&out, 0)["reason"], "operational_no_progress");
}

#[test]
fn a_frozen_stage_verification_operational_error_pauses_instead_of_failing() {
    let out = run(
        "enforce",
        r##"{ args: { frozenChain: { acceptance: true } },
          operational: (n, capability) => capability === "verify-frozen-acceptance" ? "verification preparation failed" : null }"##,
        "workflow(w)",
    );
    assert_paused(&out);
    assert_eq!(
        pause_ids(&out),
        ["pause-verify-frozen-acceptance-1"],
        "{out}"
    );
    assert_eq!(evidence(&out, 0)["reason"], "operational_no_progress");
}
