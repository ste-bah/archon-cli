//! Issue 261, round 7: progress is measured per deterministic validation
//! stage -- (tier, first failing stage, distinct defects at that stage) --
//! and the judge's free-text findings take no part in it. A stage fix that
//! lets a later stage report more defects is progress; rewording, renaming
//! and trading defects are not.

use super::{BODY, assert_paused, body, evidence, pause_ids, progress_flags, run};

/// JS helpers shared by the scenarios: `det(code, subject, stage, text)` is a
/// host-validator finding; `judge(text)` is a model finding.
const HELPERS: &str = r##"
const det = (code, subject, stage, text) => ({ text, subject, remediation_scope: "candidate_artifact",
  deterministic_defect: { provenance: "host_validator", code, subject, location: "slot", stage } });
const judge = (text) => ({ text, subject: "TASK-X-010", remediation_scope: "candidate_artifact" });
const lint = (i) => det("invalid_verifier", "TASK-X-" + String(i).padStart(3, "0"), "contracts", "TASK-X-" + i + ": verifier is weak");
const named = (i) => det("invalid_filename", "TASK-X-" + String(i).padStart(3, "0"), "structure", "candidate artifact was refused: task " + i + " file_name bad");
"##;

fn staged(findings: &str) -> serde_json::Value {
    body(&format!(
        r##"(() => {{ {HELPERS} return {{
          answer: (n) => {{ if (n > 300) throw new Error("spin: no pause after 300 calls"); return {{ status: "accepted", stopReason: "end_turn", content: "# body " + n }}; }},
          findings: {findings} }}; }})()"##
    ))
}

#[test]
fn a_parse_fix_that_reveals_four_contract_defects_keeps_running() {
    // Reviewer probe A: 1 parse defect, then 4, 3, 2, 1, 0 contract defects.
    let out = staged(
        r##"(n) => n === 1 ? [det("unparseable_task_file", "task_file", "parse", "TASK-X-010.md: yaml parse error at line 3")]
          : Array.from({ length: Math.max(6 - n, 0) }, (_, i) => lint(i + 1))"##,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 6, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn bad_json_then_three_shape_errors_then_structural_defects_keeps_running() {
    // Reviewer probe B: a first-error serde shape check reports one field at
    // a time; reaching the structural stage is progress at any count.
    let out = staged(
        r##"(n) => n === 1 ? [det("invalid_json", "skeleton", "parse", "candidate artifact was refused: the reply is not a JSON document (x)")]
          : n <= 4 ? [det("invalid_candidate_shape", "skeleton", "shape", "candidate artifact was refused: the JSON document does not match the required shape (missing field f" + n + ")")]
          : Array.from({ length: Math.max(8 - n, 0) }, (_, i) => named(i + 1))"##,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 8, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}

#[test]
fn host_defects_falling_while_judge_findings_rise_keep_running() {
    // Reviewer probe C: 10 -> 0 host defects while the judge adds findings;
    // once the host defects are gone only the judge remains, which is no
    // progress, so the loop pauses three attempts later.
    let out = staged(
        r##"(n) => [...Array.from({ length: Math.max(11 - n, 0) }, (_, i) => lint(i + 1)),
          ...Array.from({ length: n }, (_, i) => judge("judge says gap " + i + " v" + n))]"##,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 14, "{out}");
    let flags = progress_flags(evidence(&out, 0));
    assert!(flags[..11].iter().all(|flag| *flag), "{flags:?}");
    assert_eq!(flags[11..], [false, false, false]);
}

#[test]
fn judge_findings_alone_neither_credit_nor_block_progress() {
    // The judge's count falls 5 -> 1 with no host defect: not measured.
    let out = staged(
        r##"(n) => Array.from({ length: Math.max(6 - n, 1) }, (_, i) => judge("gap " + i))"##,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
}

#[test]
fn renaming_the_same_structural_defect_pauses() {
    let out = staged(
        r##"(n) => [det("invalid_filename", "TASK-X-001", "structure", "candidate artifact was refused: file_name 'bad" + n + "'")]"##,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [true, false, false, false]
    );
}

#[test]
fn oscillating_between_two_defects_of_one_stage_pauses() {
    let out = staged(r##"(n) => [named(n % 2 ? 1 : 2)]"##);
    assert_paused(&out);
    assert_eq!(out["calls"], 4, "{out}");
}

#[test]
fn falling_back_to_an_earlier_stage_is_not_progress() {
    // Shape, structure, then shape and structure again: the best stays the
    // structural one, so the returns are not progress.
    let out = staged(
        r##"(n) => n % 2 ? [det("invalid_candidate_shape", "skeleton", "shape", "shape")] : [named(1), named(2)]"##,
    );
    assert_paused(&out);
    assert_eq!(out["calls"], 5, "{out}");
    assert_eq!(
        progress_flags(evidence(&out, 0)),
        [true, true, false, false, false]
    );
}

#[test]
fn pause_evidence_carries_no_retired_attempt_bound() {
    let out = staged(r##"(n) => [named(1)]"##);
    assert_paused(&out);
    let evidence = evidence(&out, 0);
    assert!(evidence.get("no_new_best_window").is_none(), "{evidence}");
    assert!(evidence.get("attempts_since_best").is_none(), "{evidence}");
    assert!(
        !evidence["recovery"]
            .as_str()
            .unwrap_or_default()
            .contains("new best"),
        "{evidence}"
    );
}

#[test]
fn observe_mode_falls_back_to_the_best_committed_artifact_on_a_stall() {
    let out = run(
        "observe",
        &format!(r##"(() => {{ {HELPERS} return {{ findings: () => [named(1)] }}; }})()"##),
        BODY,
    );
    assert_eq!(out["accepted"], true, "{out}");
    assert_eq!(out["calls"], 4, "{out}");
    assert!(pause_ids(&out).is_empty(), "{out}");
}
