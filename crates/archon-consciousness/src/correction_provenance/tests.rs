use super::*;

fn correction(context: &str) -> Correction {
    Correction {
        id: "corr-1".into(),
        correction_type: CorrectionType::ActedWithoutPermission,
        content: "you did that without asking".into(),
        context: context.into(),
        severity: 5.0,
        rule_id: Some("rule:correction:acted-without-permission:v2".into()),
        timestamp: Utc::now(),
    }
}

#[test]
fn the_writer_and_the_parser_agree() {
    assert_eq!(
        parse_correction_context(&immediate_turn_context(7)),
        (Some(7), CorrectionPass::Immediate)
    );
    assert_eq!(
        parse_correction_context(&semantic_pass_context(7)),
        (Some(7), CorrectionPass::SemanticExtraction)
    );
}

/// The format that was already in the graph before this module existed.
#[test]
fn the_existing_context_format_still_parses() {
    assert_eq!(
        parse_correction_context("turn:12"),
        (Some(12), CorrectionPass::Immediate)
    );
    assert_eq!(
        parse_correction_context("turn:12 (semantic pass)"),
        (Some(12), CorrectionPass::SemanticExtraction)
    );
}

/// An unparseable context is unknown, not turn zero and not "the latest turn".
///
/// The fallback this test forbids is the whole hazard: a correction whose origin
/// is unknown, silently pointed at whichever actions happen to be in the window.
#[test]
fn an_unparseable_context_yields_no_turn_rather_than_a_default() {
    for context in ["", "session start", "turn:", "turn:abc", "turn 4", "12"] {
        assert_eq!(
            parse_correction_context(context),
            (None, CorrectionPass::Unrecognised),
            "context {context:?} must not resolve to a turn"
        );
    }
}

#[test]
fn provenance_of_an_unparseable_record_is_incomplete() {
    let provenance = CorrectionProvenance::from_record(&correction("session start"));

    assert_eq!(provenance.turn_number, None);
    assert_eq!(provenance.pass, CorrectionPass::Unrecognised);
    assert_eq!(
        provenance.incompleteness_code(),
        Some(PROVENANCE_UNPARSED_TURN)
    );
}

#[test]
fn turn_zero_is_rejected_because_turn_numbers_start_at_one() {
    let provenance = CorrectionProvenance::from_record(&correction("turn:0"));

    assert_eq!(provenance.incompleteness_code(), Some(PROVENANCE_ZERO_TURN));
}

#[test]
fn a_parsed_record_is_complete_and_carries_its_own_identity() {
    let provenance = CorrectionProvenance::from_record(&correction(&immediate_turn_context(4)));

    assert_eq!(provenance.incompleteness_code(), None);
    assert_eq!(provenance.turn_number, Some(4));
    assert_eq!(provenance.correction_id, "corr-1");
    assert_eq!(
        provenance.correction_type,
        CorrectionType::ActedWithoutPermission
    );
    assert_eq!(provenance.severity, 5.0);
}

/// Finding 41, stated as a test.
///
/// The derived rule id is a compile-time constant shared by every correction of
/// this type, so it must never appear as provenance. If someone later adds it to
/// `evidence_refs` for convenience, every lesson derived from any
/// `ActedWithoutPermission` correction starts citing the same source, and the
/// repeated-failure comparison the R2 gate rests on becomes meaningless.
#[test]
fn evidence_refs_never_cite_the_category_rule() {
    let record = correction(&immediate_turn_context(4));
    let provenance = CorrectionProvenance::from_record(&record);
    let refs = provenance.evidence_refs();

    assert_eq!(
        provenance.linked_rule_id.as_deref(),
        Some("rule:correction:acted-without-permission:v2"),
        "the record does link a category rule"
    );
    assert!(
        !refs
            .iter()
            .any(|reference| reference.contains("rule:correction")),
        "the category rule leaked into provenance refs: {refs:?}"
    );
    assert!(refs.contains(&"correction:corr-1".to_string()));
    assert!(refs.contains(&"turn:4".to_string()));
    assert!(refs.contains(&"correction_type:acted_without_permission".to_string()));
}

/// Two corrections of the same type from different turns must be
/// distinguishable by their provenance alone.
#[test]
fn provenance_discriminates_two_corrections_the_rule_id_cannot() {
    let mut first = correction(&immediate_turn_context(2));
    first.id = "corr-a".into();
    let mut second = correction(&immediate_turn_context(9));
    second.id = "corr-b".into();

    let first = CorrectionProvenance::from_record(&first);
    let second = CorrectionProvenance::from_record(&second);

    assert_eq!(first.linked_rule_id, second.linked_rule_id);
    assert_ne!(first.evidence_refs(), second.evidence_refs());
}
