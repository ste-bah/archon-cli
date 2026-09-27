//! Issue-64: a write outside the declared (widened) target set is refused at
//! write time; declared, widened and out-of-worktree paths pass.
use super::{DeclaredTargetScope, scope_declared_targets};
use crate::workflow_read_guard::{WorkflowReadGuard, WorkflowReadGuardSettings};
use serde_json::json;

pub(super) fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// A worktree with the declared set of a live-shaped branch: six declared
/// files, one directory scope, and one obligation file the baseline widened
/// the set with.
pub(super) fn scope(worktree: &std::path::Path) -> DeclaredTargetScope {
    DeclaredTargetScope::new(
        &strings(&[
            "crates/engine/src/adapter.rs",
            "crates/engine/src/adapter_tests.rs",
            "crates/engine/tests/adapter_unavailable.rs",
            "artifacts/runs/",
            // Widened by the baseline (an obligation file), not declared.
            "crates/engine/src/store/ingest_tests.rs",
        ]),
        Some(&worktree.display().to_string()),
    )
}

pub(super) fn guard(scope: DeclaredTargetScope) -> WorkflowReadGuard {
    WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default())
        .with_declared_targets(scope)
}

pub(super) fn write(
    guard: &WorkflowReadGuard,
    tool: &str,
    key: &str,
    path: &str,
) -> Option<String> {
    guard.before_tool(tool, &json!({ key: path, "content": "x", "patch": "x" }))
}

pub(super) fn bash(guard: &WorkflowReadGuard, command: &str) -> Option<String> {
    guard.before_tool("Bash", &json!({ "command": command }))
}

#[test]
fn a_write_to_an_undeclared_worktree_path_is_refused_with_the_full_message() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let guard = guard(scope(&worktree));
    let outside = worktree.join("crates/engine/src/gates.rs");
    let refusal = write(&guard, "Write", "file_path", &outside.display().to_string())
        .expect("an undeclared write is refused");
    assert_eq!(
        refusal,
        format!(
            "Error: {} is not in this branch's declared target_files; undeclared changes are \
             dropped from the patch at the gate, so this edit would be lost. Declared targets \
             (5): artifacts/runs/, crates/engine/src/adapter.rs, \
             crates/engine/src/adapter_tests.rs, crates/engine/src/store/ingest_tests.rs, \
             crates/engine/tests/adapter_unavailable.rs. If the change is genuinely required \
             to satisfy your own focused checks, do not edit the file: record it in \
             residual_gaps naming the file and the owner task (or \"unowned\"). The operator \
             may disable workflow.generated.enforce_declared_targets.",
            outside.display()
        )
    );
    // Every file-mutating tool, absolute or relative, new file or not, and
    // through the canonicalised root.
    for (tool, key, path) in [
        (
            "Edit",
            "file_path",
            "crates/engine/src/gates/catalog.rs".to_string(),
        ),
        (
            "MultiEdit",
            "file_path",
            "./crates/engine/src/lib.rs".to_string(),
        ),
        (
            "ApplyPatch",
            "path",
            "crates/engine/src/brand_new_file.rs".to_string(),
        ),
        ("NotebookEdit", "file_path", "notes.ipynb".to_string()),
        (
            "LargeEditBegin",
            "file_path",
            std::fs::canonicalize(&worktree)
                .unwrap()
                .join("Cargo.toml")
                .display()
                .to_string(),
        ),
    ] {
        let refusal = write(&guard, tool, key, &path);
        assert!(
            refusal
                .as_deref()
                .is_some_and(|r| r.contains("is not in this branch's declared target_files")),
            "{tool} {path}: {refusal:?}"
        );
    }
}

