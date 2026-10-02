use super::*;
use crate::spawn_placement::Placement;
use crate::workflow_read_guard::RunStoreScope;
use std::path::PathBuf;

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "{args:?}: {out:?}");
}

/// A canonical checkout keeping its run store inside it, with the agent's
/// worktree and a sibling's inside the store, the way the host lays them out.
fn layout() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let canonical = std::fs::canonicalize(temp.path())
        .unwrap()
        .join("canonical");
    std::fs::create_dir_all(canonical.join("src")).unwrap();
    git(&canonical, &["init", "-q"]);
    git(&canonical, &["config", "user.email", "t@example.invalid"]);
    git(&canonical, &["config", "user.name", "t"]);
    std::fs::write(canonical.join("src/lib.rs"), "base\n").unwrap();
    std::fs::write(canonical.join(".gitignore"), ".store/\n").unwrap();
    git(&canonical, &["add", "."]);
    git(&canonical, &["commit", "-qm", "base"]);
    let workspace = canonical.join(".store/run/worktrees/mine");
    let sibling = canonical.join(".store/run/worktrees/sibling");
    for wt in [&workspace, &sibling] {
        git(
            &canonical,
            &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()],
        );
    }
    (temp, canonical, workspace, sibling)
}

fn ctx(canonical: &Path, workspace: &Path) -> ToolContext {
    let workflow = ToolContext {
        working_dir: canonical.to_path_buf(),
        run_store: Some(RunStoreScope::default()),
        ..ToolContext::default()
    };
    let placement = Placement::resolve(&workflow, workspace, None);
    ToolContext {
        working_dir: workspace.to_path_buf(),
        sealed_repositories: vec![placement.sealed_repository().unwrap().to_path_buf()],
        run_store: Some(RunStoreScope::default()),
        ..ToolContext::default()
    }
}

fn check(path: &Path, ctx: &ToolContext) -> Result<(), String> {
    ensure_not_sealed(path, path, ctx)
}

#[test]
fn the_isolated_checkout_and_its_siblings_are_refused_and_the_workspace_is_not() {
    let (_t, canonical, workspace, sibling) = layout();
    let ctx = ctx(&canonical, &workspace);
    let refused = check(&canonical.join("src/lib.rs"), &ctx).unwrap_err();
    assert!(refused.contains("isolated from"), "{refused}");
    assert!(check(&workspace.join("src/lib.rs"), &ctx).is_ok());
    assert!(check(&sibling.join("src/lib.rs"), &ctx).is_err());
    assert!(check(Path::new("/elsewhere/a"), &ctx).is_ok());
}

/// Decision: an interactive agent keeps its access; a seal applies in a
/// workflow context only.
#[test]
fn outside_a_workflow_nothing_is_sealed() {
    let (_t, canonical, workspace, _s) = layout();
    let mut ctx = ctx(&canonical, &workspace);
    ctx.run_store = None;
    assert!(check(&canonical.join("src/lib.rs"), &ctx).is_ok());
}

/// Issue-213 C3: a link in the workspace into the sealed checkout is judged
/// by where it lands.
#[cfg(unix)]
#[test]
fn a_link_into_the_sealed_checkout_is_refused_by_where_it_resolves() {
    let (_t, canonical, workspace, _s) = layout();
    let ctx = ctx(&canonical, &workspace);
    let link = workspace.join("x");
    std::os::unix::fs::symlink(canonical.join("src"), &link).unwrap();
    let named = link.join("lib.rs");
    let resolved = std::fs::canonicalize(&named).unwrap();
    assert!(ensure_not_sealed(&named, &resolved, &ctx).is_err());
}

#[test]
fn a_run_store_inside_the_checkout_is_left_to_the_run_store_guard() {
    let (_t, canonical, workspace, sibling) = layout();
    let mut ctx = ctx(&canonical, &workspace);
    let run = canonical.join(".store/run");
    ctx.run_store = Some(RunStoreScope::new(
        run.to_str(),
        canonical.join(".store").to_str(),
        workspace.to_str(),
    ));
    assert!(check(&run.join("artifacts/report.md"), &ctx).is_ok());
    assert!(
        check(&sibling.join("a"), &ctx).is_err(),
        "a sibling owns its paths"
    );
    assert!(check(&canonical.join("src/lib.rs"), &ctx).is_err());
}

#[test]
fn a_write_root_inside_the_checkout_passes_and_one_covering_it_does_not() {
    let (_t, canonical, workspace, _s) = layout();
    let mut ctx = ctx(&canonical, &workspace);
    ctx.write_roots = vec![canonical.join("registry")];
    assert!(check(&canonical.join("registry/a.json"), &ctx).is_ok());
    ctx.write_roots = vec![canonical.clone()];
    assert!(check(&canonical.join("registry/a.json"), &ctx).is_err());
}

#[test]
fn a_path_the_host_declared_writable_passes() {
    let (_t, canonical, workspace, _s) = layout();
    let mut ctx = ctx(&canonical, &workspace);
    let declared = canonical.join("data");
    let boundary = crate::workflow_read_guard::HostWriteBoundary::new(
        &[canonical.display().to_string()],
        &[declared.display().to_string()],
    );
    ctx.workflow_read_guard = Some(std::sync::Arc::new(
        crate::workflow_read_guard::WorkflowReadGuard::new(40, 20, false, false)
            .with_declared_targets(
                crate::workflow_read_guard::DeclaredTargetScope::new(
                    &["src/lib.rs".to_string()],
                    workspace.to_str(),
                )
                .in_isolated_worktree(true)
                .with_write_boundary(boundary),
            ),
    ));
    assert!(check(&declared.join("a.json"), &ctx).is_ok());
    assert!(check(&canonical.join("src/lib.rs"), &ctx).is_err());
}
