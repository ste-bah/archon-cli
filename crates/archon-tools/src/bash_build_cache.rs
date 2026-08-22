//! Giving one isolated agent a build cache it can reuse.
//!
//! Split out of `bash_process` to hold the 500-line ceiling; the reasoning for
//! leasing rather than naming a directory per agent lives in
//! `build_cache_lease`, and the reasoning for which variables get set lives in
//! `build_cache_env`.

use crate::bash::BashTool;
use crate::build_cache_lease::BuildCacheLease;
use crate::tool::ToolContext;

/// Apply the build-cache environment for this call, returning the lease to hold
/// for as long as the command runs.
pub(super) async fn apply_build_cache(
    env_vars: &mut Vec<(String, String)>,
    tool: &BashTool,
    ctx: &ToolContext,
) -> Option<BuildCacheLease> {
    // An agent allowed to build inside its own worktree builds into a directory
    // beside it, never inside it: the worktree is removed wholesale, so a build
    // directory within it would make "remove the tree" and "discard the build"
    // the same irreversible step — and a KEPT worktree would keep the gigabytes
    // invisibly with it (#184 M3).
    //
    // The directory is LEASED from a pool rather than named after the agent.
    // Naming it after the agent made it die with the agent, so every task paid
    // a cold build even when no two tasks ever ran at once — observed live as
    // fifteen sequential tasks each rebuilding the same 17 GB of dependencies.
    // A leased slot is exclusive while held, which is the isolation a build
    // cache genuinely requires, and persists after release, which is what makes
    // the next occupant's build incremental.
    //
    // Which variables get set depends on what the repository is built with.
    // This engine implements whatever a PRD describes, so nothing here assumes
    // a language: `cache_env_for_repository` reads the repository's own
    // toolchain markers and sets only those toolchains' variables.
    if ctx.subagent_id.is_some()
        && tool.isolation_tier == crate::isolation::IsolationTier::WorktreeWithBuilds
    {
        match tool.build_cache_pool.as_ref() {
            Some(pool) => match pool.acquire().await {
                Ok(lease) => {
                    // The agent's working directory IS its checkout root, so
                    // the toolchain markers are read from where it actually
                    // builds rather than from a repository root it may not be
                    // standing in.
                    for (key, value) in crate::build_cache_env::cache_env_for_repository(
                        ctx.working_dir.as_path(),
                        lease.dir(),
                        &tool.build_cache_env_keys,
                    ) {
                        crate::bash::bash_env::set_env_override(env_vars, &key, &value);
                    }
                    // The wrapper caches across slots, so it covers what a
                    // leased directory cannot: the first agent onto a cold slot
                    // and agents holding different slots.
                    for (key, value) in crate::build_cache_env::compiler_cache_env_for_repository(
                        ctx.working_dir.as_path(),
                        &tool.compiler_cache_wrapper,
                    ) {
                        crate::bash::bash_env::set_env_override(env_vars, &key, &value);
                    }
                    Some(lease)
                }
                Err(error) => {
                    // A pool that cannot hand out a directory is a reason to
                    // build in the default location, not a reason to fail the
                    // command: the agent's work is still valid, it is only
                    // slower and less tidy.
                    tracing::warn!(%error, "bash: build cache lease unavailable");
                    None
                }
            },
            None => None,
        }
    } else {
        None
    }
}
