//! Guard == landing follow-ups: host bookkeeping names, two-way scope
//! roots, a branch with no declared targets, and the path-scoped restore
//! that lets a branch undo a side effect in another holder's file.
use super::DeclaredTargetScope;
use super::tests::{bash, guard, scope, strings, write};
use crate::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
use serde_json::json;

/// A host bookkeeping basename is dropped by the landing wherever it sits,
/// declared or granted (Issue-76), so the guard refuses it with that verdict
/// instead of admitting a write that would silently vanish.
#[test]
fn a_host_bookkeeping_basename_is_refused_even_when_declared_or_grantable() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let at = |rel: &str| worktree.join(rel).display().to_string();
    let journal = format!("crates/engine/src/{}.jsonl", "ab".repeat(32));
    for guard in [
        guard(scope(&worktree)),
        guard(scope(&worktree).with_grantable(&strings(&["crates/engine/", "artifacts/"]), &[])),
    ] {
        for refusal in [
            // Under a declared directory scope.
            write(
                &guard,
                "Write",
                "file_path",
                &at("artifacts/runs/patch_manifest.json"),
            ),
            // Inside the roots, unclaimed: grantable but for its name.
            write(
                &guard,
                "Edit",
                "file_path",
                &at("crates/engine/src/gate-envelope.json"),
            ),
            bash(&guard, &format!("echo x > {journal}")),
            // Directly at the repository root.
            bash(&guard, "cat > patch_manifest.json <<'X'\nx\nX"),
        ] {
            let refusal = refusal.expect("a host bookkeeping basename is refused");
            assert!(
                refusal.contains("the host reserves for its own coordination bookkeeping")
                    && refusal
                        .contains("even when it is a declared target, so this write would be lost"),
                "{refusal}"
            );
            assert!(
                !refusal.contains("not in this branch's declared"),
                "{refusal}"
            );
        }
        // A look-alike that is not the host's name is judged as before.
        assert_eq!(
            write(
                &guard,
                "Write",
                "file_path",
                &at("artifacts/runs/manifest.json")
            ),
            None
        );
    }
    // Outside the worktree it is not part of the patch, so not judged.
    let outside = temp.path().join("run/patch_manifest.json");
    assert_eq!(
        write(
            &guard(scope(&worktree)),
            "Write",
            "file_path",
            &outside.display().to_string()
        ),
        None
    );
}

/// The scope-roots ceiling matches as the landing's `ScopeRoots::covers`
/// does, in both directions: a path that is an ANCESTOR of a directory root
/// is inside the ceiling there, so it is not refused as "outside" here.
#[test]
fn scope_roots_match_in_both_directions_like_the_landing() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let at = |rel: &str| worktree.join(rel).display().to_string();
    let granting = guard(scope(&worktree).with_grantable(&strings(&["crates/engine/src/"]), &[]));
    assert_eq!(
        write(&granting, "Write", "file_path", &at("crates/engine")),
        None
    );
    assert_eq!(
        write(
            &granting,
            "Write",
            "file_path",
            &at("crates/engine/src/x.rs")
        ),
        None
    );
    // A sibling that merely shares a prefix is still outside.
    let sibling = write(
        &granting,
        "Write",
        "file_path",
        &at("crates/engine/srcx/a.rs"),
    )
    .expect("a prefix sibling is outside the roots");
    assert!(
        sibling.contains("outside this branch's scope roots"),
        "{sibling}"
    );
    let other = write(&granting, "Write", "file_path", &at("crates/other/a.rs"))
        .expect("an unrelated path is outside the roots");
    assert!(
        other.contains("outside this branch's scope roots"),
        "{other}"
    );
}

