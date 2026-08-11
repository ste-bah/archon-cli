use super::*;

fn close(left: f32, right: f32) -> bool {
    (left - right).abs() < 1e-4
}

/// The strongest case the engine has: the provider itself said the action
/// failed. That is deterministic evidence, and it outranks everything else.
#[test]
fn a_failed_tool_run_is_attributed_on_deterministic_evidence() {
    let mut reader = tool_run("tu-read", "SearchDocs", 4, 0);
    let mut failing = tool_run("tu-shell", "RunShell", 4, 1);
    failing.effect_class = ActionEffectClass::Mutate;
    failing.failed = true;
    reader.effect_class = ActionEffectClass::Read;

    let assessment = attribute(&input(
        correction("factual_error", "no, that broke the build", 5),
        vec![reader, failing],
        Vec::new(),
    ));

    assert!(assessment.attributed);
    assert_eq!(assessment.cohort, AttributionCohort::Accepted);
    assert_eq!(
        assessment.rationale_code,
        RATIONALE_ATTRIBUTED_DETERMINISTIC
    );
    let accepted = assessment.accepted_candidate().expect("an accepted link");
    assert_eq!(accepted.candidate.tool_use_id.as_deref(), Some("tu-shell"));
    assert_eq!(
        accepted.candidate.cause_action_class,
        CauseActionClass::ToolRun
    );
    assert!(
        close(assessment.confidence, 0.60),
        "expected 0.45 failure + 0.15 recency, got {}",
        assessment.confidence
    );
    assert!(accepted.evidence_codes().contains(&"deterministic_failure"));
    assert_eq!(
        accepted.candidate.action_attempt_id.as_deref(),
        Some("attribution-session:tu-shell:1"),
        "the accepted link must name an immutable action attempt"
    );
}

/// A permission complaint against the one thing that could have changed
/// anything. No failure to point at, so the acceptance rests on structural
/// evidence: the effect class matches the complaint, and there is nothing else
/// in the window to confuse it with.
#[test]
fn a_permission_correction_after_a_single_mutating_action_is_attributed() {
    let mut write = tool_run("tu-write", "WriteFile", 2, 0);
    write.effect_class = ActionEffectClass::Mutate;

    let assessment = attribute(&input(
        correction("acted_without_permission", "you did that without asking", 3),
        vec![write],
        Vec::new(),
    ));

    assert!(assessment.attributed);
    assert_eq!(assessment.rationale_code, RATIONALE_ATTRIBUTED_CORROBORATED);
    let codes = assessment
        .accepted_candidate()
        .expect("accepted")
        .evidence_codes();
    assert!(codes.contains(&"effect_class_affinity"));
    assert!(codes.contains(&"sole_eligible_action"));
}

/// `Correction -> Corrects -> Decision`: a complaint about the approach taken
/// attaches to the decision, not to whichever tool ran last.
#[test]
fn an_approach_correction_attaches_to_the_decision() {
    let assessment = attribute(&input(
        correction("approach_correction", "you should have run the tests", 3),
        Vec::new(),
        vec![decision("dec-1", 2)],
    ));

    assert!(assessment.attributed);
    let accepted = assessment.accepted_candidate().expect("accepted");
    assert_eq!(
        accepted.candidate.cause_action_class,
        CauseActionClass::Decision
    );
    assert_eq!(accepted.candidate.decision_id.as_deref(), Some("dec-1"));
    assert!(accepted.evidence_codes().contains(&"decision_scope"));
}

// ── refusals ─────────────────────────────────────────────────

/// The rule the roadmap states outright: never infer ownership from lexical
/// similarity alone.
///
/// Two mutating writes, neither failed, against a factual-error complaint that
/// happens to name a file one of them touched. Word overlap and recency are all
/// that is left, and they are not enough.
#[test]
fn word_overlap_and_recency_alone_do_not_carry_an_attribution() {
    let mut first = tool_run("tu-a", "WriteFile", 4, 0);
    first.effect_class = ActionEffectClass::Mutate;
    first.input_summary = "config.toml".into();
    let mut second = tool_run("tu-b", "WriteFile", 4, 1);
    second.effect_class = ActionEffectClass::Mutate;
    second.input_summary = "config.toml".into();

    let assessment = attribute(&input(
        correction("factual_error", "no, the config file is wrong", 5),
        vec![first, second],
        Vec::new(),
    ));

    assert!(!assessment.attributed);
    assert!(assessment.abstained());
    assert_eq!(assessment.rationale_code, RATIONALE_ABSTAIN_UNCORROBORATED);
    assert_eq!(
        assessment.ranked.len(),
        2,
        "the refusal must still record what was considered"
    );
    assert!(
        assessment.accepted_candidate().is_none(),
        "an abstention must expose no cause even though it ranked candidates"
    );
}