#[test]
fn a_declared_path_a_widened_obligation_file_and_a_directory_scope_pass() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let guard = guard(scope(&worktree));
    for path in [
        worktree
            .join("crates/engine/src/adapter.rs")
            .display()
            .to_string(),
        "crates/engine/src/adapter_tests.rs".to_string(),
        "./crates/engine/tests/adapter_unavailable.rs".to_string(),
        // The obligation file the baseline widened the set with.
        "crates/engine/src/store/ingest_tests.rs".to_string(),
        // Under a declared directory scope.
        "artifacts/runs/2026-09-21/record.json".to_string(),
        worktree.join("artifacts/runs").display().to_string(),
    ] {
        assert!(
            write(&guard, "Write", "file_path", &path).is_none(),
            "{path} was refused"
        );
    }
}

#[test]
fn a_path_outside_the_worktree_root_is_not_judged() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    let project = temp.path().join("project/.archon/notes");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let guard = guard(scope(&worktree));
    for path in [
        project.join("evidence.md").display().to_string(),
        "/tmp/scratch.txt".to_string(),
        "../sibling/src/lib.rs".to_string(),
    ] {
        assert!(
            write(&guard, "Write", "file_path", &path).is_none(),
            "{path} was refused"
        );
        assert!(
            bash(&guard, &format!("sed -i 's/a/b/' {path}")).is_none(),
            "{path} was refused via sed"
        );
    }
}

#[test]
fn a_shell_write_naming_an_undeclared_path_is_refused_and_a_declared_one_runs() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let guard = guard(scope(&worktree));
    let refusal = bash(
        &guard,
        "sed -i 's/use a;/use b;/' crates/engine/src/gates.rs && cargo check",
    )
    .expect("sed -i at an undeclared path is refused");
    assert!(
        refusal.starts_with(
            "`sed -i s/use a;/use b;/ crates/engine/src/gates.rs` writes \
             crates/engine/src/gates.rs, which is not in this branch's declared target_files;"
        ),
        "{refusal}"
    );
    assert!(refusal.contains(
        "record it in residual_gaps naming the file and the owner task (or \"unowned\")"
    ));
    for command in [
        "cat > crates/engine/src/lib.rs <<'EOF'\nx\nEOF",
        "cargo test 2>&1 | tee crates/engine/out.txt",
        "python3 -c \"open('crates/engine/src/gen.rs','w').write('x')\"",
    ] {
        assert!(bash(&guard, command).is_some(), "{command} ran");
    }
    for command in [
        "sed -i 's/a/b/' crates/engine/src/adapter.rs",
        "cat > artifacts/runs/out.json <<'EOF'\n{}\nEOF",
        "cargo test -p engine 2>&1 | tee /tmp/out.txt",
        "cargo clippy -p engine -- -D warnings",
        "sed -n '1,20p' crates/engine/src/gates.rs",
        "grep -rn gates crates/engine/src > /tmp/hits",
    ] {
        assert!(bash(&guard, command).is_none(), "{command} was refused");
    }
}

#[test]
fn the_operator_switch_turns_the_rule_off_and_an_unscoped_guard_is_inert() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let off = WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings {
        enforce_declared_targets: false,
        ..WorkflowReadGuardSettings::default()
    })
    .with_declared_targets(scope(&worktree));
    assert!(write(&off, "Write", "file_path", "crates/engine/src/gates.rs").is_none());
    assert!(bash(&off, "sed -i 's/a/b/' crates/engine/src/gates.rs").is_none());
    // The other refusals are untouched by the switch.
    assert!(bash(&off, "cargo fmt --all").is_some());

    let unscoped = WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default());
    assert!(
        write(
            &unscoped,
            "Write",
            "file_path",
            "crates/engine/src/gates.rs"
        )
        .is_none()
    );
    let no_root = guard(DeclaredTargetScope::new(&strings(&["src/a.rs"]), None));
    assert!(write(&no_root, "Write", "file_path", "src/b.rs").is_none());
}

#[test]
fn the_scope_reaches_a_guard_built_inside_it_and_the_switch_still_wins() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (on, off) = runtime.block_on(scope_declared_targets(scope(&worktree), async {
        (
            WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings::default()),
            WorkflowReadGuard::from_settings(&WorkflowReadGuardSettings {
                enforce_declared_targets: false,
                ..WorkflowReadGuardSettings::default()
            }),
        )
    }));
    assert!(write(&on, "Edit", "file_path", "crates/engine/src/gates.rs").is_some());
    assert!(write(&on, "Edit", "file_path", "crates/engine/src/adapter.rs").is_none());
    assert!(write(&off, "Edit", "file_path", "crates/engine/src/gates.rs").is_none());
}

