use super::*;

/// Every phrasing the deterministic arm is required to recognise.
///
/// This is the population the R3 promotion gate measures recall over, and the
/// required value is exactly 1.0 -- not "high". Adding a phrasing to the table
/// without adding it here would let recall regress unobserved.
const EXPLICIT_CORPUS: &[(&str, CorrectionType)] = &[
    ("no, that is not the file", CorrectionType::FactualError),
    ("no you have the wrong crate", CorrectionType::FactualError),
    ("wrong file entirely", CorrectionType::FactualError),
    ("that's wrong", CorrectionType::FactualError),
    ("that is wrong, try again", CorrectionType::FactualError),
    (
        "I said use the other branch",
        CorrectionType::RepeatedInstruction,
    ),
    (
        "I already told you to run the tests",
        CorrectionType::RepeatedInstruction,
    ),
    (
        "I already asked for the summary",
        CorrectionType::RepeatedInstruction,
    ),
    (
        "as I mentioned, the port is 8080",
        CorrectionType::RepeatedInstruction,
    ),
    (
        "don't push without asking me",
        CorrectionType::DidForbiddenAction,
    ),
    ("do not edit that file", CorrectionType::DidForbiddenAction),
    (
        "stop rewriting the config",
        CorrectionType::DidForbiddenAction,
    ),
    (
        "you should never do that to main",
        CorrectionType::DidForbiddenAction,
    ),
    (
        "I didn't ask you to refactor",
        CorrectionType::ActedWithoutPermission,
    ),
    (
        "I did not ask for a rewrite",
        CorrectionType::ActedWithoutPermission,
    ),
    (
        "you did that without permission",
        CorrectionType::ActedWithoutPermission,
    ),
    (
        "you committed without asking",
        CorrectionType::ActedWithoutPermission,
    ),
    (
        "instead, use the cached path",
        CorrectionType::ApproachCorrection,
    ),
    (
        "you should have run the tests",
        CorrectionType::ApproachCorrection,
    ),
    (
        "there is a better approach here",
        CorrectionType::ApproachCorrection,
    ),
    (
        "use this instead: the bounded reader",
        CorrectionType::ApproachCorrection,
    ),
];

/// Phrasings that are real corrections but sit outside the table.
///
/// The classifier must ABSTAIN on these, not answer "not a correction": it has
/// no evidence either way with the provider arm off, and a confident negative
/// would enter the precision denominator as a fact.
const UNLISTED_PHRASINGS: &[&str] = &[
    "that's not what I meant",
    "you've misread the requirement",
    "hmm, that isn't quite it",
];

#[derive(Debug)]
struct StubProvider(Option<ProviderJudgement>);

impl AmbiguousCorrectionProvider for StubProvider {
    fn judge(&self, _user_input: &str) -> Option<ProviderJudgement> {
        self.0.clone()
    }
}

fn judgement(is_correction: bool, confidence: f32) -> ProviderJudgement {
    ProviderJudgement {
        is_correction,
        correction_type: is_correction.then_some(CorrectionType::ApproachCorrection),
        confidence,
    }
}

fn provider_arm(judgement: Option<ProviderJudgement>) -> CorrectionClassifier {
    CorrectionClassifier::new(CorrectionClassifierConfig {
        provider_enabled: true,
        ..CorrectionClassifierConfig::default()
    })
    .with_provider(Arc::new(StubProvider(judgement)))
}

/// The R3 gate's hard requirement: explicit deterministic cases recall = 1.0.
#[test]
fn explicit_phrase_recall_is_exactly_one() {
    let classifier = CorrectionClassifier::default();
    let mut recalled = 0usize;

    for (input, expected) in EXPLICIT_CORPUS {
        let classification = classifier.classify(input);
        assert!(
            classification.is_correction,
            "explicit phrasing must classify as a correction: {input:?}"
        );
        assert!(
            !classification.abstained(),
            "explicit phrasing must never abstain: {input:?}"
        );
        assert_eq!(
            classification.correction_type,
            Some(*expected),
            "taxonomy drifted for {input:?}"
        );
        assert_eq!(
            classification.rationale_code,
            format!("{RATIONALE_EXPLICIT_PHRASE_PREFIX}{}", expected.as_code())
        );
        assert!(classification.confidence >= DEFAULT_ABSTAIN_BELOW);
        recalled += 1;
    }

    assert_eq!(
        recalled as f64 / EXPLICIT_CORPUS.len() as f64,
        1.0,
        "explicit-phrase recall must be 1.0"
    );
}

