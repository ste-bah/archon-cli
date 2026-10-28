//! Issue 261, round 2: the one progress rule every loop shares -- what counts
//! as a new finding, which attempts count, and what may stop a loop that is
//! still converging.

use super::{BODY, body, pause_ids, run};

// --- finding identity -------------------------------------------------------

#[test]
fn rewording_by_case_space_or_punctuation_advances_the_ladder() {
    let out = body(
        r#"{ findings: (n) => n <= 13 ? [["Validation is absent.", "validation is absent", "VALIDATION IS ABSENT!", "validation   is absent;"][n % 4]] : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn different_numbers_at_the_same_defect_count_advance_the_ladder() {
    // Different argument numbers do not lower the number of outstanding defects.
    let out =
        body(r#"{ findings: (n) => n <= 13 ? ["argument " + n + " lacks validation"] : [] }"#);
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn changing_the_subject_at_the_same_defect_count_advances_the_ladder() {
    let out = body(
        r#"{ findings: (n) => n <= 13 ? [{ text: "check is weak", subject: "AC-X-00" + n, remediation_scope: "body" }] : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
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
fn no_novelty_still_reaches_the_next_authored_rung() {
    // A distant improvement cannot justify 29 attempts at the same count.
    let out = body(
        r#"{ findings: (n) => n <= 13 ? Array.from({ length: 5 - Math.floor(n / 30) }, (_, i) => "defect " + n + "-" + i) : [] }"#,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

// --- one count of consecutive attempts without progress ---------------------

#[test]
fn alternating_transport_and_incomplete_replies_fill_only_the_host_window() {
    let out = body(
        r#"{ answer: (n) => n % 2 ? { status: "failed", summary: "transport" } : { status: "accepted", stopReason: "max_tokens", content: "cut" } }"#,
    );
    assert_eq!(out["calls"], 5, "{out}");
    assert_eq!(pause_ids(&out).len(), 1, "{out}");
    let evidence = &out["pauses"][0]["evidence"];
    assert_eq!(evidence["reason"], "operational_no_progress", "{out}");
    assert_eq!(evidence["answered_attempts"], 2, "{out}");
    let kinds: Vec<_> = evidence["progress_history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "operational",
            "incomplete",
            "operational",
            "incomplete",
            "operational"
        ]
    );
    assert!(
        evidence["recovery"].as_str().unwrap().contains("transport"),
        "{out}"
    );
    assert!(
        !evidence["recovery"].as_str().unwrap().contains("resume"),
        "{out}"
    );
    assert!(
        out["progress_events"].as_array().unwrap().is_empty(),
        "{out}"
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
fn set_gate_rounds_trading_one_defect_pause_and_resume_the_ladder() {
    let out = run(
        "enforce",
        r#"{ resumed: ['pause-set-gates-1'], findings: (n, capability) => {
          if (capability !== "task-set-lint") return [];
          const round = globalThis.lintRound = (globalThis.lintRound || 0) + 1;
          return round <= 12 ? [{ text: "task TASK-X-010: obligation " + String.fromCharCode(96 + round) + " is unclaimed", subject: "TASK-X-010", source_path: "/p/tasks/TASK-X-010.md", remediation_scope: "body" }] : [];
        } }"#,
        "workflow(w)",
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(pause_ids(&out), ["pause-set-gates-1"], "{out}");
}

#[test]
fn set_gate_rounds_that_repeat_one_defect_advance_author_rungs() {
    let out = run(
        "enforce",
        r#"{ findings: (n, capability) => capability === "task-set-lint" && n <= 12
          ? [{ text: "task TASK-X-010: obligation a is unclaimed", subject: "TASK-X-010", source_path: "/p/tasks/TASK-X-010.md", remediation_scope: "body" }]
          : [] }"#,
        "workflow(w)",
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn the_body_entry_point_still_drives_one_subject() {
    // Guards the shared driver: the body policy is reachable as before.
    let out = run("enforce", "{}", BODY);
    assert_eq!(out["accepted"], true, "{out}");
}