/// A write branch in its own isolated item worktree, asked to undo a build
/// side effect in another holder's file, can restore it: a path-scoped `git
/// checkout -- <path>` or `git restore` to `HEAD` is admitted. Every other
/// form, anything that could aim it elsewhere, a call not marked isolated
/// (a serial or coordinated write in the canonical tree), and a read-only
/// call stay refused.
#[test]
fn a_path_scoped_restore_is_admitted_only_in_an_isolated_worktree() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let granting = |isolated: bool| {
        guard(
            scope(&worktree)
                .with_grantable(&strings(&["crates/engine/"]), &strings(&["Cargo.lock"]))
                .in_isolated_worktree(isolated),
        )
    };
    let isolated = granting(true);
    // Writing the claimed file is refused, so a restore is the only way back.
    assert!(bash(&isolated, "git show HEAD:Cargo.lock > Cargo.lock").is_some());
    for command in [
        "git checkout -- Cargo.lock",
        "git checkout HEAD -- Cargo.lock crates/engine/gen.rs",
        "git restore Cargo.lock",
        "git restore --worktree --source=HEAD -- ./crates/engine/gen.rs",
    ] {
        assert_eq!(bash(&isolated, command), None, "{command}");
    }
    for command in [
        "git checkout main",
        "git checkout -- .",
        "git checkout -- ./",
        "git checkout -- crates/..",
        "git checkout -- ../other/Cargo.lock",
        "git checkout -- /etc/hosts",
        "git checkout -- ~/x",
        "git checkout -- \"$HOME/x\"",
        "git checkout -- `echo x`",
        "git checkout -- {a,b}",
        "git checkout HEAD~2 -- Cargo.lock",
        "git checkout -b other",
        "git restore --staged Cargo.lock",
        "git restore -s HEAD~1 Cargo.lock",
        "git restore -- 'crates/*'",
        "git checkout -- :/",
        "git -C /elsewhere checkout -- Cargo.lock",
        "git --git-dir=/elsewhere/.git checkout -- Cargo.lock",
        "git -c core.worktree=/elsewhere checkout -- Cargo.lock",
        "GIT_DIR=/elsewhere/.git git checkout -- Cargo.lock",
        "GIT_WORK_TREE=/elsewhere git restore Cargo.lock",
        "env GIT_DIR=/x git checkout -- Cargo.lock",
        "cd /elsewhere && git checkout -- Cargo.lock",
        "pushd crates && git restore Cargo.lock",
        "(cd ../../.. && git checkout -- src/x.rs)",
        "{ cd /elsewhere; git restore -- Cargo.lock; }",
        "if true; then cd /x; git checkout -- Cargo.lock; fi",
        "export GIT_WORK_TREE=/canonical; git checkout -- Cargo.lock",
        "git checkout -- 'a(b)'",
        "git checkout -- Cargo.lock; true",
    ] {
        let refusal = bash(&isolated, command).unwrap_or_else(|| panic!("{command} ran"));
        assert!(
            refusal.contains("is refused") && refusal.contains("`git checkout -- <path>...`"),
            "{refusal}"
        );
    }
    // A subshell no longer hides a mutation from the refusal at all.
    assert!(bash(&isolated, "(git reset --hard)").is_some());
    // Not marked isolated: the canonical tree a serial or coordinated write
    // shares. No restore, and no hint that one is allowed.
    let shared = granting(false);
    let refusal = bash(&shared, "git checkout -- Cargo.lock").expect("refused");
    assert!(!refusal.contains("is allowed"), "{refusal}");
    let read_only = WorkflowReadGuard::shell_only(&WorkflowReadGuardSettings::default());
    let refusal = read_only
        .before_tool("Bash", &json!({"command": "git checkout -- Cargo.lock"}))
        .expect("a read-only call may not restore");
    assert!(!refusal.contains("is allowed"), "{refusal}");
}

/// A branch that declares no targets is not judged by the declared-target
/// rule, but the landing still drops a host bookkeeping name it writes.
#[test]
fn a_branch_with_no_declared_targets_is_still_refused_host_bookkeeping_names() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let none = guard(DeclaredTargetScope::new(
        &[],
        Some(&worktree.display().to_string()),
    ));
    let at = |rel: &str| worktree.join(rel).display().to_string();
    assert!(
        write(&none, "Write", "file_path", &at("src/patch_manifest.json"))
            .is_some_and(|r| r.contains("reserves for its own coordination bookkeeping"))
    );
    assert_eq!(write(&none, "Write", "file_path", &at("src/lib.rs")), None);
}