/// Issue-120: with the landing's grant stamped, the guard admits exactly
/// what the landing keeps -- an undeclared path inside the scope roots no
/// other item claims -- and refuses the rest with what the landing would
/// actually do. Without the stamp nothing changes.
#[test]
fn a_path_the_landing_would_grant_is_admitted_and_the_rest_are_told_the_landing_verdict() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let at = |rel: &str| worktree.join(rel).display().to_string();
    let granting = guard(scope(&worktree).with_grantable(
        &strings(&["crates/engine/", "artifacts/runs/"]),
        &strings(&["crates/engine/src/registry.rs", "crates/engine/src/lanes/"]),
    ));
    // Unclaimed, inside the roots: the landing grants it, by Edit or shell.
    assert_eq!(
        write(
            &granting,
            "Edit",
            "file_path",
            &at("crates/engine/src/gates.rs")
        ),
        None
    );
    assert_eq!(
        bash(&granting, "cat > crates/engine/src/gates.rs <<'X'\nx\nX"),
        None
    );
    // A path directly at the repository root is inside every ceiling.
    assert_eq!(
        write(&granting, "Write", "file_path", &at("build.rs")),
        None
    );
    let claimed = write(
        &granting,
        "Edit",
        "file_path",
        &at("crates/engine/src/lanes/a.rs"),
    )
    .expect("another item's scope is refused");
    assert!(
        claimed.contains(
            "another item of this wave, or a task this branch does not serve, declares it, so the landing would refuse this branch's WHOLE patch"
        ),
        "{claimed}"
    );
    assert!(!claimed.contains("would be lost"), "{claimed}");
    let outside =
        bash(&granting, "echo x > crates/other/src/lib.rs").expect("outside the roots is refused");
    assert!(
        outside.contains(
            "it is outside this branch's scope roots (artifacts/runs/, crates/engine/), so the landing would discard it"
        ),
        "{outside}"
    );
    // The same path without the stamp: refused as before.
    let strict = guard(scope(&worktree));
    let before = write(
        &strict,
        "Edit",
        "file_path",
        &at("crates/engine/src/gates.rs"),
    )
    .expect("refused without the stamp");
    assert!(before.contains("so this edit would be lost"), "{before}");
}

/// Issue-120: a grantable path the task forbids is never admitted, by Edit,
/// by a shell write the forbidden rule cannot see, or spelled with `..`:
/// the landing rejects a branch that changes one.
#[test]
fn a_grantable_path_the_task_forbids_is_refused_however_it_is_written() {
    let temp = tempfile::tempdir().unwrap();
    let worktree = temp.path().join("iso/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let root = worktree.display().to_string();
    let at = |rel: &str| worktree.join(rel).display().to_string();
    let guard = guard(scope(&worktree).with_grantable(&strings(&["crates/engine/"]), &[]))
        .with_forbidden_paths(crate::workflow_read_guard::ForbiddenPathScope::new(
            &strings(&["`crates/engine/src/frozen.rs` (frozen)"]),
            &[root],
        ));
    for refusal in [
        write(
            &guard,
            "Edit",
            "file_path",
            &at("crates/engine/src/frozen.rs"),
        ),
        bash(&guard, "cat > crates/engine/src/frozen.rs <<'X'\nx\nX"),
        write(
            &guard,
            "Edit",
            "file_path",
            &at("crates/engine/src/x/../frozen.rs"),
        ),
    ] {
        let refusal = refusal.expect("a forbidden grantable path is refused");
        assert!(refusal.contains("forbid"), "{refusal}");
    }
    // Its unforbidden neighbour is still granted.
    assert_eq!(
        bash(&guard, "cat > crates/engine/src/free.rs <<'X'\nx\nX"),
        None
    );
}
