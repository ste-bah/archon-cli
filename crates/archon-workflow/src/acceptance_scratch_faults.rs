//! Whose an observation's operational error is.
//!
//! An observation ends a check with an operational error when it cannot
//! give a verdict. Some of these errors are caused by the check's own run:
//! its output or its files broke a limit of the site, a process it started
//! outlived its process tree, or it changed the tree it ran in. The host
//! cannot repair these; only the check's author can, so they are the
//! author's. A teardown the host could not verify may be the host's (for
//! example under memory pressure), so it is ambiguous. Every other error is
//! the host's.

/// The check's output went past the site's output limit.
pub const OUTPUT_LIMIT_EXCEEDED: &str = "native acceptance output limit exceeded";
/// What the check wrote went past the site's scratch size limit.
pub const SCRATCH_LIMIT_EXCEEDED: &str = "native acceptance scratch limit exceeded";
/// A process the check started kept its output pipes open after teardown.
pub const PIPES_HELD: &str = "scratch child output pipes stayed open after teardown: a process outside the check's process tree still holds them";
/// Processes the check started still used the scratch after its tree ended.
pub const LEFT_BEHIND: &str = "processes the check left behind still use the scratch";
/// The host could not verify that the check's process tree was gone.
pub const TEARDOWN_UNVERIFIED: &str = "scratch process tree teardown could not be verified";
/// The tree or build layout the check ran in changed while it ran.
pub const IDENTITY_CHANGED: &str = "native scratch source or build identity changed";
/// The check replaced the build output mapping of its tree.
pub const TARGET_MAPPING_CHANGED: &str = "native build identity changed: target mapping";
/// The check names a sibling path that resolves to the project root or an
/// ancestor of it, which no site copies.
pub const SIBLING_ROOT_REFUSED: &str =
    "the check names a sibling path that resolves to the project root or an ancestor of it";
/// The check alone names a sibling path whose copy is larger than the
/// site's scratch size limit.
pub const SIBLING_TOO_LARGE: &str =
    "the check names a sibling path whose copy exceeds the site's scratch size limit";
/// The check alone names a sibling path whose shape the site cannot hold:
/// a path that is not a directory where one is needed, or one that
/// collides with a path the site already has.
pub const SIBLING_SHAPE_REFUSED: &str =
    "the check names a sibling path whose shape the site cannot copy";

/// Parts of the build identity that only the check's own run can change:
/// the host's tools and environment are not among them.
const CHECK_PARTS: [&str; 4] = ["source", "target", "target links", "build configuration"];

/// Whose an operational error is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckFault {
    /// The check's own run caused it: what the check did, and what it must
    /// do instead.
    Check {
        trigger: &'static str,
        remedy: &'static str,
    },
    /// It may be the check's or the host's.
    Ambiguous,
    /// The host's.
    Host,
}

/// The text of an identity change of `parts` (see [`check_fault`]).
pub(super) fn identity_changed(parts: &[&str]) -> String {
    format!(
        "{IDENTITY_CHANGED} ({}); warm target reuse refused",
        parts.join(", ")
    )
}

/// Whose the operational error `error` is.
pub fn check_fault(error: &str) -> CheckFault {
    const STOP: &str = "wait for every process it starts and leave none running when it ends";
    const TREE: &str = "write only under a temporary directory; never change the tracked sources, the build output directory or the build configuration of the tree it runs in";
    if error.starts_with(OUTPUT_LIMIT_EXCEEDED) {
        return CheckFault::Check {
            trigger: "its output went past the site's output limit",
            remedy: "print only what decides its verdict, and write bulky output to a file under a temporary directory",
        };
    }
    if error.starts_with(SCRATCH_LIMIT_EXCEEDED) {
        return CheckFault::Check {
            trigger: "the files it wrote went past the site's scratch size limit",
            remedy: "write less data, and remove what it no longer needs while it runs",
        };
    }
    if error.starts_with(PIPES_HELD) {
        return CheckFault::Check {
            trigger: "a process it started kept its output pipes open after it ended",
            remedy: STOP,
        };
    }
    if error.starts_with(LEFT_BEHIND) {
        return CheckFault::Check {
            trigger: "processes it started still used the scratch after its process tree ended",
            remedy: STOP,
        };
    }
    if error.starts_with(SIBLING_ROOT_REFUSED) {
        return CheckFault::Check {
            trigger: "it names a path outside its working directory that resolves to the project root or a directory above it, which the site never copies",
            remedy: "name only paths under its own working directory or a sibling directory beside it, never the project root or a directory above it",
        };
    }
    if error.starts_with(SIBLING_TOO_LARGE) {
        return CheckFault::Check {
            trigger: "it names a path outside its working directory whose copy is larger than the site's scratch size limit",
            remedy: "name only the sibling files or directories it reads, and keep bulky data out of them",
        };
    }
    if error.starts_with(SIBLING_SHAPE_REFUSED) {
        return CheckFault::Check {
            trigger: "it names a path outside its working directory that is not a directory where one is needed, or that collides with a path of the site",
            remedy: "name only existing sibling files or directories beside its working directory, by the path they have",
        };
    }
    if error.contains(TARGET_MAPPING_CHANGED) {
        return CheckFault::Check {
            trigger: "it replaced the build output directory of the tree it ran in",
            remedy: TREE,
        };
    }
    if let Some(rest) = error.strip_prefix(IDENTITY_CHANGED) {
        let parts = rest
            .strip_prefix(" (")
            .and_then(|rest| rest.split_once(')'))
            .map(|(parts, _)| parts.split(", ").collect::<Vec<_>>())
            .unwrap_or_default();
        if !parts.is_empty() && parts.iter().all(|part| CHECK_PARTS.contains(part)) {
            return CheckFault::Check {
                trigger: "it changed the source tree or build layout it ran in",
                remedy: TREE,
            };
        }
        return CheckFault::Host;
    }
    if error.starts_with(TEARDOWN_UNVERIFIED) {
        return CheckFault::Ambiguous;
    }
    CheckFault::Host
}

#[cfg(test)]
#[path = "acceptance_scratch_faults_tests.rs"]
mod tests;