#[test]
fn unlisted_phrasings_abstain_rather_than_denying() {
    let classifier = CorrectionClassifier::default();

    for input in UNLISTED_PHRASINGS {
        let classification = classifier.classify(input);
        assert!(
            classification.abstained(),
            "expected abstention for {input:?}"
        );
        assert_eq!(classification.rationale_code, RATIONALE_ABSTAIN_NO_SIGNAL);
        assert_eq!(classification.predicted_label(), "abstain");
        assert_eq!(classification.correction_type, None);
        assert_eq!(classification.confidence, 0.0);
    }
}

/// The provider arm must stay dark unless it is explicitly switched on, even
/// when a provider has been injected: a model call on every user turn is a cost
/// the fast path has not agreed to pay.
#[test]
fn provider_arm_is_off_by_default() {
    assert!(!CorrectionClassifierConfig::default().provider_enabled);

    let classifier = CorrectionClassifier::default()
        .with_provider(Arc::new(StubProvider(Some(judgement(true, 0.99)))));

    let classification = classifier.classify("that's not what I meant");
    assert!(
        classification.abstained(),
        "an injected provider must not run while the arm is disabled"
    );
    assert_eq!(classification.rationale_code, RATIONALE_ABSTAIN_NO_SIGNAL);
}

#[test]
fn enabled_provider_arm_answers_ambiguous_language() {
    let classification =
        provider_arm(Some(judgement(true, 0.9))).classify("that's not what I meant");

    assert!(classification.is_correction);
    assert!(!classification.abstained());
    assert_eq!(classification.predicted_label(), "correction");
    assert_eq!(
        classification.correction_type,
        Some(CorrectionType::ApproachCorrection)
    );
    assert_eq!(classification.rationale_code, RATIONALE_PROVIDER_JUDGED);
}

/// A confident negative is an answer, and must be distinguishable from an
/// abstention: the R3 gate needs >=100 adjudicated non-corrections.
#[test]
fn confident_negative_is_an_answer_not_an_abstention() {
    let classification = provider_arm(Some(judgement(false, 0.88))).classify("what does this do?");

    assert!(!classification.is_correction);
    assert!(!classification.abstained());
    assert_eq!(classification.predicted_label(), "not_correction");
    assert_eq!(classification.correction_type, None);
}

#[test]
fn provider_below_threshold_abstains() {
    let classifier = provider_arm(Some(judgement(true, 0.4)));

    let classification = classifier.classify("that's not what I meant");
    assert!(classification.abstained());
    assert_eq!(
        classification.rationale_code,
        RATIONALE_ABSTAIN_BELOW_THRESHOLD
    );
    assert_eq!(classification.confidence, 0.4);
    assert_eq!(
        classification.correction_type, None,
        "an abstention must not carry a taxonomy a caller could act on"
    );
}

#[test]
fn enabled_provider_that_declines_abstains() {
    let classification = provider_arm(None).classify("that's not what I meant");

    assert!(classification.abstained());
    assert_eq!(
        classification.rationale_code,
        RATIONALE_ABSTAIN_PROVIDER_UNAVAILABLE
    );
}

/// Enabled with nothing injected is a wiring mistake, not a licence to guess.
#[test]
fn enabled_arm_without_injected_provider_abstains() {
    let classifier = CorrectionClassifier::new(CorrectionClassifierConfig {
        provider_enabled: true,
        ..CorrectionClassifierConfig::default()
    });

    let classification = classifier.classify("that's not what I meant");
    assert!(classification.abstained());
    assert_eq!(
        classification.rationale_code,
        RATIONALE_ABSTAIN_PROVIDER_UNAVAILABLE
    );
}

/// NaN and out-of-range confidences would poison every downstream mean and
/// threshold, so they are treated as no answer rather than clamped.
#[test]
fn non_finite_or_out_of_range_confidence_abstains() {
    for bad in [f32::NAN, f32::INFINITY, -0.5, 1.5] {
        let classification = provider_arm(Some(judgement(true, bad))).classify("ambiguous");
        assert!(
            classification.abstained(),
            "confidence {bad} must produce an abstention"
        );
        assert_eq!(
            classification.rationale_code,
            RATIONALE_ABSTAIN_PROVIDER_UNAVAILABLE
        );
    }
}

/// The deterministic arm is consulted first and is never overridden, so
/// enabling the provider arm later cannot regress explicit-case recall.
#[test]
fn explicit_phrases_are_not_second_guessed_by_the_provider() {
    let classifier = provider_arm(Some(judgement(false, 0.99)));

    let classification = classifier.classify("no, that is not the file");
    assert!(classification.is_correction);
    assert_eq!(
        classification.correction_type,
        Some(CorrectionType::FactualError)
    );
}

#[test]
fn empty_input_abstains() {
    let classification = CorrectionClassifier::default().classify("");
    assert!(classification.abstained());
}

#[test]
fn version_is_recorded_on_the_configuration() {
    assert_eq!(
        CorrectionClassifier::default().version(),
        CORRECTION_CLASSIFIER_VERSION
    );
}
