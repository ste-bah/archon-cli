//! Attacks on the evidence rules.
//!
//! The whole point of this stage is that a discovered scope is not merely
//! another guess. If the evidence bind is weak, the stage replaces one
//! prophecy with a more expensive one — so these push on the boundary between
//! "read it" and "invented it" rather than on the happy path.

use std::path::Path;

use super::scope_discovery::{DiscoveredScope, accepted_scope};

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    std::fs::write(dir.path().join("src/lib.rs"), "// lib\n").expect("lib");
    std::fs::write(dir.path().join("src/other.rs"), "// other\n").expect("other");
    dir
}

fn scope(declared: &[&str], read: &[&str]) -> DiscoveredScope {
    DiscoveredScope {
        declared: declared.iter().map(|s| (*s).to_string()).collect(),
        read: read.iter().map(|s| (*s).to_string()).collect(),
    }
}

/// A file the pass opened is evidenced.
#[test]
fn a_file_that_was_read_is_accepted() {
    let dir = repo();
    let accepted = accepted_scope(
        &scope(&["src/lib.rs"], &["src/lib.rs"]),
        &[],
        Some(dir.path()),
    );
    assert_eq!(accepted, Some(vec!["src/lib.rs".to_string()]));
}

/// THE attack. An existing file the pass never opened is exactly the guess this
/// stage exists to replace, and must not be laundered into evidence by the mere
/// fact that it exists.
#[test]
fn an_existing_file_that_was_never_read_is_rejected() {
    let dir = repo();
    let accepted = accepted_scope(
        &scope(&["src/lib.rs", "src/other.rs"], &["src/lib.rs"]),
        &[],
        Some(dir.path()),
    );
    assert_eq!(
        accepted,
        Some(vec!["src/lib.rs".to_string()]),
        "src/other.rs was claimed but never opened"
    );
}

/// The one case reading cannot cover: a file that does not exist yet, in a
/// directory that does.
#[test]
fn a_new_file_in_a_real_directory_is_accepted() {
    let dir = repo();
    let accepted = accepted_scope(&scope(&["src/new.rs"], &[]), &[], Some(dir.path()));
    assert_eq!(accepted, Some(vec!["src/new.rs".to_string()]));
}

/// A new file in a directory that does not exist is invention.
#[test]
fn a_new_file_in_an_imaginary_directory_is_rejected() {
    let dir = repo();
    let accepted = accepted_scope(&scope(&["nowhere/new.rs"], &[]), &[], Some(dir.path()));
    assert_eq!(accepted, None, "no evidence survives, so keep the guess");
}

/// Contract deliverables are not the pass's to drop: a live failure lost a
/// 455-line source file exactly this way.
#[test]
fn contract_deliverables_survive_whatever_the_pass_says() {
    let dir = repo();
    let accepted = accepted_scope(
        &scope(&["src/lib.rs"], &["src/lib.rs"]),
        &["src/contracted.rs".to_string()],
        Some(dir.path()),
    )
    .expect("some scope");
    assert!(accepted.contains(&"src/contracted.rs".to_string()));
}

/// Nothing evidenced means keep the guess. This stage is an optimisation, not
/// a gate: it must never narrow a scope to nothing.
#[test]
fn a_pass_with_no_evidence_keeps_the_guess() {
    let dir = repo();
    assert_eq!(
        accepted_scope(&scope(&[], &["src/lib.rs"]), &[], Some(dir.path())),
        None
    );
    assert_eq!(
        accepted_scope(&DiscoveredScope::default(), &[], Some(dir.path())),
        None
    );
}

/// Without a repository root there is nothing to check an unread path against,
/// so only genuinely-read files survive.
#[test]
fn without_a_repository_root_only_read_files_are_evidenced() {
    let accepted = accepted_scope(
        &scope(&["src/lib.rs", "src/new.rs"], &["src/lib.rs"]),
        &[],
        None,
    );
    assert_eq!(accepted, Some(vec!["src/lib.rs".to_string()]));
}

/// Traversal must not resolve to a real parent outside the repository.
#[test]
fn a_path_escaping_the_repository_is_rejected() {
    let dir = repo();
    let accepted = accepted_scope(&scope(&["../outside/new.rs"], &[]), &[], Some(dir.path()));
    assert_eq!(accepted, None);
}

