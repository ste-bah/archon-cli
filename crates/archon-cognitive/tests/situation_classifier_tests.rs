use archon_cognitive::{ClassifyInput, CognitiveSurface, SituationClassifier, SituationKind};

fn classify(text: &str) -> archon_cognitive::Situation {
    SituationClassifier.classify(ClassifyInput {
        user_text: text,
        session_id: "test-session",
        turn_number: 1,
        surface: CognitiveSurface::Tui,
    })
}

#[test]
fn classifies_all_situation_kinds() {
    let cases = [
        ("hello", SituationKind::Greeting),
        (
            "delete all production database rows",
            SituationKind::HighRisk,
        ),
        ("commit and push these changes", SituationKind::GitMutation),
        (
            "check why the github actions build failed",
            SituationKind::CiDebug,
        ),
        (
            "resume the failed research pipeline",
            SituationKind::PipelineControl,
        ),
        (
            "run eval-jepa for the world model",
            SituationKind::WorldModelTask,
        ),
        ("fix the parser and add tests", SituationKind::CodeChange),
        ("research this with citations", SituationKind::Research),
        (
            "what is Elliott Wave theory?",
            SituationKind::SimpleQuestion,
        ),
        ("maybe around the thing later", SituationKind::Ambiguous),
    ];

    for (text, expected) in cases {
        let situation = classify(text);
        assert_eq!(situation.kind, expected, "{text}");
        assert!(!situation.user_text_hash.is_empty());
        assert_ne!(situation.user_text_hash, text);
    }
}

#[test]
fn greeting_with_substantive_request_is_not_greeting() {
    let situation = classify("ok fix the failing tui test");
    assert_eq!(situation.kind, SituationKind::CodeChange);
}

#[test]
fn short_question_is_simple_question() {
    let situation = classify("how does memory work?");
    assert_eq!(situation.kind, SituationKind::SimpleQuestion);
    assert!(situation.confidence_score >= 0.55);
}
