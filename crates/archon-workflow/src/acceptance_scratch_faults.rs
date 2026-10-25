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
//!
//! Where the observation detects a fault itself, it records the cause on
//! the check's result as a typed [`FaultCause`], and [`classify`] judges
//! by that cause; the text is only the message. [`check_fault`] judges by
//! the text alone: the process layer's limits, and stored results that
//! carry no cause.
use serde::{Deserialize, Serialize};

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

/// Why a check's own run gave no verdict, recorded on its result where the
/// observation detected it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", rename_all = "snake_case")]
pub enum FaultCause {
    /// Its run changed these parts of the site's identity.
    IdentityChanged { parts: Vec<String> },
    /// Its run removed this part of the site, the repository's git
    /// metadata included.
    Removed { part: String },
    /// Its run replaced the build output mapping of its tree.
    TargetMappingChanged,
    /// It names a sibling path resolving to the project root or above.
    SiblingRootRefused,
    /// Its path leaves the selected root and returns through a site-owned name.
    SiteAliasRefused,
    /// It alone names a sibling whose copy exceeds the scratch limit.
    SiblingTooLarge,
    /// It alone names a sibling whose shape the site cannot copy.
    SiblingShapeRefused,
    /// The process layer observed an output limit violation.
    OutputLimitExceeded,
    /// The process layer observed a scratch size limit violation.
    ScratchLimitExceeded,
    /// A process started by the check kept a captured output pipe open.
    PipesHeld,
    /// A process started by the check still used the scratch after its tree ended.
    LeftBehind,
    /// The process layer could not verify teardown of the check's process tree.
    TeardownUnverified,
    /// The check left its private, verified site in a state the host could not remove.
    SiteTeardownFailed,
}

const STOP: &str = "wait for every process it starts and leave none running when it ends";
const TREE: &str = "write only under a temporary directory; never change the tracked sources, the build output directory or the build configuration of the tree it runs in";

impl FaultCause {
    /// Whose this cause is.
    pub fn fault(&self) -> CheckFault {
        let check = |trigger, remedy| CheckFault::Check { trigger, remedy };
        match self {
            Self::IdentityChanged { parts }
                if parts.is_empty() || !parts.iter().all(|p| CHECK_PARTS.contains(&p.as_str())) =>
            {
                CheckFault::Host
            }
            Self::IdentityChanged { .. } => {
                check("it changed the source tree or build layout it ran in", TREE)
            }
            Self::Removed { .. } => check(
                "it removed a part of the source tree or build layout it ran in",
                TREE,
            ),
            Self::TargetMappingChanged => check(
                "it replaced the build output directory of the tree it ran in",
                TREE,
            ),
            Self::SiblingRootRefused => check(
                "it names a path outside its working directory that resolves to the project root or a directory above it, which the site never copies",
                "name only paths under its own working directory or a sibling directory beside it, never the project root or a directory above it",
            ),
            Self::SiteAliasRefused => check(
                "it names a path that leaves its selected root and returns through a directory name the site uses for its own copy",
                "name only paths under its selected root or sibling directories whose names are not used by the site",
            ),
            Self::SiblingTooLarge => check(
                "it names a path outside its working directory whose copy is larger than the site's scratch size limit",
                "name only the sibling files or directories it reads, and keep bulky data out of them",
            ),
            Self::SiblingShapeRefused => check(
                "it names a path outside its working directory that is not a directory where one is needed, or that collides with a path of the site",
                "name only existing sibling files or directories beside its working directory, by the path they have",
            ),
            Self::OutputLimitExceeded => check(
                "its output went past the site's output limit",
                "print only what decides its verdict, and write bulky output to a file under a temporary directory",
            ),
            Self::ScratchLimitExceeded => check(
                "the files it wrote went past the site's scratch size limit",
                "write less data, and remove what it no longer needs while it runs",
            ),
            Self::PipesHeld => check(
                "a process it started kept its output pipes open after it ended",
                STOP,
            ),
            Self::LeftBehind => check(
                "processes it started still used the scratch after its process tree ended",
                STOP,
            ),
            Self::TeardownUnverified => CheckFault::Ambiguous,
            Self::SiteTeardownFailed => check(
                "permission was denied when removing a file from its scratch site (for example, one with an immutable flag)",
                "leave the scratch site removable when the check ends",
            ),
        }
    }
}

/// Whose the operational error `error` of a result recording `cause` is:
/// by the typed cause when the observation recorded one, else by the text.
pub fn classify(cause: Option<&FaultCause>, error: &str) -> CheckFault {
    cause.map_or_else(|| check_fault(error), FaultCause::fault)
}

/// The text of an identity change of `parts` (see [`check_fault`]).
pub(super) fn identity_changed(parts: &[&str]) -> String {
    format!(
        "{IDENTITY_CHANGED} ({}); warm target reuse refused",
        parts.join(", ")
    )
}

/// Whose the operational error `error` is, judged by its text alone (see
/// the module docs).
pub fn check_fault(error: &str) -> CheckFault {
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
    // A stored result written before causes were recorded.
    let legacy = [
        (SIBLING_ROOT_REFUSED, FaultCause::SiblingRootRefused),
        (SIBLING_TOO_LARGE, FaultCause::SiblingTooLarge),
        (SIBLING_SHAPE_REFUSED, FaultCause::SiblingShapeRefused),
    ];
    if let Some((_, cause)) = legacy.iter().find(|(text, _)| error.starts_with(text)) {
        return cause.fault();
    }
    if error.contains(TARGET_MAPPING_CHANGED) {
        return FaultCause::TargetMappingChanged.fault();
    }
    if let Some(rest) = error.strip_prefix(IDENTITY_CHANGED) {
        let parts = rest
            .strip_prefix(" (")
            .and_then(|rest| rest.split_once(')'))
            .map(|(parts, _)| parts.split(", ").map(str::to_string).collect())
            .unwrap_or_default();
        return FaultCause::IdentityChanged { parts }.fault();
    }
    if error.starts_with(TEARDOWN_UNVERIFIED) {
        return CheckFault::Ambiguous;
    }
    CheckFault::Host
}

#[cfg(test)]
#[path = "acceptance_scratch_faults_tests.rs"]
mod tests;
