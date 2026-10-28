//! Issue 261, round 2: the one progress rule every loop shares -- what counts
//! as a new finding, which attempts count, and what may stop a loop that is
//! still converging.

use super::{BODY, assert_paused, body, evidence, pause_ids, progress_flags, run};

// --- finding identity -------------------------------------------------------

#[test]
fn rewording_by_case_space_or_punctuation_is_the_same_finding() {
    let out = body(
        r#"{ findings: (n) => [["Validation is absent.", "validation is absent", "VALIDATION IS ABSENT!", "validation   is absent;"][n % 4]] }"#,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "a baseline and three restatements: {out}");
}

#[test]
fn different_numbers_at_the_same_defect_count_do_not_progress() {
    // Different argument numbers do not lower the number of outstanding defects.
    let out = body(r#"{ findings: (n) => n <= 5 ? ["argument " + n + " lacks validation"] : [] }"#);
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [true, false, false, false]
    );
}

#[test]
fn changing_the_subject_at_the_same_defect_count_does_not_progress() {
    let out = body(
        r#"{ findings: (n) => n <= 5 ? [{ text: "check is weak", subject: "AC-X-00" + n, remediation_scope: "body" }] : [] }"#,
    );
    assert_paused(&out);
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [true, false, false, false]
    );
}

// --- a converging loop is never stopped by a count --------------------------

#[test]
fn a_best_that_keeps_improving_is_never_stopped_by_a_count() {
    // 69 host defects, one fewer every attempt: 70 calls, past the old
    // bound of 64.
    let out = body(
        r#"{ findings: (n) => n <= 69 ? Array.from({ length: 70 - n }, (_, i) => "candidate artifact was refused: defect " + i) : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 70, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn novelty_cannot_extend_the_window_until_a_distant_new_best() {
    // A distant improvement cannot justify 29 attempts at the same count.
    let out = body(
        r#"{ findings: (n) => n <= 95 ? Array.from({ length: 5 - Math.floor(n / 30) }, (_, i) => "defect " + n + "-" + i) : [] }"#,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
}

// --- one count of consecutive attempts without progress ---------------------

#[test]
fn transport_failures_and_incomplete_replies_use_separate_windows() {
    let out = body(
        r#"{ answer: (n) => n % 2 ? { status: "failed", summary: "transport" } : { status: "accepted", stopReason: "max_tokens", content: "cut" } }"#,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 5, "{out}");
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [false, false, false, false, false]
    );
    assert_eq!(
        evidence(&out, 0)["reason"],
        "operational_no_progress",
        "three transport failures close the operational window independently"
    );
}

// --- acceptance entries -----------------------------------------------------

#[test]
fn an_acceptance_round_that_completes_an_entry_is_progress() {
    // Round r completes the r-th entry and fails the next on malformed
    // replies; the fourth round completes the last one and the contract
    // freezes clean.
    let out = run(
        "enforce",
        r#"{
          args: { acceptanceCriteria: { "AC-X-001": "a", "AC-X-002": "b", "AC-X-003": "c", "AC-X-004": "d" } },
          answer: (n, id) => {
            const match = /^acceptance-author-(AC-X-00(\d))-(\d+)$/.exec(id);
            if (!match) return { status: "accepted", stopReason: "end_turn", content: "body text" };
            const round = Math.floor((Number(match[3]) - 1) / 3);
            const ok = Number(match[2]) <= round;
            return { status: "accepted", stopReason: "end_turn", content: ok ? JSON.stringify({ id: match[1] }) : "prose" };
          },
        }"#,
        "workflow(w)",
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

// --- set gates follow the same rule -----------------------------------------

#[test]
fn set_gate_rounds_trading_one_defect_for_another_pause() {
    let out = run(
        "enforce",
        r#"{ findings: (n, capability) => {
          if (capability !== "task-set-lint") return [];
          const round = globalThis.lintRound = (globalThis.lintRound || 0) + 1;
          return round <= 6 ? [{ text: "task TASK-X-010: obligation " + String.fromCharCode(96 + round) + " is unclaimed", subject: "TASK-X-010", source_path: "/p/tasks/TASK-X-010.md", remediation_scope: "body" }] : [];
        } }"#,
        "workflow(w)",
    );
    assert_paused(&out);
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [true, false, false, false]
    );
}

#[test]
fn set_gate_rounds_that_repeat_one_defect_pause_after_the_stall_window() {
    let out = run(
        "enforce",
        r#"{ findings: (n, capability) => capability === "task-set-lint"
          ? [{ text: "task TASK-X-010: obligation a is unclaimed", subject: "TASK-X-010", source_path: "/p/tasks/TASK-X-010.md", remediation_scope: "body" }]
          : [] }"#,
        "workflow(w)",
    );
    assert_paused(&out);
    assert_eq!(pause_ids(&out), ["pause-set-gates-1"], "{out}");
    let evidence = evidence(&out, 0);
    assert_eq!(evidence["reason"], "no_progress", "{evidence}");
    assert_eq!(progress_flags(evidence), [true, false, false, false]);
}

#[test]
fn the_body_entry_point_still_drives_one_subject() {
    // Guards the shared driver: the body policy is reachable as before.
    let out = run("enforce", "{}", BODY);
    assert_eq!(out["accepted"], true, "{out}");
}
