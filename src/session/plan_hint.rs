use archon_permissions::mode::PermissionMode;
use archon_world_model::RuntimeTaskClass;

pub(crate) fn plan_mode_hint_for_prompt(
    summary: &str,
    current_mode: PermissionMode,
) -> Option<&'static str> {
    let class = classify_interactive_coding_request(summary);
    archon_world_model::guardrail::plan_mode_hint(summary, class, current_mode)
}

fn classify_interactive_coding_request(summary: &str) -> RuntimeTaskClass {
    let words = prompt_words(summary);

    if is_informational_request(&words) {
        RuntimeTaskClass::GeneralAnswer
    } else if contains_any(&words, &["debug", "bug", "error", "failure", "fix"]) {
        RuntimeTaskClass::Debugging
    } else if contains_any(&words, &["refactor", "rename", "reorganize", "restructure"]) {
        RuntimeTaskClass::Refactor
    } else if contains_any(
        &words,
        &[
            "add",
            "change",
            "create",
            "implement",
            "modify",
            "update",
            "write",
            "migrate",
        ],
    ) {
        RuntimeTaskClass::CodingChange
    } else {
        RuntimeTaskClass::GeneralAnswer
    }
}

fn prompt_words(summary: &str) -> Vec<String> {
    summary
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn is_informational_request(words: &[String]) -> bool {
    if let Some(semantic_verb) = second_person_auxiliary_semantic_verb(words) {
        return is_informational_verb(semantic_verb);
    }

    words
        .first()
        .is_some_and(|word| is_informational_verb(word))
}

fn second_person_auxiliary_semantic_verb(words: &[String]) -> Option<&str> {
    matches!(
        words.first().map(String::as_str),
        Some("can" | "could" | "would" | "will" | "should")
    )
    .then(|| words.get(1))
    .flatten()
    .filter(|word| word.as_str() == "you")
    .and_then(|_| words.get(2).map(String::as_str))
}

fn is_informational_verb(word: &str) -> bool {
    matches!(
        word,
        "explain"
            | "describe"
            | "summarize"
            | "show"
            | "tell"
            | "what"
            | "why"
            | "when"
            | "where"
            | "who"
            | "which"
            | "how"
            | "is"
            | "are"
            | "do"
            | "does"
            | "did"
            | "can"
            | "could"
            | "would"
            | "will"
            | "should"
    )
}

fn contains_any(words: &[String], markers: &[&str]) -> bool {
    words
        .iter()
        .any(|word| markers.iter().any(|marker| word == marker))
}

pub(crate) fn inject_plan_mode_hint(input: String, current_mode: PermissionMode) -> String {
    match plan_mode_hint_for_prompt(&input, current_mode) {
        Some(hint) => format!("{hint}\n\n{input}"),
        None => input,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_hint_for_complex_coding_work() {
        let prompt = inject_plan_mode_hint(
            "Update src/a.rs and src/b.rs, then migrate config".into(),
            PermissionMode::Default,
        );

        assert!(prompt.starts_with("This request spans multiple implementation concerns."));
        assert!(prompt.ends_with("Update src/a.rs and src/b.rs, then migrate config"));
    }

    #[test]
    fn skips_general_questions_with_coding_marker_substrings() {
        assert!(
            plan_mode_hint_for_prompt(
                "Is address documentation stored in src/a.rs and src/b.rs?",
                PermissionMode::Default,
            )
            .is_none()
        );
        assert!(
            plan_mode_hint_for_prompt(
                "Give a prefix explanation for src/a.rs and src/b.rs.",
                PermissionMode::Default,
            )
            .is_none()
        );
        assert!(
            plan_mode_hint_for_prompt(
                "What update happened in src/a.rs and src/b.rs?",
                PermissionMode::Default,
            )
            .is_none()
        );
    }

    #[test]
    fn skips_leading_informational_requests_but_keeps_imperative_coding() {
        assert!(
            plan_mode_hint_for_prompt(
                "Explain the update in src/a.rs and src/b.rs",
                PermissionMode::Default,
            )
            .is_none()
        );
        assert!(
            plan_mode_hint_for_prompt(
                "Explain then update src/a.rs and src/b.rs",
                PermissionMode::Default,
            )
            .is_none()
        );
        assert!(
            plan_mode_hint_for_prompt("Update src/a.rs and src/b.rs", PermissionMode::Default,)
                .is_some()
        );
    }

    #[test]
    fn hints_second_person_auxiliary_coding_requests_only() {
        assert!(
            plan_mode_hint_for_prompt(
                "Can you update src/a.rs and src/b.rs?",
                PermissionMode::Default,
            )
            .is_some()
        );
        assert!(
            plan_mode_hint_for_prompt(
                "Would you refactor src/a.rs and src/b.rs?",
                PermissionMode::Default,
            )
            .is_some()
        );
        assert!(
            plan_mode_hint_for_prompt(
                "Can you explain the update in src/a.rs and src/b.rs?",
                PermissionMode::Default,
            )
            .is_none()
        );
        assert!(
            plan_mode_hint_for_prompt(
                "Would you describe the refactor in src/a.rs and src/b.rs?",
                PermissionMode::Default,
            )
            .is_none()
        );
        assert!(
            plan_mode_hint_for_prompt(
                "Can I update src/a.rs and src/b.rs?",
                PermissionMode::Default,
            )
            .is_none()
        );
        assert!(
            plan_mode_hint_for_prompt(
                "Could this update affect src/a.rs and src/b.rs?",
                PermissionMode::Default,
            )
            .is_none()
        );
    }

    #[test]
    fn skips_hint_for_simple_or_already_planned_requests() {
        assert_eq!(
            inject_plan_mode_hint("Fix typo".into(), PermissionMode::Default),
            "Fix typo"
        );
        assert_eq!(
            inject_plan_mode_hint("Update src/a.rs and src/b.rs".into(), PermissionMode::Plan,),
            "Update src/a.rs and src/b.rs"
        );
        assert_eq!(
            inject_plan_mode_hint("What is plan mode?".into(), PermissionMode::Default),
            "What is plan mode?"
        );
    }
}
