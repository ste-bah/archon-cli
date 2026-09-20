//! The host names the roots, and names them from what it already resolved.

use super::declared_write_roots;
use crate::v2::WorkflowV2ProjectArtifactContext;

fn absolute(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{path}")
    } else {
        path.to_string()
    }
}

fn context() -> WorkflowV2ProjectArtifactContext {
    WorkflowV2ProjectArtifactContext {
        project_root: Some(absolute("/projects/project-1")),
        // Relative by construction — see `artifact_roots_for_run`. Already
        // inside the project root, so they add nothing and must not be
        // resolved against this process's working directory.
        artifact_roots: vec![
            ".archon/artifacts".to_string(),
            ".archon/workflows/run-7".to_string(),
        ],
        ..WorkflowV2ProjectArtifactContext::default()
    }
}

/// The case the whole design turns on: the deliverable lives under a project
/// directory that is NOT the repository the agent has checked out, and is not a
/// git repository at all. Confining to the workspace would refuse it.
#[test]
fn the_project_root_is_writable_even_when_it_is_not_the_repository() {
    let roots = declared_write_roots(&context(), Some(&absolute("/checkouts/some-repo")));

    assert!(
        roots.contains(&absolute("/projects/project-1")),
        "the project root holds the deliverable and must be writable: {roots:?}"
    );
    assert!(
        roots.contains(&absolute("/checkouts/some-repo")),
        "the repository the agent edits must be writable too: {roots:?}"
    );
}

/// Relative entries are dropped rather than joined. Joining would restate the
/// project root; resolving them against the process working directory would
/// widen the set to wherever archon happens to have been launched.
#[test]
fn relative_artifact_roots_do_not_become_roots() {
    let roots = declared_write_roots(&context(), None);

    assert_eq!(
        roots,
        vec![absolute("/projects/project-1")],
        "only absolute, host-resolved directories may confine a write: {roots:?}"
    );
}

/// Nothing declared means nothing to enforce. The caller decides what to do
/// about that; inventing a root here is the substitution that blocks a
/// deliverable.
#[test]
fn an_undeclared_run_yields_no_roots() {
    let roots = declared_write_roots(&WorkflowV2ProjectArtifactContext::default(), None);

    assert!(
        roots.is_empty(),
        "an undeclared run must not be confined by a guess: {roots:?}"
    );
}

/// The same directory named twice is one root. Duplicates are harmless to the
/// check and confusing in the refusal message, which lists them.
#[test]
fn a_repository_that_is_the_project_root_is_listed_once() {
    let mut artifacts = context();
    artifacts.repository_root = Some(absolute("/projects/project-1"));

    let roots = declared_write_roots(&artifacts, Some(&absolute("/projects/project-1")));

    assert_eq!(roots, vec![absolute("/projects/project-1")], "{roots:?}");
}

/// An absolute evidence root outside the project is carried; the run writes
/// there and would otherwise be refused its own bookkeeping.
#[test]
fn an_absolute_branch_evidence_root_is_carried() {
    let mut artifacts = context();
    artifacts.branch_evidence_root = Some(absolute("/evidence/run-7"));

    let roots = declared_write_roots(&artifacts, None);

    assert!(
        roots.contains(&absolute("/evidence/run-7")),
        "evidence the run writes must be writable: {roots:?}"
    );
}