/// THE traversal attack that actually bites. The earlier traversal test passed
/// because the escape target did not exist; `Path::starts_with` is LEXICAL, so
/// `root/../sibling` satisfies it on components alone. Give the escape a real
/// directory to land in and the containment check has to do actual work.
#[test]
fn a_path_escaping_into_a_real_sibling_directory_is_rejected() {
    let parent = tempfile::tempdir().expect("tempdir");
    let root = parent.path().join("repo");
    std::fs::create_dir_all(root.join("src")).expect("repo");
    let sibling = parent.path().join("sibling");
    std::fs::create_dir_all(&sibling).expect("sibling");

    let accepted = accepted_scope(
        &scope(&["../sibling/new.rs"], &[]),
        &[],
        Some(Path::new(&root)),
    );
    assert_eq!(
        accepted, None,
        "a path resolving outside the repository must never be granted"
    );
}

/// The same file, spelled three ways. A pass that reports an absolute path or
/// a backslash form has still READ that file, and calling it unevidenced would
/// discard real evidence over spelling.
#[test]
fn a_file_read_under_a_different_spelling_is_still_evidenced() {
    let dir = repo();
    let absolute = dir.path().join("src/lib.rs").to_string_lossy().to_string();

    let accepted = accepted_scope(
        &scope(&["src/lib.rs"], &[absolute.as_str()]),
        &[],
        Some(dir.path()),
    );
    assert_eq!(
        accepted,
        Some(vec!["src/lib.rs".to_string()]),
        "an absolute in-repo spelling is the same file"
    );

    let accepted = accepted_scope(
        &scope(&[r"src\lib.rs"], &["src/lib.rs"]),
        &[],
        Some(dir.path()),
    );
    assert_eq!(
        accepted,
        Some(vec!["src/lib.rs".to_string()]),
        "a backslash spelling is the same file"
    );
}

/// The accepted scope is emitted in one spelling, whatever went in, so the
/// planner is never handed two names for one file.
#[test]
fn the_accepted_scope_is_deduplicated_across_spellings() {
    let dir = repo();
    let absolute = dir.path().join("src/lib.rs").to_string_lossy().to_string();
    let accepted = accepted_scope(
        &scope(&["src/lib.rs", absolute.as_str()], &["src/lib.rs"]),
        &[],
        Some(dir.path()),
    );
    assert_eq!(accepted, Some(vec!["src/lib.rs".to_string()]));
}

/// Blank and whitespace declarations are not paths.
#[test]
fn blank_declarations_are_rejected() {
    let dir = repo();
    assert_eq!(
        accepted_scope(&scope(&["", "   "], &[]), &[], Some(dir.path())),
        None
    );
}

// ---------------------------------------------------------------------------
// The pass must be READ-ONLY. A discovery turn that could write would be a
// second, unplanned, unowned writer running before the wave plan exists.
// ---------------------------------------------------------------------------

use crate::{WorkflowV2FanoutItem, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2WriteMode};

fn write_branch() -> WorkflowV2FanoutItem {
    let mut call = WorkflowV2HostCall {
        id: "implementation-wave-1-impl-a".into(),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: Default::default(),
    };
    call.options.target_files = vec!["src/guessed.rs".into()];
    call.options.target_files_from_item = true;
    call.options
        .extra
        .insert("wave_claims".into(), serde_json::json!([{"item_id": "x"}]));
    WorkflowV2FanoutItem::read_only(
        "implementation-wave-1-impl-a".to_string(),
        "implementer".to_string(),
        call,
        serde_json::json!({"item": {"target_files": ["src/guessed.rs"]}}),
    )
}

#[test]
fn the_discovery_call_cannot_write() {
    let call = super::scope_discovery::scope_discovery_call(&write_branch());
    assert!(
        call.write_mode.is_none(),
        "write_mode drives is_write_capable"
    );
    assert_eq!(
        call.method,
        WorkflowV2HostMethod::Agent,
        "a branch call carries Implementation whether or not it can write"
    );
}

/// The pass must not inherit the very guess it exists to replace, nor the wave
/// context of a plan that does not exist yet.
#[test]
fn the_discovery_call_carries_no_guessed_scope_or_wave_context() {
    let call = super::scope_discovery::scope_discovery_call(&write_branch());
    assert!(call.options.target_files.is_empty());
    assert!(!call.options.target_files_from_item);
    assert!(!call.options.extra.contains_key("wave_claims"));
    assert!(!call.options.extra.contains_key("target_ownership_scopes"));
}

/// A distinct id, so the pass cannot collide with the branch's own result.
#[test]
fn the_discovery_call_has_its_own_id() {
    let call = super::scope_discovery::scope_discovery_call(&write_branch());
    assert_eq!(call.id, "implementation-wave-1-impl-a-scope-discovery");
}
