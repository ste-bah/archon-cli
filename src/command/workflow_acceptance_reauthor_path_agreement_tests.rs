use super::*;
use archon_workflow::task_set_contract::TrustedCwd;

#[cfg(unix)]
#[path = "workflow_acceptance_reauthor_path_matrix_helpers_tests.rs"]
mod path_matrix_helpers;

#[test]
fn an_unresolvable_root_is_refused_with_its_actual_reason() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let missing = set.project.path().join("not-created");
    let refusal = archon_workflow::acceptance_scratch::site_path_alone(
        &missing,
        &missing,
        "test -f ../input",
        TrustedCwd::ProjectRoot,
        std::path::Path::new("../input"),
        false,
    )
    .expect_err("an unavailable root cannot model a prepared site");
    assert!(refusal.site_target == std::path::Path::new("<site>"));
    assert!(
        refusal
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("project root cannot be resolved")),
        "{refusal:?}"
    );
}

#[test]
fn repository_root_path_to_project_root_is_refused() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let base = set.project.path().join("roots");
    let project = base.join("workspace");
    let repo = base.join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(project.join("item"), "present").unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -f ../workspace/item".into(),
        cwd: TrustedCwd::RepoRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect_err("the site refuses a repository-root path into the project root");
    assert!(finding.contains("`../workspace`"), "{finding}");
    assert!(finding.contains("<site>/workspace/item"), "{finding}");
    assert!(
        finding.contains(archon_workflow::acceptance_scratch::faults::SIBLING_ROOT_REFUSED),
        "{finding}"
    );
}

#[test]
fn a_whole_check_refusal_names_the_sibling_that_caused_it() {
    let set = frozen_set(&[("AC-X-002", "true", false)]);
    let project = set.project.path().join("workspace");
    let repository = set.project.path().join("repository");
    let other = set.project.path().join("other");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(project.join("item"), "project input").unwrap();
    std::fs::write(other.join("item"), "other input").unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repository;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -e ../other/item && test -e ../workspace/item".into(),
        cwd: TrustedCwd::RepoRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect_err("the later project alias refuses the whole check");
    assert!(finding.contains("`../workspace`"), "{finding}");
    assert!(!finding.contains("`../other/item`"), "{finding}");
    assert!(finding.contains("<site>/workspace/item"), "{finding}");
    assert!(
        finding.contains(archon_workflow::acceptance_scratch::faults::SIBLING_ROOT_REFUSED),
        "{finding}"
    );
}

#[test]
fn a_cross_root_path_in_here_document_finding_names_the_path() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let project = set.project.path().join("workspace");
    let repo = set.project.path().join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    let named = "../external/item";
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: format!("runner - <<'PROGRAM'\nopen('{named}')\nPROGRAM"),
        cwd: TrustedCwd::ProjectRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect_err("the here-document path has no site copy");
    assert!(finding.contains(named), "{finding}");
    assert!(finding.contains("<site>/external/item"), "{finding}");
}

#[test]
fn repository_root_path_cannot_leave_and_reenter_a_directory_named_repo() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let base = set.project.path().join("roots");
    for (index, repository_name) in ["repository", "repo"].into_iter().enumerate() {
        let project = base.join(format!("workspace-{index}"));
        let repository = base.join(repository_name);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&repository).unwrap();
        std::fs::write(repository.join("item"), "present").unwrap();
        let mut author_scope = scope(&set);
        author_scope.project_root = project;
        author_scope.repository_root = repository;
        let mut candidate = set.contract().acceptance[0].clone();
        candidate.check = AcceptanceCheck::Command {
            command: "test -f ../repo/item".into(),
            cwd: TrustedCwd::RepoRoot,
        };
        let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
            .expect_err("the named path leaves the live repository root");
        assert!(finding.contains("leaves the repository root"), "{finding}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn author_path_decisions_agree_with_one_check_site_for_neutral_layout() {
    path_matrix_helpers::run_layout("neutral", 0).await;
}

#[cfg(unix)]
#[tokio::test]
async fn author_path_decisions_agree_with_one_check_site_for_site_names_layout() {
    path_matrix_helpers::run_layout("site_names", 1).await;
}

#[cfg(unix)]
#[tokio::test]
async fn author_path_decisions_agree_with_one_check_site_for_repository_inside_project_layout() {
    path_matrix_helpers::run_layout("repository_inside_project", 2).await;
}

#[cfg(unix)]
#[tokio::test]
async fn author_path_decisions_agree_with_one_check_site_for_project_inside_repository_layout() {
    path_matrix_helpers::run_layout("project_inside_repository", 3).await;
}

#[cfg(unix)]
#[tokio::test]
async fn author_path_decisions_agree_with_one_check_site_for_same_path_layout() {
    path_matrix_helpers::run_layout("same_path", 4).await;
}
