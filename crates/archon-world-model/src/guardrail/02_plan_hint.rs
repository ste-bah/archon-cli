const EXPLICIT_PLAN_MARKERS: &[&[&str]] = &[
    &["multi", "part"],
    &["multiple", "parts"],
    &["multi", "step"],
    &["high", "risk"],
];

const COMMON_FILE_EXTENSIONS: &[&str] = &[
    "c", "cc", "cpp", "cs", "css", "go", "h", "hpp", "html", "java", "js", "json", "jsx", "kt",
    "lock", "md", "php", "py", "rb", "rs", "scss", "sql", "svelte", "swift", "toml", "ts", "tsx",
    "vue", "xml", "yaml", "yml",
];

const PLAN_MODE_HINT: &str =
    "This request spans multiple implementation concerns. Consider using /plan to review and approve a structured plan before editing.";

pub fn required_evidence_kind(
    required: GuardrailRequiredAction,
) -> archon_completion::RequiredEvidenceKind {
    use archon_completion::RequiredEvidenceKind;

    match required {
        GuardrailRequiredAction::RunTests => RequiredEvidenceKind::Tests,
        GuardrailRequiredAction::RunBuild => RequiredEvidenceKind::Build,
        GuardrailRequiredAction::RunLint => RequiredEvidenceKind::Lint,
        GuardrailRequiredAction::RunTypecheck => RequiredEvidenceKind::Typecheck,
        GuardrailRequiredAction::RunVerifier => RequiredEvidenceKind::Verifier,
        GuardrailRequiredAction::ReviewPlanAgainstUserGoal => RequiredEvidenceKind::PlanReview,
        GuardrailRequiredAction::CheckSourceEvidence => RequiredEvidenceKind::SourceEvidence,
        GuardrailRequiredAction::RecordManualOutcome => RequiredEvidenceKind::ManualOutcome,
        GuardrailRequiredAction::RequireUserApproval => RequiredEvidenceKind::HumanApproval,
    }
}

pub fn should_suggest_plan_mode(summary: &str, class: RuntimeTaskClass) -> bool {
    matches!(
        class,
        RuntimeTaskClass::CodingChange | RuntimeTaskClass::Refactor | RuntimeTaskClass::Debugging
    ) && (has_multiple_path_mentions(summary) || has_explicit_plan_marker(summary))
}

pub fn plan_mode_hint(
    summary: &str,
    class: RuntimeTaskClass,
    current_mode: archon_permissions::mode::PermissionMode,
) -> Option<&'static str> {
    (current_mode != archon_permissions::mode::PermissionMode::Plan
        && should_suggest_plan_mode(summary, class))
        .then_some(PLAN_MODE_HINT)
}

fn has_multiple_path_mentions(summary: &str) -> bool {
    let paths = summary
        .split_whitespace()
        .filter_map(normalize_file_reference)
        .collect::<std::collections::BTreeSet<_>>();

    paths.len() >= 2
}

fn normalize_file_reference(token: &str) -> Option<String> {
    let token = token.trim_matches(|character: char| {
        character.is_ascii_punctuation() && character != '/' && character != '.' && character != '_'
    });
    looks_like_path(token).then(|| token.to_ascii_lowercase())
}

fn looks_like_path(token: &str) -> bool {
    token.contains('/')
        || token.contains('\\')
        || token.rsplit_once('.').is_some_and(|(name, extension)| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
                && COMMON_FILE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

fn has_explicit_plan_marker(summary: &str) -> bool {
    let words = summary
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();

    EXPLICIT_PLAN_MARKERS.iter().any(|marker| {
        words
            .windows(marker.len())
            .any(|window| window.iter().map(String::as_str).eq(marker.iter().copied()))
    })
}
