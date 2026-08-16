//! How far to isolate a subagent from its siblings (#184 M3).
//!
//! Non-isolated parallel agents share one working tree with last-write-wins.
//! The obvious remedy — a worktree each — is wrong as a default on a repo like
//! this one, and the reason is disk rather than correctness.
//!
//! A worktree shares `.git` and checks out working files only: a couple of
//! hundred megabytes, under a second. What costs is an agent **building**
//! inside one, because a fresh `target/` on this workspace runs to ~10GB and
//! several minutes. Isolating every parallel writer by default would trade an
//! invisible conflict for an invisible disk fire.
//!
//! So isolation is a ladder, and the rung is chosen from what the agent is
//! actually going to do:
//!
//! | Tier | What it gets | Disk |
//! |---|---|---|
//! | [`Shared`] | the working tree, plus M2's write-intent claims | none |
//! | [`Worktree`] | its own checkout; build and test commands refused | ~200MB |
//! | [`WorktreeWithBuilds`] | its own checkout and its own `target/` | full |
//!
//! Tier 1 (patch-based, edit-only agents producing diffs the parent applies)
//! and Tier 2 (a tool-layer overlay) are specified in the issue and not built
//! here; they slot in between `Shared` and `Worktree` without changing this
//! enum's ordering.
//!
//! ## Why Tier 3 refuses builds rather than discouraging them
//!
//! "Discouraged" is a comment, not a control. One `cargo check` from a Tier 3
//! agent creates the cold `target/` the tier exists to avoid, and nothing would
//! report it. Refusing, with a named escape hatch to [`WorktreeWithBuilds`],
//! keeps the cost visible and the decision with the operator — silent
//! promotion would reintroduce the 10GB invisibly, which is the whole failure
//! this ladder is built around.

use serde::{Deserialize, Serialize};

/// How isolated a subagent is from its siblings.
///
/// Ordered by cost: `Shared < Worktree < WorktreeWithBuilds`. The ordering is
/// load-bearing — `isolation_max_tier` clamps against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationTier {
    /// The working tree everyone else is using. Conflicts are prevented by
    /// declaring intent (M2), not by separation.
    Shared,
    /// Its own checkout. Build and test commands are refused: verification runs
    /// once, post-merge, in the main tree.
    Worktree,
    /// Its own checkout and its own build directory. The expensive rung, taken
    /// only when an agent genuinely must build before its work can be merged.
    WorktreeWithBuilds,
}

impl IsolationTier {
    /// Whether this tier gets a git worktree of its own.
    pub fn needs_worktree(self) -> bool {
        matches!(self, Self::Worktree | Self::WorktreeWithBuilds)
    }

    /// Whether an agent at this tier may run build or test commands.
    pub fn may_build(self) -> bool {
        matches!(self, Self::Shared | Self::WorktreeWithBuilds)
    }

    /// The `isolation` string an `Agent` call uses to ask for this tier.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shared => "none",
            Self::Worktree => "worktree",
            Self::WorktreeWithBuilds => "worktree-with-builds",
        }
    }

    /// Parse an explicit `isolation` argument.
    ///
    /// `None` for anything unrecognised, so the caller decides between refusing
    /// and falling back rather than silently getting a tier it did not ask for.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "none" | "shared" => Some(Self::Shared),
            "worktree" => Some(Self::Worktree),
            "worktree-with-builds" => Some(Self::WorktreeWithBuilds),
            _ => None,
        }
    }
}

/// When to isolate an agent that did not ask to be isolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoIsolation {
    /// Never. Every agent shares the tree unless it says otherwise.
    Off,
    /// Only when this agent's declared writes overlap a running agent's.
    ///
    /// The default, and the only trigger: isolation costs disk, and disjoint
    /// writers do not need it.
    #[default]
    Overlap,
    /// Every write-capable agent, whether or not anything overlaps.
    Always,
}

/// What the caller asked for and what the situation demands.
#[derive(Debug, Clone)]
pub struct IsolationRequest {
    /// The `isolation` argument on the `Agent` call, if it gave one.
    pub explicit: Option<String>,
    /// Whether this agent's declared writes overlap a running agent's (M2).
    pub overlaps_live_claim: bool,
    /// Whether this agent can write at all. Read-only agents never need
    /// isolating, whatever else is true.
    pub write_capable: bool,
}

/// Why an agent ended up at the tier it did — carried so the spawn result can
/// say, rather than isolating silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsolationReason {
    /// The call named it.
    Requested,
    /// Declared writes overlapped a running agent's.
    OverlappingClaim,
    /// Policy isolates every writer.
    PolicyAlways,
    /// Nothing demanded isolation.
    Default,
    /// A higher tier was asked for or implied, and policy capped it.
    Clamped(IsolationTier),
}

