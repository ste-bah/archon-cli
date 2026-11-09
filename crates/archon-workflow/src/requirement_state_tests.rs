use super::*;
use crate::breaker_verdict::Verdict;
fn entry() -> RequirementEntry {
    serde_json::from_value(serde_json::json!({"task_id":"task", "id":"id", "class":"candidate_done", "reason":null, "fallback":"undecided", "checks":{}, "deliverables":[], "text_item":false, "passing_check_without_deliverable":false})).unwrap()
}
fn pair(verdict: Verdict) -> PairVerdict {
    serde_json::from_value(serde_json::json!({"task_id":"task", "id":"id", "verdict":verdict,"killed_by":{},"patch_keys":{}})).unwrap()
}
fn ctx() -> DecisionContext<'static> {
    DecisionContext {
        inspection: false,
        disabled: false,
        refusal: None,
        text_evidence: false,
    }
}
#[test]
fn every_current_verdict_has_a_literal_state_and_reason() {
    use RequirementState::*;
    for (verdict, state, reason) in [
        (Verdict::DoneCandidate, Done, "breakage_kill"),
        (Verdict::WeakTest, Undecided, "weak_test"),
        (Verdict::Unmutable, Missing, "unmutable"),
        (
            Verdict::BodyGutUnavailable,
            Undecided,
            "body_gut_unavailable",
        ),
        (
            Verdict::NoViableBehaviourMutant,
            Undecided,
            "no_viable_behaviour_mutant",
        ),
        (Verdict::BreakerUnusable, Undecided, "breaker_unusable"),
        (Verdict::UndecidedRelevance, Undecided, "irrelevant_kill"),
        (
            Verdict::UndecidedEquivalence,
            Undecided,
            "weak_test_undecided",
        ),
        (
            Verdict::RequirementTextMissing,
            Undecided,
            "requirement_text_missing",
        ),
        (Verdict::BreakerUnavailable, Undecided, "author_unavailable"),
        (Verdict::JudgeUnavailable, Undecided, "author_unavailable"),
        (Verdict::NoPassingCheck, Undecided, "no_passing_check"),
        (
            Verdict::NoCountableDeliverable,
            Missing,
            "no_countable_deliverable",
        ),
        (Verdict::Unverified, Undecided, "control_unstable"),
        (Verdict::BudgetExhausted, Undecided, "budget_exhausted"),
        (Verdict::HostUnrun, Undecided, "host_unrun"),
        (Verdict::Refused, Undecided, "proof_refused:unspecified"),
    ] {
        assert_eq!(
            decide(&entry(), Some(&pair(verdict)), ctx()),
            RequirementDecision::new(state, reason),
            "{verdict:?}"
        );
    }
}
#[test]
fn first_match_order_and_all_preproof_reasons() {
    use RequirementClass::*;
    use RequirementState::*;
    let mut e = entry();
    e.class = NoCheck;
    assert_eq!(
        decide(
            &e,
            None,
            DecisionContext {
                inspection: true,
                ..ctx()
            }
        )
        .state,
        Inspection
    );
    assert_eq!(decide(&e, None, ctx()).reason, "no_check");
    e.class = RequirementClass::Missing;
    for reason in [
        "deliverables_absent_at_base",
        "deliverable_unverified_at_base",
        "all_checks_fail_on_base",
    ] {
        e.reason = Some(reason.into());
        assert_eq!(decide(&e, None, ctx()).reason, reason);
    }
    e.class = NoBaseResult;
    assert_eq!(decide(&e, None, ctx()).reason, "no_base_result");
    e.class = CandidateDone;
    e.fallback = Some(Fallback::Missing);
    assert_eq!(
        decide(&e, Some(&pair(Verdict::WeakTest)), ctx()),
        RequirementDecision::new(RequirementState::Missing, "base_failing_check")
    );
    e.text_item = true;
    assert_eq!(
        decide(
            &e,
            Some(&pair(Verdict::WeakTest)),
            DecisionContext {
                text_evidence: true,
                ..ctx()
            }
        )
        .reason,
        "base_failing_check"
    );
    for context in [
        DecisionContext {
            disabled: true,
            text_evidence: true,
            ..ctx()
        },
        DecisionContext {
            refusal: Some("classification_stale"),
            text_evidence: true,
            ..ctx()
        },
    ] {
        assert_eq!(decide(&e, None, context).state, RequirementState::Missing);
    }
    assert_eq!(
        decide(
            &e,
            None,
            DecisionContext {
                disabled: true,
                ..ctx()
            }
        )
        .state,
        RequirementState::Missing
    );
    e.fallback = Some(Fallback::Undecided);
    for code in ["classification_stale", "proof_site_unavailable"] {
        let d = decide(
            &e,
            None,
            DecisionContext {
                refusal: Some(code),
                ..ctx()
            },
        );
        assert_eq!(d.state, Undecided);
        assert!(d.reason.ends_with(code));
    }
    assert_eq!(
        decide(
            &e,
            None,
            DecisionContext {
                disabled: true,
                ..ctx()
            }
        )
        .reason,
        "proof_disabled"
    );
}
#[test]
fn inspection_eligibility_has_no_language_or_layout_assumptions() {
    for (text, constraint, expected) in [
        ("Use alpha/beta", true, false),
        ("Use `alpha`", true, false),
        ("Use alpha.xyz.", true, false),
        ("Supports version 1.2.", true, true),
        ("Keep the result stable e.g.", true, false),
        ("Keep the result stable", false, false),
        ("Keep the result stable", true, true),
        ("Use alpha\\beta", true, false),
    ] {
        assert_eq!(inspection_only(text, constraint), expected, "{text}");
    }
}
#[test]
fn task_status_protects_every_undecided_only_task() {
    let states = |values: &[RequirementState]| {
        values
            .iter()
            .enumerate()
            .map(|(i, s)| (i.to_string(), RequirementDecision::new(*s, "test")))
            .collect()
    };
    use RequirementState::*;
    for (values, status) in [
        (vec![], None),
        (vec![Done], Some("done")),
        (vec![Done, Undecided], Some("kept")),
        (vec![Undecided], Some("kept")),
        (vec![Done, Missing], None),
        (vec![Undecided, Inspection], None),
    ] {
        assert_eq!(task_status(&states(&values)).as_deref(), status);
    }
}
#[test]
fn legacy_classification_locks_reserialize_byte_identically() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/classification");
    for name in ["acceptance.lock", "skeleton.lock"] {
        let bytes = std::fs::read(root.join(name)).unwrap();
        let serialized = if name == "acceptance.lock" {
            serde_json::to_vec_pretty(
                &serde_json::from_slice::<crate::task_set_contract::AcceptanceLock>(&bytes)
                    .unwrap(),
            )
            .unwrap()
        } else {
            serde_json::to_vec_pretty(
                &serde_json::from_slice::<crate::task_skeleton::TaskSkeletonLock>(&bytes).unwrap(),
            )
            .unwrap()
        };
        assert_eq!(serialized, bytes, "{name}");
    }
}
#[test]
fn proof_payload_cap_keeps_digests_and_discards_largest_first() {
    let proof = |n| DoneProof {
        id: "id".into(),
        kind: DoneKind::Breakage,
        check_id: "check".into(),
        check_digest: "digest".into(),
        base_commit: "base".into(),
        mutant_id: Some("m".into()),
        body_gut_mutant_id: Some("g".into()),
        patch_digest: Some("p".into()),
        patch: Some("x".repeat(n)),
        output_tail: None,
        output_digest: Some("o".into()),
        literals: vec![],
        evidence_truncated: false,
    };
    let mut proofs = vec![proof(10), proof(DONE_EVIDENCE_BYTES_MAX)];
    bound_evidence(&mut proofs);
    assert!(proofs[0].patch.is_some());
    assert!(proofs[1].patch.is_none());
    assert!(proofs[1].evidence_truncated);
    assert_eq!(proofs[1].patch_digest.as_deref(), Some("p"));
    assert_eq!(proofs[1].output_digest.as_deref(), Some("o"));
}

