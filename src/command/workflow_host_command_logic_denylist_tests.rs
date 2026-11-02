//! Issue 361: the walk is sound and every denylist entry and exception is
//! reached (moved from `workflow_host_command_logic_guard_tests.rs`).
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::super::workflow_host_command_logic::LOGIC_DENYLIST;
use super::{REPOSITORY, walk_problems};

/// The walk is sound: every reference it meets resolves, and lands on a
/// hashed or a denied file. Every denylist entry is reached and says why.
/// `workflow_host_command_logic_walk_tests` proves this check can fail.
#[test]
fn logic_361_every_referenced_module_is_hashed_or_denied() {
    let mut reached = BTreeSet::new();
    for (id, hashed) in REPOSITORY.iter() {
        let problems = walk_problems(hashed);
        assert!(
            problems.is_empty(),
            "{id}: the logic walk is unsound (teach the walk, never ignore it):\n{}",
            problems.join("\n")
        );
        let closure = &hashed.closure;
        reached.extend(closure.denied.keys().cloned());
    }
    for (entry, reason) in LOGIC_DENYLIST {
        assert!(
            reason.len() > 20,
            "{entry}: a denylist entry needs its reason"
        );
        assert!(
            reached.contains(*entry),
            "{entry}: no capability reaches it; remove it"
        );
    }
}

/// Issue 288: every denylist exception sits under a denylist entry (or it
/// would not need one), says why, and is hashed by some capability.
#[test]
fn logic_361_every_denylist_exception_is_hashed() {
    use crate::command::workflow_host_command_logic_closure::LOGIC_DENYLIST_EXCEPTIONS;
    let hashed: BTreeSet<&Path> = REPOSITORY
        .values()
        .flat_map(|hashed| hashed.closure.files.iter().map(PathBuf::as_path))
        .collect();
    for (file, reason) in LOGIC_DENYLIST_EXCEPTIONS {
        assert!(reason.len() > 20, "{file}: an exception needs its reason");
        assert!(
            LOGIC_DENYLIST
                .iter()
                .any(|(entry, _)| entry.ends_with('/') && file.starts_with(entry)),
            "{file}: no denylist entry covers it; remove the exception"
        );
        assert!(
            hashed.contains(Path::new(file)),
            "{file}: no capability hashes it; remove the exception"
        );
    }
}