/// A near tie is the absence of an answer, not a weak one.
#[test]
fn two_equally_supported_failures_abstain_rather_than_pick_one() {
    let mut first = tool_run("tu-a", "RunShell", 4, 0);
    first.failed = true;
    let mut second = tool_run("tu-b", "RunShell", 4, 1);
    second.failed = true;

    let assessment = attribute(&input(
        correction("factual_error", "no, that is wrong", 5),
        vec![first, second],
        Vec::new(),
    ));

    assert_eq!(assessment.rationale_code, RATIONALE_ABSTAIN_AMBIGUOUS);
    assert_eq!(assessment.cohort, AttributionCohort::Abstained);
    assert!(
        assessment.confidence >= ACCEPT_THRESHOLD_FOR_TEST,
        "the top candidate cleared the confidence floor and was still refused"
    );
    assert!(assessment.accepted_candidate().is_none());
}

const ACCEPT_THRESHOLD_FOR_TEST: f32 = crate::attribution::scoring::ACCEPT_CONFIDENCE;

/// Structural support that is real but thin: the turn had a decision and
/// nothing else, and the complaint is not about the choice.
#[test]
fn a_lone_weakly_supported_decision_falls_below_the_floor() {
    let assessment = attribute(&input(
        correction("factual_error", "no, that is not the file", 3),
        Vec::new(),
        vec![decision("dec-1", 2)],
    ));

    assert_eq!(assessment.rationale_code, RATIONALE_ABSTAIN_BELOW_THRESHOLD);
    assert!(assessment.confidence < ACCEPT_THRESHOLD_FOR_TEST);
}

// ── eligibility ──────────────────────────────────────────────

/// The rollback trigger the R2 gate names first: a causal lesson linked to the
/// wrong session. An action from another session is not a weak candidate, it is
/// not a candidate.
#[test]
fn actions_from_another_session_are_never_candidates() {
    let mut foreign = tool_run("tu-foreign", "RunShell", 4, 0);
    foreign.session_id = "a-different-session".into();
    foreign.failed = true;
    let mut foreign_decision = decision("dec-foreign", 4);
    foreign_decision.session_id = "a-different-session".into();

    let assessment = attribute(&input(
        correction("factual_error", "no, that broke the build", 5),
        vec![foreign],
        vec![foreign_decision],
    ));

    assert!(assessment.unattributed());
    assert_eq!(
        assessment.rationale_code,
        UNATTRIBUTED_NO_ELIGIBLE_CANDIDATE
    );
    assert!(assessment.ranked.is_empty());
}

/// The correction arrived at the start of its own turn, so nothing recorded
/// against that turn or later can be what the user was correcting.
#[test]
fn actions_at_or_after_the_correction_turn_are_not_candidates() {
    let mut same_turn = tool_run("tu-same", "RunShell", 5, 0);
    same_turn.failed = true;
    let mut later = tool_run("tu-later", "RunShell", 6, 0);
    later.failed = true;

    let assessment = attribute(&input(
        correction("factual_error", "no, that broke the build", 5),
        vec![same_turn, later],
        Vec::new(),
    ));

    assert_eq!(
        assessment.rationale_code,
        UNATTRIBUTED_NO_ELIGIBLE_CANDIDATE
    );
}

#[test]
fn actions_older_than_the_lookback_window_are_dropped() {
    let correction_turn = 5;
    let too_old = correction_turn - ATTRIBUTION_LOOKBACK_TURNS - 1;
    let mut stale = tool_run("tu-stale", "RunShell", too_old, 0);
    stale.failed = true;

    let assessment = attribute(&input(
        correction("factual_error", "no, that broke the build", correction_turn),
        vec![stale],
        Vec::new(),
    ));

    assert_eq!(
        assessment.rationale_code,
        UNATTRIBUTED_NO_ELIGIBLE_CANDIDATE
    );
}

// ── provenance preconditions ─────────────────────────────────

