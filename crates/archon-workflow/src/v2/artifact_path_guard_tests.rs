//! Child module of `artifact_path_guard`, so these reach its private helpers.
//!
//! The prose fixtures are the verbatim acceptance criteria that run
//! `wf-67dd2599` turned into directories, including the one whose `/` produced
//! a nested tree.

use super::*;

/// Verbatim from issue #168. The `/` in "environment/readiness blockers" is
/// what produced a parent directory with a child directory inside it.
const NESTED_CRITERION: &str = "A gap-audit report or equivalent reducer evidence that cites \
     inspected source files and separates repo implementation gaps from environment/readiness \
     blockers";

/// Verbatim from issue #168. Nests at `provider/artifact`.
const SLASHED_CRITERION: &str = "If any provider/artifact contract is unavailable, record a \
     fail-closed residual gap rather than creating a healthy dataset registry entry";

#[test]
fn acceptance_criterion_with_a_slash_is_refused_not_turned_into_a_tree() {
    for criterion in [NESTED_CRITERION, SLASHED_CRITERION] {
        let rejection = validate_declared_artifact_path(criterion)
            .expect_err("an acceptance criterion is not a path");
        assert!(
            matches!(rejection, ArtifactPathRejection::Prose { .. }),
            "expected a prose refusal for {criterion:?}, got {rejection:?}"
        );
        // The refusal must arrive before any component of the sentence is
        // treated as a directory name.
        assert!(
            rejection.to_string().contains("reads as prose"),
            "refusal must say why: {rejection}"
        );
    }
}

/// The criterion is refused as one value, before any `/` in it is read as a
/// separator. That matters because the tail fragment on its own —
/// `readiness blockers` — is two unpunctuated words and would pass as a
/// directory name. It never gets the chance: the whole sentence is refused, so
/// no part of it is ever handed to anything that would create it.
#[test]
fn a_criterion_is_refused_whole_before_its_slash_is_read_as_a_separator() {
    let leading = NESTED_CRITERION.split('/').next().expect("leading segment");
    let trailing = NESTED_CRITERION
        .split('/')
        .nth(1)
        .expect("trailing segment");

    assert!(validate_declared_artifact_path(NESTED_CRITERION).is_err());
    assert!(
        validate_declared_artifact_path(leading).is_err(),
        "the segment that would become the project-root entry must be refused"
    );
    assert_eq!(
        trailing, "readiness blockers",
        "fixture guard: this is the fragment that is innocuous alone"
    );
}

#[test]
fn short_punctuated_criteria_are_refused_too() {
    for criterion in [
        "Fail closed, always.",
        "Record: nothing.",
        "Is it present?",
        "No stubs!",
    ] {
        assert!(
            validate_declared_artifact_path(criterion).is_err(),
            "punctuated criterion must be refused: {criterion:?}"
        );
    }
}

#[test]
fn real_artifact_paths_are_accepted() {
    for path in [
        ".archon/artifacts/gap-audit.json",
        ".archon/workflows/wf-67dd2599-1463-499e-8622-3da72c13baba/artifacts/final-report.json",
        "artifacts/dataset-registry.json",
        "docs/reports/coverage-summary.md",
        "/tmp/project/.archon/artifacts/report.json",
        "reports/my report.md",
    ] {
        validate_declared_artifact_path(path)
            .unwrap_or_else(|err| panic!("legitimate path refused: {path:?}: {err}"));
    }
}

#[test]
fn oversize_segment_is_refused() {
    let segment = "a".repeat(MAX_ARTIFACT_SEGMENT_CHARS + 1);
    let rejection = validate_declared_artifact_path(&format!("artifacts/{segment}.json"))
        .expect_err("an oversize segment is refused");
    assert!(matches!(
        rejection,
        ArtifactPathRejection::SegmentTooLong { .. }
    ));
}

#[test]
fn unset_project_root_is_an_error_not_an_empty_expansion() {
    let raw = "${PROJECT_ROOT}/.archon/trading-lab/data/datasets/validation.json";
    let rejection =
        expand_project_root_template(raw, None).expect_err("unset PROJECT_ROOT must be an error");
    assert_eq!(
        rejection,
        ArtifactPathRejection::UnboundTemplateVariable {
            name: "PROJECT_ROOT".to_string()
        }
    );
    // The point of the error: the alternative silently yields a relative path.
    assert!(rejection.to_string().contains("never an empty expansion"));
}

#[test]
fn empty_project_root_binding_is_also_an_error() {
    let raw = "${PROJECT_ROOT}/.archon/artifacts/report.json";
    assert!(expand_project_root_template(raw, Some("   ")).is_err());
    assert!(expand_project_root_template(raw, Some("")).is_err());
}

