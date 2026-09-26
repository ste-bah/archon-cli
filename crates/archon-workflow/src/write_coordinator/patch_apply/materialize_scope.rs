//! Where a declared ignored project artifact may be placed (Issue-113).
//!
//! An allowlist, twice over. A path is placed only when
//!
//! 1. the task universe declares it as a deliverable contract (host-parsed
//!    task files, never an agent's claim: `PatchManifest::materializable`),
//!    and
//! 2. it sits inside a namespace directory under `.archon/` that no engine
//!    code loads anything from: configuration, agents, skills, plugins,
//!    hooks, policy, prompts, credentials, state, or run records.
//!
//! [`ENGINE_LOADED`] is that second list, enumerated from the code: every
//! `.archon/<name>` the runtime sources name (string paths, `join(".archon")
//! .join(..)` chains and `[".archon", ..]` segments). A gate test re-scans
//! the sources and fails when a runtime `.archon/<name>` is neither listed
//! here nor a pure data store, so a directory the engine starts loading from
//! can never silently become writable.

use std::path::PathBuf;

/// Every top-level `.archon/` entry the engine loads from, lower-case. The
/// runtime reader of each is in the gate test's classification.
pub(crate) const ENGINE_LOADED: &[&str] = &[
    ".credentials.json",
    "agent-evolution",
    "agent-memory",
    "agent-memory-local",
    "agents",
    "archon-data.db",
    "archon.md",
    "artifacts",
    "audit-control-endpoint",
    "cassettes",
    "codex-tos-ack",
    "cognitive",
    "config.local.toml",
    "config.toml",
    "context.local.toml",
    "context.toml",
    "docs",
    "evidence",
    "hooks.local.toml",
    "hooks.toml",
    "kb",
    "large-edits",
    "leann.db",
    "learning-state.db",
    "learning.db",
    "lint-cache",
    "logs",
    "lsp-config.json",
    "models",
    "output-styles",
    "pipelines",
    "plugin-artifacts",
    "plugins",
    "policy",
    "policy.toml",
    "project.json",
    "reasoning-quality",
    "run",
    "runs",
    "scheduled_tasks.json",
    "self-calibration",
    "sessions",
    "settings.json",
    "skills",
    "specs",
    "spill",
    "teams",
    "test-fixtures",
    "tools",
    "video-artifacts",
    "web",
    "workflow-templates",
    "workflows",
    "worktrees",
    "world-model",
];

/// The project root verifiers are stamped against: the one
/// `project_artifact_context_from_v2_root` gives the host dispatch.
pub(super) fn project_root(run_root: &std::path::Path) -> Option<String> {
    crate::v2::project_artifacts::project_artifact_context_from_v2_root(&run_root.join("v2"))
        .project_root
        .filter(|root| !root.trim().is_empty())
}

/// Where the verifier reads `rel` -- the exact path the verification prompts
/// are stamped with -- when that is inside a namespace directory under
/// `.archon/` no engine code loads from. Compared case-blind, as the
/// filesystem may be; a hidden namespace is refused too.
pub(super) fn destination(project_root: &str, rel: &str) -> Option<PathBuf> {
    let absolute =
        crate::v2::project_artifact_stamping::project_artifact_destination(project_root, rel)?;
    let absolute = PathBuf::from(absolute);
    let inside: Vec<String> = absolute
        .strip_prefix(project_root)
        .ok()?
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    match inside.as_slice() {
        [archon, namespace, _, ..]
            if archon == ".archon"
                && !namespace.starts_with('.')
                && !ENGINE_LOADED.contains(&namespace.as_str()) =>
        {
            Some(absolute)
        }
        _ => None,
    }
}

/// The destination's state as a manifest records it: its content hash,
/// `absent`, or `<not a file>` for anything else there (a link included).
pub(super) fn destination_state(path: &std::path::Path) -> String {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent".to_string(),
        Ok(meta) if meta.is_file() => std::fs::read(path)
            .map(|bytes| blake3::hash(&bytes).to_hex().to_string())
            .unwrap_or_else(|_| "<unreadable>".to_string()),
        Ok(_) => "<not a file>".to_string(),
        Err(_) => "<unreadable>".to_string(),
    }
}

/// Each captured ignored target's destination state NOW, keyed by declared
/// path: the baseline a landing's copy must still find there
/// (`PatchManifest::destination_baselines`).
pub(crate) fn destination_baselines(
    run_root: &std::path::Path,
    ignored: &[(String, Vec<u8>)],
) -> std::collections::BTreeMap<String, String> {
    let Some(root) = project_root(run_root) else {
        return Default::default();
    };
    ignored
        .iter()
        .filter_map(|(rel, _)| {
            let destination = destination(&root, rel)?;
            Some((rel.clone(), destination_state(&destination)))
        })
        .collect()
}

#[cfg(test)]
#[path = "materialize_scope_tests.rs"]
mod tests;
