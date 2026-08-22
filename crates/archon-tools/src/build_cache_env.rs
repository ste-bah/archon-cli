//! Where an isolated agent's build cache lives, for whatever toolchain it uses.
//!
//! An agent working in its own worktree must not share a build cache with an
//! agent working concurrently in another. Cargo is explicit that there is no
//! safe way to do it — two worktrees of one repository present path
//! dependencies with identical names, versions and relative paths, and a shared
//! `CARGO_TARGET_DIR` treats them as the same crate (rust-lang/cargo#12516),
//! so one agent's edited source silently links into another's build. Other
//! toolchains have their own versions of the same hazard. Isolation is the
//! only safe answer available.
//!
//! Isolation is not the same as a COLD cache, though, and that is what this
//! module exists to separate. A directory keyed to the agent is discarded with
//! it, so every agent — including agents that never run at the same time as
//! each other — rebuilds the world. Observed live: fifteen sequential tasks,
//! each paying a full 17 GB dependency build nothing else would ever reuse.
//!
//! So the directory is LEASED from a small pool instead. A lease is exclusive
//! for as long as an agent holds it, which preserves the isolation; the
//! directory survives the release, so the next agent to take that slot finds
//! the previous occupant's dependency artifacts already built. The pool is
//! sized to how many agents may write at once, which is also what bounds the
//! disk: slots × cache, not tasks × cache.
//!
//! # Not a Rust module
//!
//! Nothing here knows what the project is written in. A workflow implements
//! whatever the PRD describes, and the next one may be TypeScript or Go. What
//! this module holds is the shape shared by every toolchain — a cache
//! directory named by an environment variable — plus a table saying which
//! variables belong to which toolchain, and a marker file that says whether
//! the repository uses it at all. A repository with no recognised marker gets
//! no cache variables, which is correct: nothing is assumed on its behalf.

use std::path::{Path, PathBuf};

/// One toolchain's build-cache environment.
struct ToolchainCache {
    /// Repository-root file whose presence means this toolchain is in use.
    marker: &'static str,
    /// Environment variables that point the toolchain at its cache.
    vars: &'static [&'static str],
    /// Subdirectory of the lease, so two toolchains in one repository do not
    /// write over each other.
    subdir: &'static str,
}

/// Toolchains recognised without configuration.
///
/// Deliberately toolchain knowledge, never project knowledge: a marker file and
/// the cache variables its ecosystem documents. A project whose toolchain is
/// absent here declares its own variables in config rather than waiting for
/// this list to grow — see `extra_cache_env_keys`.
const TOOLCHAIN_CACHES: &[ToolchainCache] = &[
    ToolchainCache {
        marker: "Cargo.toml",
        vars: &["CARGO_TARGET_DIR"],
        subdir: "cargo",
    },
    ToolchainCache {
        marker: "go.mod",
        vars: &["GOCACHE", "GOMODCACHE"],
        subdir: "go",
    },
    ToolchainCache {
        marker: "package.json",
        vars: &["npm_config_cache", "YARN_CACHE_FOLDER"],
        subdir: "node",
    },
    ToolchainCache {
        marker: "pyproject.toml",
        vars: &["PIP_CACHE_DIR", "UV_CACHE_DIR"],
        subdir: "python",
    },
    ToolchainCache {
        marker: "requirements.txt",
        vars: &["PIP_CACHE_DIR"],
        subdir: "python",
    },
    ToolchainCache {
        marker: "build.gradle",
        vars: &["GRADLE_USER_HOME"],
        subdir: "gradle",
    },
    ToolchainCache {
        marker: "build.gradle.kts",
        vars: &["GRADLE_USER_HOME"],
        subdir: "gradle",
    },
    ToolchainCache {
        marker: "pom.xml",
        vars: &["MAVEN_OPTS_LOCAL_REPO"],
        subdir: "maven",
    },
];

/// Cache variables to set for a repository, given the directory a lease points
/// at.
///
/// Only toolchains whose marker file is actually present contribute. Returns
/// the pairs to apply; an empty result means this repository uses nothing this
/// engine recognises, and no cache variable should be invented for it.
pub fn cache_env_for_repository(
    repository_root: &Path,
    lease_dir: &Path,
    extra_cache_env_keys: &[String],
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for toolchain in TOOLCHAIN_CACHES {
        if !repository_root.join(toolchain.marker).exists() {
            continue;
        }
        let dir = lease_dir.join(toolchain.subdir);
        for var in toolchain.vars {
            if out.iter().any(|(name, _)| name == var) {
                continue;
            }
            out.push((var.to_string(), dir.to_string_lossy().into_owned()));
        }
    }
    // Project-declared variables land in their own subdirectory and are applied
    // whatever the repository looks like — a project naming one is asserting it
    // uses it, which is better evidence than a marker file.
    let extra_dir = lease_dir.join("project");
    for key in extra_cache_env_keys {
        let key = key.trim();
        if key.is_empty() || out.iter().any(|(name, _)| name == key) {
            continue;
        }
        out.push((key.to_string(), extra_dir.to_string_lossy().into_owned()));
    }
    out
}

/// Compiler-cache wrappers, and the variable each toolchain reads to use one.
///
/// A wrapper caches compiled output across directories, so it helps exactly
/// where a leased slot cannot: the first agent to touch a slot, and agents on
/// different slots. It is opt-in because it is not free — sccache does not
/// cache proc macros and wants incremental compilation off, so turning it on
/// blindly can cost more than it saves.
const COMPILER_CACHE_WRAPPERS: &[(&str, &str, &str)] = &[
    // (toolchain marker, variable naming the wrapper, wrapper executable)
    ("Cargo.toml", "RUSTC_WRAPPER", "sccache"),
    ("CMakeLists.txt", "CMAKE_C_COMPILER_LAUNCHER", "ccache"),
];

/// Wrapper variables to set, when the operator asked for a wrapper AND the
/// repository uses a toolchain that reads one.
///
/// `wrapper` is whatever the operator configured — an absolute path or a name
/// on PATH. Nothing is inferred: an empty setting means no wrapper, because a
/// build that silently routes through a tool nobody chose is harder to explain
/// than a slow one.
pub fn compiler_cache_env_for_repository(
    repository_root: &Path,
    wrapper: &str,
) -> Vec<(String, String)> {
    let wrapper = wrapper.trim();
    if wrapper.is_empty() {
        return Vec::new();
    }
    let name = Path::new(wrapper)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(wrapper);
    COMPILER_CACHE_WRAPPERS
        .iter()
        .filter(|(marker, _, expected)| *expected == name && repository_root.join(marker).exists())
        .map(|(_, var, _)| (var.to_string(), wrapper.to_string()))
        .collect()
}

/// The directory a lease slot owns.
///
/// Named by slot rather than by agent, which is the whole point: slot 0 is the
/// same directory for every agent that ever holds it, so its contents outlive
/// any one of them.
pub fn lease_slot_dir(pool_root: &Path, slot: usize) -> PathBuf {
    pool_root.join(format!("build-cache-{slot}"))
}

#[cfg(test)]
#[path = "build_cache_env_tests.rs"]
mod tests;