#[test]
fn base_failing_check_wins_over_unavailable_proof_and_schedules_work() {
    let mut e = entry();
    e.fallback = Some(Fallback::Missing);
    for v in [
        Verdict::BodyGutUnavailable,
        Verdict::NoViableBehaviourMutant,
        Verdict::UndecidedEquivalence,
    ] {
        let decision = decide(&e, Some(&pair(v)), ctx());
        assert_eq!(decision.state, RequirementState::Missing);
        assert_eq!(decision.reason, "base_failing_check");
        assert_eq!(task_status(&[(e.id.clone(), decision)].into()), None);
    }
}

#[test]
fn validation_rejects_inconsistent_host_records_and_stale_checks_are_cleared() {
    use crate::task_skeleton::{TaskSkeleton, refresh_done_proofs, skeleton_defects};
    let mut s:TaskSkeleton=serde_json::from_value(serde_json::json!({"schema_version":1,"acceptance_digest":"anchor","tasks":[{"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["id"]}]})).unwrap();
    assert!(skeleton_defects(&s, "anchor").is_empty());
    s.tasks[0].requirement_states.insert(
        "other".into(),
        RequirementDecision::new(RequirementState::Done, "breakage_kill"),
    );
    let codes: Vec<_> = skeleton_defects(&s, "anchor")
        .into_iter()
        .map(|d| d.identity.code)
        .collect();
    assert!(codes.contains(&"requirement_states_mismatch".into()));
    assert!(codes.contains(&"done_status_mismatch".into()));
    s.tasks[0].requirement_states.clear();
    s.tasks[0].requirement_states.insert(
        "id".into(),
        RequirementDecision::new(RequirementState::Done, "breakage_kill"),
    );
    s.tasks[0].status = Some("done".into());
    assert!(
        skeleton_defects(&s, "anchor")
            .iter()
            .any(|d| d.identity.code == "done_proof_mismatch")
    );
    let proof:DoneProof=serde_json::from_value(serde_json::json!({"id":"id","kind":"breakage","check_id":"missing-check","check_digest":"digest","base_commit":"base","mutant_id":"m","body_gut_mutant_id":"g","patch_digest":"p"})).unwrap();
    s.tasks[0].proven_done.push(proof);
    let c:crate::task_set_contract::AcceptanceContract=serde_json::from_value(serde_json::json!({"schema_version":1,"prd":{"path":"input","digest":"anchor"},"gap_policy":{},"acceptance":[],"supplementary":[]})).unwrap();
    refresh_done_proofs(&mut s, &c);
    assert_eq!(s.tasks[0].requirement_states["id"].reason, "proof_stale");
    assert_eq!(s.tasks[0].status.as_deref(), Some("kept"));
    assert!(skeleton_defects(&s, "anchor").is_empty());
    assert!(s.tasks[0].proven_done.is_empty());
}

#[test]
fn unavailable_cause_preserves_transience_and_unusable_authorship() {
    use crate::breaker_verdict::{AttemptOutcome, AttemptRecord, UnavailableCause};
    let mut p = pair(Verdict::BreakerUnavailable);
    p.attempts.push(AttemptRecord {
        attempt: 1,
        reply_key: "reply".into(),
        outcome: AttemptOutcome::Unavailable {
            cause: UnavailableCause::Unusable,
            reason: "unusable reply".into(),
        },
        refusals: vec![],
    });
    assert_eq!(decide(&entry(), Some(&p), ctx()).reason, "author_unusable");
    p.attempts.push(AttemptRecord {
        attempt: 2,
        reply_key: "retry".into(),
        outcome: AttemptOutcome::Unavailable {
            cause: UnavailableCause::Transient,
            reason: "provider unavailable".into(),
        },
        refusals: vec![],
    });
    assert_eq!(
        decide(&entry(), Some(&p), ctx()).reason,
        "author_unavailable"
    );
}

#[test]
fn legacy_positional_fields_keep_their_slots_and_candidate_host_echoes_are_discarded() {
    let raw=br#"{"schema_version":1,"acceptance_digest":"anchor","tasks":[["TASK-X-001","TASK-X-001.md",[],[],[],[],[],null]]}"#;
    let published: crate::task_skeleton::TaskSkeleton = serde_json::from_slice(raw).unwrap();
    let candidate = crate::task_skeleton::read_candidate(raw).unwrap();
    assert_eq!(candidate, published);
    let echoed=br#"{"schema_version":1,"acceptance_digest":"anchor","tasks":[["TASK-X-001","TASK-X-001.md",[],[],[],[],[],null,"mangled",[42]]]}"#;
    assert_eq!(
        crate::task_skeleton::read_candidate(echoed).unwrap(),
        candidate
    );
    assert!(serde_json::from_slice::<crate::task_skeleton::TaskSkeleton>(echoed).is_err());
}

#[test]
fn a_recorded_base_failure_beats_another_checks_unknown_result() {
    let mut e = entry();
    e.fallback = None;
    e.checks = [
        (
            "failed".into(),
            Some(crate::baseline_results::BaselineOutcome::Failed),
        ),
        ("unknown".into(), None),
    ]
    .into();
    for class in [
        RequirementClass::NoBaseResult,
        RequirementClass::CandidateDone,
    ] {
        e.class = class;
        let decision = decide(&e, Some(&pair(Verdict::BodyGutUnavailable)), ctx());
        assert_eq!(
            decision,
            RequirementDecision::new(RequirementState::Missing, "base_failing_check")
        );
        assert_eq!(task_status(&[(e.id.clone(), decision)].into()), None);
    }
}

#[test]
fn text_evidence_cannot_override_survivor_verdicts() {
    let mut e = entry();
    e.text_item = true;
    for verdict in [
        Verdict::WeakTest,
        Verdict::UndecidedEquivalence,
        Verdict::BudgetExhausted,
        Verdict::BodyGutUnavailable,
    ] {
        let d = decide(
            &e,
            Some(&pair(verdict)),
            DecisionContext {
                text_evidence: true,
                ..ctx()
            },
        );
        assert_eq!(d.state, RequirementState::Undecided, "{verdict:?}");
        assert_ne!(d.reason, "text_evidence");
    }
}
