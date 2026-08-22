//! Write confinement: an agent may read the tree it was branched from, and may
//! not write it.
//!
//! The run this comes from recorded its repository root as a worktree while its
//! write agent edited five files in the canonical checkout, including 111 lines
//! of debug scaffolding. Nothing refused the write and nothing noticed it; the
//! stage had produced no result for four hours by the time a person read
//! `git status`. Reads and writes shared one list of permitted directories, so
//! handing the agent the canonical tree to read handed it the tree to write.

use super::{resolve_existing_write_target, resolve_write_target_path};
use crate::tool::ToolContext;

struct Trees {
    _root: tempfile::TempDir,
    canonical: std::path::PathBuf,
    worktree: std::path::PathBuf,
}

/// A canonical checkout and a worktree beside it, each holding one file.
fn trees() -> Trees {
    let root = tempfile::tempdir().expect("tempdir");
    let canonical = root.path().join("canonical");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&canonical).expect("canonical");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::write(canonical.join("lib.rs"), "// canonical\n").expect("seed canonical");
    std::fs::write(worktree.join("lib.rs"), "// worktree\n").expect("seed worktree");
    Trees {
        _root: root,
        canonical: std::fs::canonicalize(&canonical).expect("canonicalize"),
        worktree: std::fs::canonicalize(&worktree).expect("canonicalize"),
    }
}

/// The context a worktree-isolated write agent actually gets: its own working
/// directory, the canonical checkout inherited as a readable extra directory,
/// and writing confined to the worktree.
fn isolated(trees: &Trees) -> ToolContext {
    ToolContext {
        working_dir: trees.worktree.clone(),
        extra_dirs: vec![trees.canonical.clone()],
        write_roots: vec![trees.worktree.clone()],
        ..ToolContext::default()
    }
}

/// The same agent before this change: one list, so the canonical tree is
/// writable. Kept as a test so the difference is visible rather than asserted.
fn unconfined(trees: &Trees) -> ToolContext {
    ToolContext {
        working_dir: trees.worktree.clone(),
        extra_dirs: vec![trees.canonical.clone()],
        ..ToolContext::default()
    }
}

#[test]
fn an_isolated_agent_may_write_its_own_worktree() {
    let trees = trees();
    let ctx = isolated(&trees);
    let target = trees.worktree.join("new.rs");

    resolve_write_target_path(&target.display().to_string(), &ctx)
        .expect("its own worktree must stay writable");
}

#[test]
fn an_isolated_agent_may_edit_a_file_in_its_own_worktree() {
    let trees = trees();
    let ctx = isolated(&trees);
    let target = trees.worktree.join("lib.rs");

    resolve_existing_write_target(&target.display().to_string(), &ctx)
        .expect("editing its own file must stay allowed");
}

/// The defect. A new file in the canonical tree — what `Write` and `ApplyPatch`
/// resolve — must now be refused.
#[test]
fn an_isolated_agent_cannot_create_a_file_in_the_canonical_tree() {
    let trees = trees();
    let ctx = isolated(&trees);
    let target = trees.canonical.join("smuggled.rs");

    let error = resolve_write_target_path(&target.display().to_string(), &ctx)
        .expect_err("a write into the canonical tree must be refused");
    assert!(
        error.contains("outside this agent's writable directories"),
        "the refusal must say why: {error}"
    );
}

/// And the one that actually happened: editing a file that already exists in
/// the canonical checkout.
#[test]
fn an_isolated_agent_cannot_edit_a_file_in_the_canonical_tree() {
    let trees = trees();
    let ctx = isolated(&trees);
    let target = trees.canonical.join("lib.rs");

    let error = resolve_existing_write_target(&target.display().to_string(), &ctx)
        .expect_err("an edit of the canonical tree must be refused");
    assert!(
        error.contains("outside this agent's writable directories"),
        "the refusal must say why: {error}"
    );
}

/// Reading it stays legitimate. Confining writes must not blind the agent to
/// the tree it was branched from — it frequently has to read it to do the work.
#[test]
fn an_isolated_agent_may_still_read_the_canonical_tree() {
    let trees = trees();
    let ctx = isolated(&trees);
    let target = trees.canonical.join("lib.rs");

    super::resolve_existing_file_path(&target.display().to_string(), &ctx)
        .expect("reading the canonical tree must remain allowed");
}

/// Unconfined contexts are untouched. An interactive session adds a directory
/// with `/add-dir` because it intends to edit there, so an empty `write_roots`
/// must keep behaving exactly as before.
#[test]
fn an_unconfined_context_writes_wherever_it_reads() {
    let trees = trees();
    let ctx = unconfined(&trees);
    let target = trees.canonical.join("lib.rs");

    resolve_existing_write_target(&target.display().to_string(), &ctx)
        .expect("an unconfined context must be unchanged");
}

/// A write root that cannot be resolved is an error, not a silently skipped
/// entry — dropping it would widen the confinement it exists to impose.
#[test]
fn an_unresolvable_write_root_refuses_rather_than_widens() {
    let trees = trees();
    let mut ctx = isolated(&trees);
    ctx.write_roots = vec![std::path::PathBuf::from("/nonexistent/worktree")];
    let target = trees.worktree.join("lib.rs");

    let error = resolve_existing_write_target(&target.display().to_string(), &ctx)
        .expect_err("an unresolvable write root must not fail open");
    assert!(
        error.contains("Failed to resolve write root"),
        "the refusal must name the cause: {error}"
    );
}