/// Decide the tier for one spawn.
///
/// An explicit request wins, subject to the cap — a caller that names a tier
/// gets it or is told it was clamped, never quietly downgraded without a
/// reason. Otherwise the tier is `Shared` unless policy or an overlap says
/// otherwise.
pub fn resolve_tier(
    request: &IsolationRequest,
    auto: AutoIsolation,
    max_tier: IsolationTier,
) -> (IsolationTier, IsolationReason) {
    let (wanted, reason) = match request.explicit.as_deref().and_then(IsolationTier::parse) {
        Some(tier) => (tier, IsolationReason::Requested),
        None if !request.write_capable => (IsolationTier::Shared, IsolationReason::Default),
        None => match auto {
            AutoIsolation::Always => (IsolationTier::Worktree, IsolationReason::PolicyAlways),
            AutoIsolation::Overlap if request.overlaps_live_claim => {
                (IsolationTier::Worktree, IsolationReason::OverlappingClaim)
            }
            _ => (IsolationTier::Shared, IsolationReason::Default),
        },
    };

    if wanted > max_tier {
        return (max_tier, IsolationReason::Clamped(wanted));
    }
    (wanted, reason)
}

/// Whether a shell command builds or tests, and which part of it does.
///
/// Deliberately keyed on the **command text**, not the working directory. A
/// worktree-isolated agent still has the main checkout in scope through
/// `extra_dirs`, so it can `cd` out of its worktree inside a single bash
/// invocation; a gate that trusted the cwd would be walked around in one line.
///
/// `archon-world-model` has a richer classifier, but it is private, it lives in
/// a crate `archon-tools` does not depend on, and its public entry point folds
/// verification into a task class where `rm -rf target` outranks it. This is
/// the narrow question — does this line compile, test or lint — answered where
/// it is asked.
///
/// Errs toward refusing: a chained command is refused if **any** segment
/// builds, because `ls && cargo build` builds.
pub fn build_command_in(command: &str) -> Option<String> {
    command
        .split(['\n', ';'])
        .flat_map(|line| line.split("&&"))
        .flat_map(|line| line.split("||"))
        .flat_map(|line| line.split('|'))
        .map(str::trim)
        .find(|segment| segment_builds(segment))
        .map(|segment| segment.to_string())
}

/// Tools whose every invocation compiles, tests or lints.
const ALWAYS_BUILDS: &[&str] = &["make", "pytest", "tox", "gradle", "mvn", "ninja", "bazel"];

/// Package managers where the *subcommand* decides.
const PACKAGE_MANAGERS: &[&str] = &["cargo", "npm", "pnpm", "yarn", "bun", "go", "dotnet"];

/// Subcommands of those managers that build, test or lint.
const BUILDING_SUBCOMMANDS: &[&str] = &[
    "build",
    "test",
    "check",
    "clippy",
    "bench",
    "run",
    "install",
    "lint",
    "typecheck",
    "tsc",
    "compile",
    "doc",
    "publish",
    "package",
];

fn segment_builds(segment: &str) -> bool {
    let mut tokens = segment
        .split_whitespace()
        // Leading `VAR=value` assignments and common wrappers are not the
        // command; skipping them stops `RUSTFLAGS=… cargo build` reading as
        // an unknown tool.
        .skip_while(|token| token.contains('=') || matches!(*token, "env" | "time" | "nice"));

    let Some(program) = tokens.next() else {
        return false;
    };
    // `/usr/bin/cargo` and `cargo.exe` are still cargo.
    let program = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .trim_end_matches(".exe");

    if ALWAYS_BUILDS.contains(&program) {
        return true;
    }
    if !PACKAGE_MANAGERS.contains(&program) {
        return false;
    }

    // The first non-flag token is the subcommand. `npm run build` needs two.
    tokens
        .filter(|token| !token.starts_with('-'))
        .take(2)
        .any(|token| BUILDING_SUBCOMMANDS.contains(&token))
}

/// The message an agent gets when it tries to build at [`IsolationTier::Worktree`].
///
/// Names both ways out, because a refusal the model cannot act on just becomes
/// a retry loop.
pub fn build_refusal(command: &str) -> String {
    format!(
        "Refused: `{command}` builds or tests, and this agent is isolated in a worktree \
         without its own build directory.\n\
         Building here would create a fresh target/ that costs gigabytes and is thrown away \
         at merge.\n\
         Either finish edit-only and let verification run once after merge, or respawn with \
         isolation \"worktree-with-builds\" if this agent genuinely must build before its work \
         can be reviewed."
    )
}

#[cfg(test)]
#[path = "isolation_tests.rs"]
mod tests;
