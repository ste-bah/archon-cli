//! Symbolic links must not carry a confined write out of its roots.
//!
//! Every test builds a real tree with real links: a symlink guard asserted
//! against a mocked filesystem proves only that the mock agrees with itself,
//! and the defect being guarded is precisely that the kernel disagrees with a
//! path comparison.

use std::path::PathBuf;

use crate::path_guard::{resolve_existing_write_target, resolve_write_target_path};
use crate::tool::ToolContext;

struct Trees {
    _root: tempfile::TempDir,
    /// Where the agent may write.
    workspace: PathBuf,
    /// Somewhere it may not.
    outside: PathBuf,
}

fn trees() -> Trees {
    let root = tempfile::tempdir().expect("tempdir");
    let workspace = root.path().join("workspace");
    let outside = root.path().join("outside");
    std::fs::create_dir_all(workspace.join("src")).expect("workspace/src");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(workspace.join("src/lib.rs"), "// workspace\n").expect("seed workspace");
    std::fs::write(outside.join("secret.rs"), "// outside\n").expect("seed outside");
    Trees {
        _root: root,
        workspace: std::fs::canonicalize(&workspace).expect("canonicalize workspace"),
        outside: std::fs::canonicalize(&outside).expect("canonicalize outside"),
    }
}

/// Confined to the workspace, with the outside tree readable — the shape a
/// workflow agent actually gets.
fn confined(trees: &Trees) -> ToolContext {
    ToolContext {
        working_dir: trees.workspace.clone(),
        extra_dirs: vec![trees.outside.clone()],
        write_roots: vec![trees.workspace.clone()],
        ..ToolContext::default()
    }
}

fn unconfined(trees: &Trees) -> ToolContext {
    ToolContext {
        working_dir: trees.workspace.clone(),
        extra_dirs: vec![trees.outside.clone()],
        ..ToolContext::default()
    }
}

#[cfg(unix)]
fn link(original: &std::path::Path, at: &std::path::Path) {
    std::os::unix::fs::symlink(original, at).expect("symlink");
}

#[cfg(windows)]
fn link(original: &std::path::Path, at: &std::path::Path) {
    if original.is_dir() {
        std::os::windows::fs::symlink_dir(original, at).expect("symlink dir");
    } else {
        std::os::windows::fs::symlink_file(original, at).expect("symlink file");
    }
}

/// The baseline every other test is measured against: an ordinary write to a
/// permitted target still succeeds.
#[test]
fn a_confined_agent_writes_its_own_workspace() {
    let trees = trees();
    let ctx = confined(&trees);

    resolve_write_target_path(
        &trees.workspace.join("src/new.rs").display().to_string(),
        &ctx,
    )
    .expect("a new file inside the workspace must be permitted");
    resolve_existing_write_target(
        &trees.workspace.join("src/lib.rs").display().to_string(),
        &ctx,
    )
    .expect("an existing file inside the workspace must be permitted");
}

/// Traversal, which the containment check alone already answers. Kept because
/// the symlink work must not regress it.
#[test]
fn a_traversal_out_of_the_workspace_is_refused_with_a_readable_message() {
    let trees = trees();
    let ctx = confined(&trees);
    let escape = trees.workspace.join("src/../../outside/secret.rs");

    let error = resolve_existing_write_target(&escape.display().to_string(), &ctx)
        .expect_err("`..` out of the workspace must be refused");
    assert!(
        error.contains("outside this agent's writable directories"),
        "the refusal must say what rule was broken: {error}"
    );
    assert!(
        error.contains(&trees.workspace.display().to_string()),
        "the refusal must name where the agent MAY write: {error}"
    );
}

/// Point one: the target is itself a link out. `canonicalize` follows it, so
/// containment alone sees only the destination — which is why the link is
/// refused before the containment answer is trusted.
#[test]
fn a_symlink_target_pointing_outside_is_refused() {
    let trees = trees();
    let ctx = confined(&trees);
    let planted = trees.workspace.join("src/escape.rs");
    link(&trees.outside.join("secret.rs"), &planted);

    let error = resolve_existing_write_target(&planted.display().to_string(), &ctx)
        .expect_err("writing through a link out of the workspace must be refused");
    assert!(
        error.contains("symbolic link"),
        "the refusal must name the link as the reason: {error}"
    );
}

/// The same link judged only by where it points would pass containment, because
/// it does not point outside. It is still refused: the destination of a link is
/// data, and a check that passed today is not a promise about the write.
#[test]
fn a_symlink_target_pointing_inside_is_refused_too() {
    let trees = trees();
    let ctx = confined(&trees);
    let planted = trees.workspace.join("src/alias.rs");
    link(&trees.workspace.join("src/lib.rs"), &planted);

    let error = resolve_existing_write_target(&planted.display().to_string(), &ctx)
        .expect_err("a link is refused for being a link, not for where it points");
    assert!(
        error.contains("symbolic link"),
        "the refusal must name the link: {error}"
    );
}

/// Point two: a PARENT component is the link, and the target does not exist
/// yet. This is the case a single check on the target misses entirely — there
/// is no target to check.
#[test]
fn a_symlinked_parent_directory_is_refused() {
    let trees = trees();
    let ctx = confined(&trees);
    let planted = trees.workspace.join("out");
    link(&trees.outside, &planted);

    let error = resolve_write_target_path(&planted.join("new.rs").display().to_string(), &ctx)
        .expect_err("a new file under a linked directory must be refused");
    assert!(
        error.contains("symbolic link") || error.contains("outside this agent's writable"),
        "the refusal must name a reason the agent can act on: {error}"
    );
}

/// A parent link pointing back INSIDE the workspace is refused for the same
/// reason as the inside-pointing target link, and this is the case that would
/// silently pass if the component walk located the root by string prefix: the
/// roots are canonical and the requested path is not.
#[test]
fn a_symlinked_parent_pointing_inside_is_refused() {
    let trees = trees();
    let ctx = confined(&trees);
    let planted = trees.workspace.join("mirror");
    link(&trees.workspace.join("src"), &planted);

    let error = resolve_write_target_path(&planted.join("new.rs").display().to_string(), &ctx)
        .expect_err("a linked directory inside the workspace is still a link");
    assert!(
        error.contains("symbolic link"),
        "the refusal must name the link: {error}"
    );
}

/// The mechanism is OFF where it should be off. An interactive session has no
/// write roots, and a symlink in a checkout is an ordinary thing to edit.
#[test]
fn an_unconfined_context_still_writes_through_symlinks() {
    let trees = trees();
    let ctx = unconfined(&trees);
    let planted = trees.workspace.join("src/alias.rs");
    link(&trees.workspace.join("src/lib.rs"), &planted);

    resolve_existing_write_target(&planted.display().to_string(), &ctx)
        .expect("no write roots means no confinement and no link policy");
}

/// And an unconfined context keeps writing outside its working directory
/// wherever it may read, which is the behaviour `/add-dir` exists to give.
#[test]
fn an_unconfined_context_writes_an_added_directory() {
    let trees = trees();
    let ctx = unconfined(&trees);

    resolve_existing_write_target(&trees.outside.join("secret.rs").display().to_string(), &ctx)
        .expect("an added directory must stay writable when nothing confines the agent");
}
