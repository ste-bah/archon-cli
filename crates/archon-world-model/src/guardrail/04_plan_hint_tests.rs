#[cfg(test)]
mod plan_hint_tests {
    use super::*;

    #[test]
    fn plan_hint_classifies_only_complex_coding_work() {
        assert!(should_suggest_plan_mode(
            "Update src/a.rs and src/b.rs, then migrate config",
            RuntimeTaskClass::CodingChange,
        ));
        assert!(should_suggest_plan_mode(
            "Refactor this multi-part component lifecycle",
            RuntimeTaskClass::Refactor,
        ));
        assert!(should_suggest_plan_mode(
            "Debug this high-risk release failure",
            RuntimeTaskClass::Debugging,
        ));
        assert!(!should_suggest_plan_mode(
            "Fix typo",
            RuntimeTaskClass::CodingChange,
        ));
        assert!(!should_suggest_plan_mode(
            "Update src/a.rs",
            RuntimeTaskClass::CodingChange,
        ));
        assert!(!should_suggest_plan_mode(
            "What is plan mode?",
            RuntimeTaskClass::GeneralAnswer,
        ));
        assert!(!should_suggest_plan_mode(
            "Run cargo test and cargo build",
            RuntimeTaskClass::VerificationOnly,
        ));
    }

    #[test]
    fn plan_hint_recognizes_common_standalone_files_and_precise_markers() {
        assert!(should_suggest_plan_mode(
            "Update README.md and package.json",
            RuntimeTaskClass::CodingChange,
        ));
        assert!(!should_suggest_plan_mode(
            "Refactor a multi-particle component",
            RuntimeTaskClass::Refactor,
        ));
    }

    #[test]
    fn required_actions_use_the_canonical_completion_evidence_mapping() {
        assert_eq!(
            required_evidence_kind(GuardrailRequiredAction::RunTests),
            archon_completion::RequiredEvidenceKind::Tests,
        );
        assert_eq!(
            required_evidence_kind(GuardrailRequiredAction::RequireUserApproval),
            archon_completion::RequiredEvidenceKind::HumanApproval,
        );
    }
}