/// A correction whose record does not place it in the conversation cannot be
/// attributed, however rich the window is.
#[test]
fn a_correction_without_a_turn_is_unattributed_not_pinned_to_the_newest_action() {
    let mut failing = tool_run("tu-shell", "RunShell", 4, 0);
    failing.failed = true;
    let mut unplaced = correction("factual_error", "no, that broke the build", 0);
    unplaced.turn_number = 0;

    let assessment = attribute(&input(unplaced, vec![failing], Vec::new()));

    assert_eq!(
        assessment.rationale_code,
        UNATTRIBUTED_PROVENANCE_INCOMPLETE
    );
    assert!(assessment.accepted_candidate().is_none());
}

#[test]
fn a_correction_without_a_session_is_unattributed() {
    let mut anonymous = correction("factual_error", "no, that broke the build", 5);
    anonymous.session_id = String::new();

    let assessment = attribute(&input(anonymous, Vec::new(), Vec::new()));

    assert_eq!(
        assessment.rationale_code,
        UNATTRIBUTED_PROVENANCE_INCOMPLETE
    );
}

#[test]
fn a_turn_that_did_nothing_is_an_empty_window() {
    let assessment = attribute(&input(
        correction("factual_error", "no, that is wrong", 5),
        Vec::new(),
        Vec::new(),
    ));

    assert_eq!(assessment.rationale_code, UNATTRIBUTED_EMPTY_WINDOW);
    assert_eq!(assessment.cohort, AttributionCohort::Unattributed);
}

// ── procedure identity ───────────────────────────────────────

/// Same input, same verdict. A measurement whose value depends on iteration
/// order cannot be pooled across a window.
#[test]
fn attribution_is_deterministic() {
    let mut first = tool_run("tu-a", "RunShell", 4, 0);
    first.failed = true;
    let mut second = tool_run("tu-b", "SearchDocs", 4, 1);
    second.effect_class = ActionEffectClass::Read;
    let subject = input(
        correction("factual_error", "no, that broke the build", 5),
        vec![first, second],
        vec![decision("dec-1", 4)],
    );

    assert_eq!(attribute(&subject), attribute(&subject));
}

/// `DecisionRecord` keeps the selected candidate by id only, so the action kind
/// has to come back out of the summary the same crate renders. If that format
/// changes, this fails rather than every decision candidate silently becoming
/// `unresolved_action_kind`.
#[test]
fn the_action_kind_is_recoverable_from_a_real_decision_summary() {
    use crate::attribution::input::{UNRESOLVED_ACTION_KIND, action_kind_from_decision_summary};

    let situation = crate::Situation {
        id: "sit-1".into(),
        session_id: SESSION.into(),
        turn_number: 2,
        user_text_hash: "hash".into(),
        kind: crate::SituationKind::SimpleQuestion,
        confidence_score: 0.9,
        confidence: crate::ClassifierConfidence::High,
        reason: "test".into(),
        surface: crate::CognitiveSurface::Cli,
        created_at: Utc.with_ymd_and_hms(2026, 8, 10, 11, 0, 0).unwrap(),
    };
    let candidate = crate::Candidate {
        id: "cand-1".into(),
        situation_id: "sit-1".into(),
        action_kind: crate::CandidateActionKind::RunTests,
        tool_name: None,
        expected_evidence: "test output".into(),
        expected_user_output: "result".into(),
        risk_class: crate::RiskLevel::Low,
        rollback_path: None,
        heuristic_score: 1.0,
        score_source: crate::ScoreSource::Heuristic,
        created_at: situation.created_at,
    };
    let record =
        crate::DecisionStore::decision_from_candidates(&situation, &[candidate]).expect("decision");

    assert_eq!(
        action_kind_from_decision_summary(&record.user_visible_summary),
        crate::CandidateActionKind::RunTests.as_str()
    );
    assert_eq!(
        action_kind_from_decision_summary("no arrow here"),
        UNRESOLVED_ACTION_KIND,
        "an unrecognised summary must not yield a fabricated kind"
    );
}

/// Shadow is compiled in, not configured.
#[test]
fn the_engine_runs_in_shadow_mode() {
    assert_eq!(ATTRIBUTION_MODE, AttributionMode::Shadow);
    assert_eq!(AttributionEngine.mode(), AttributionMode::Shadow);
    assert_eq!(AttributionEngine.version(), CAUSAL_ATTRIBUTION_VERSION);
}