#[test]
fn bound_project_root_expands() {
    let expanded = expand_project_root_template(
        "${PROJECT_ROOT}/.archon/artifacts/report.json",
        Some("/repo"),
    )
    .expect("bound PROJECT_ROOT expands");
    assert_eq!(expanded, "/repo/.archon/artifacts/report.json");
}

#[test]
fn other_shell_variables_are_named_rather_than_guessed() {
    let raw = "${PROJECT_ROOT}/.archon/trading-lab/data/datasets/${DATASET_ID}/${VERSION}/validation.json";
    let rejection =
        expand_project_root_template(raw, Some("/repo")).expect_err("DATASET_ID binds to nothing");
    assert_eq!(
        rejection,
        ArtifactPathRejection::UnboundTemplateVariable {
            name: "DATASET_ID".to_string()
        }
    );
}

#[test]
fn unexpanded_template_never_passes_validation() {
    for raw in [
        "${PROJECT_ROOT}/.archon/artifacts/report.json",
        ".archon/data/datasets/<dataset-id>/validation.json",
    ] {
        let rejection = validate_declared_artifact_path(raw).expect_err("template is not a path");
        assert!(matches!(
            rejection,
            ArtifactPathRejection::UnexpandedTemplate { .. }
        ));
    }
}

#[test]
fn malformed_template_is_refused_rather_than_half_expanded() {
    assert_eq!(
        expand_project_root_template("${PROJECT_ROOT/.archon/x.json", Some("/repo")),
        Err(ArtifactPathRejection::MalformedTemplate)
    );
}

#[test]
fn template_tokens_finds_both_shapes_in_order() {
    assert_eq!(
        template_tokens("${A}/x/<b>/y/${C}"),
        vec!["${A}".to_string(), "<b>".to_string(), "${C}".to_string()]
    );
}

#[test]
fn a_directory_is_not_artifact_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let directory = temp.path().join("gap-audit.json");
    std::fs::create_dir_all(&directory).expect("directory");

    assert!(directory.exists(), "the naive check would pass here");
    assert!(!artifact_file_is_evidence(&directory));
    assert_eq!(
        artifact_file_defect(&directory),
        Some("is a directory, not the declared file")
    );
}

#[test]
fn an_empty_file_is_not_artifact_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("report.json");
    std::fs::write(&path, "").expect("empty file");

    assert!(path.exists(), "the naive check would pass here");
    assert!(!artifact_file_is_evidence(&path));
    assert_eq!(artifact_file_defect(&path), Some("is an empty file"));
}

#[test]
fn a_non_empty_regular_file_is_artifact_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("report.json");
    std::fs::write(&path, "{}").expect("file");

    assert!(artifact_file_is_evidence(&path));
    assert_eq!(artifact_file_defect(&path), None);
}

#[test]
fn an_absent_path_is_reported_as_absent() {
    let temp = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        artifact_file_defect(&temp.path().join("nothing-here.json")),
        Some("does not exist")
    );
}

#[test]
fn project_root_litter_reports_criterion_named_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    // The litter, recreated exactly as `mkdir -p` on the criterion would leave
    // it: a parent named after the sentence up to the slash, with the remainder
    // as a child.
    let parent = NESTED_CRITERION.split('/').next().expect("parent segment");
    let child = NESTED_CRITERION.split('/').nth(1).expect("child segment");
    std::fs::create_dir_all(root.join(parent).join(child)).expect("litter");
    std::fs::create_dir_all(root.join("src")).expect("real dir");
    std::fs::write(root.join("Cargo.toml"), "[package]").expect("real file");

    let litter = project_root_path_litter(root);

    assert_eq!(litter, vec![parent.to_string()]);
    assert!(!litter.iter().any(|name| name == "src"));
    assert!(!litter.iter().any(|name| name == "Cargo.toml"));
}

#[test]
fn project_root_litter_is_empty_for_an_ordinary_tree() {
    let temp = tempfile::tempdir().expect("tempdir");
    for name in ["src", "crates", "docs", ".archon", "README.md"] {
        std::fs::create_dir_all(temp.path().join(name)).expect("entry");
    }
    assert!(project_root_path_litter(temp.path()).is_empty());
}

#[test]
fn litter_detection_uses_the_length_threshold_from_the_issue() {
    let long_name = "x".repeat(LITTER_NAME_CHARS + 1);
    assert!(entry_name_is_litter(&long_name));
    assert!(!entry_name_is_litter(&"x".repeat(LITTER_NAME_CHARS)));
}
