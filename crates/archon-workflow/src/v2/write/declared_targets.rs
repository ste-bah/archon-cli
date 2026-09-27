//! A write branch's declared target set, stamped for the tool guard
//! (Issue-64).
//!
//! The set the ownership gates judge the branch's changes against is the
//! coordinator plan's `target_files` — the item's declaration AFTER
//! `worktree_wave_prepare::widen_to_obligations` added the baseline's
//! obligation files — plus the plan's directory scopes. That widened set,
//! not the raw declaration, is what the guard must refuse writes outside of:
//! an obligation file is one the coder was told to change.
//!
//! Stamped on the branch input under
//! [`crate::agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY`], the host
//! dispatch reads it back and scopes it for
//! `archon_tools::workflow_read_guard`, which refuses a Write/Edit/patch — or
//! a shell write naming its file — at a worktree path outside the set before
//! anything changes. A top-level key, like the forbidden-path stamp: host
//! built, rendered with the rest of the input (never authored), and in
//! `reuse_identity::VOLATILE_INPUT_KEYS` so it never moves the reuse hash.

use std::path::{Path, PathBuf};

use archon_write_plan::WritePlan;

use crate::agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY;

/// The repo-relative entries the guard is handed: every target file, and
/// every directory scope with a trailing `/`. Sorted and deduplicated.
pub(super) fn entries(plan: &WritePlan) -> Vec<String> {
    let mut entries: Vec<String> = plan
        .target_files
        .iter()
        .map(|path| path.as_str().to_string())
        .chain(
            plan.target_dir_scopes
                .iter()
                .map(|scope| format!("{}/", scope.as_str().trim_end_matches('/'))),
        )
        .filter(|entry| !entry.is_empty() && entry != "/")
        .collect();
    entries.sort();
    entries.dedup();
    entries
}

/// Stamp the set onto the branch input. Absent when the plan declares
/// nothing: the guard is then inert for the call.
pub(super) fn stamp(input: &mut serde_json::Value, plan: &WritePlan) {
    let entries = entries(plan);
    if entries.is_empty() {
        return;
    }
    if let Some(object) = input.as_object_mut() {
        object.insert(
            DECLARED_TARGETS_INPUT_KEY.to_string(),
            serde_json::json!(entries),
        );
    }
}

/// Issue-120: stamp what else the landing keeps, so the guard judges an
/// undeclared path as the ownership grant will (`worktree_scope_grant`):
/// kept when inside the plan's scope roots and claimed by no OTHER item of
/// `wave`. The same roots and claims the grant resolves from.
pub(super) fn stamp_grantable(
    input: &mut serde_json::Value,
    plan: &WritePlan,
    wave: &[crate::v2::write_scope_extension::WaveClaim],
) {
    let mut claimed: Vec<String> = wave
        .iter()
        .filter(|claim| claim.item_id != plan.item_id.as_str())
        .flat_map(|claim| claim.owned.iter().cloned())
        .collect();
    claimed.sort();
    claimed.dedup();
    if let Some(object) = input.as_object_mut() {
        object.insert(
            crate::agent_dispatch_port::GRANTABLE_SCOPE_INPUT_KEY.to_string(),
            serde_json::json!({
                "scope_roots": super::scope_roots::scope_roots(plan).entries(),
                "claimed": claimed,
            }),
        );
    }
}

/// Mark the branch as running in its own isolated item worktree, whose
/// `HEAD` is the landing's base (see
/// [`crate::agent_dispatch_port::ISOLATED_WORKTREE_INPUT_KEY`]).
pub(super) fn stamp_isolated(input: &mut serde_json::Value) {
    if let Some(object) = input.as_object_mut() {
        object.insert(
            crate::agent_dispatch_port::ISOLATED_WORKTREE_INPUT_KEY.to_string(),
            serde_json::Value::Bool(true),
        );
    }
}

/// Directory names toolchains keep dependencies and caches in. Toolchain
/// knowledge, never project knowledge — the same stance as
/// `archon_tools::build_cache_env` — so a gitignored data directory
/// (`data/`, `models/`, `.archon/`) is never among them.
const SHARED_TOOLCHAIN_DIRS: &[&str] = &[
    "node_modules",
    ".venv",
    "venv",
    "target",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    ".gradle",
];

/// Stamp the host's write boundary for an isolated branch (Issue-124): the
/// project root and the canonical checkout are sealed; the branch's declared
/// project artifacts stay writable, and so does each canonical toolchain
/// dependency or cache directory ([`SHARED_TOOLCHAIN_DIRS`]) whose children
/// the worktree shares by symlink — writes there are what they were before.
/// Any other shared directory stays sealed: a gitignored directory can hold
/// live data.
pub(super) fn stamp_write_boundary(
    input: &mut serde_json::Value,
    project_root: Option<&Path>,
    canonical_root: &Path,
    declared_artifacts: &[PathBuf],
    workspace: &crate::write_coordinator::worktree_isolation::ItemWorkspace,
) {
    let toolchain_dir = |entry: &str| {
        Path::new(entry.trim_end_matches('/'))
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| SHARED_TOOLCHAIN_DIRS.contains(&name))
    };
    let shared = workspace
        .materialized_ignored
        .materialized
        .iter()
        .filter(|(_, mechanism)| {
            *mechanism == crate::write_coordinator::worktree_isolation::Mechanism::SharedDirectory
        })
        .filter(|(entry, _)| toolchain_dir(entry))
        .map(|(entry, _)| canonical_root.join(entry.trim_end_matches('/')));
    let writable: Vec<String> = declared_artifacts
        .iter()
        .cloned()
        .chain(shared)
        .map(|path| path.display().to_string())
        .collect();
    let sealed: Vec<String> = project_root
        .into_iter()
        .chain(std::iter::once(canonical_root))
        .map(|path| path.display().to_string())
        .collect();
    if let Some(object) = input.as_object_mut() {
        object.insert(
            crate::agent_dispatch_port::WRITE_BOUNDARY_INPUT_KEY.to_string(),
            serde_json::json!({"sealed": sealed, "writable": writable}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_dispatch_port::declared_targets;

    #[test]
    fn the_write_boundary_seals_project_and_checkout_and_leaves_reuse_alone() {
        use crate::write_coordinator::worktree_isolation::{
            ItemWorkspace, MaterializedIgnored, Mechanism,
        };
        let project = Path::new("/work/project");
        let checkout = Path::new("/work/checkout");
        let workspace = ItemWorkspace {
            plan: plan(&["src/lib.rs"], &[]),
            baseline_commit: "base".into(),
            materialized_ignored: MaterializedIgnored {
                materialized: vec![
                    ("node_modules/".into(), Mechanism::SharedDirectory),
                    ("data/".into(), Mechanism::SharedDirectory),
                    (".archon/".into(), Mechanism::SharedDirectory),
                    (".env".into(), Mechanism::Copy),
                ],
                skipped: Vec::new(),
            },
        };
        let declared = [project.join(".archon/lab/reports/summary.json")];
        let mut input = serde_json::json!({"item": {"item_id": "a"}});
        let before = crate::v2::reuse_identity::reuse_input_hash(&input);
        stamp_write_boundary(&mut input, Some(project), checkout, &declared, &workspace);
        let (sealed, writable) =
            crate::agent_dispatch_port::write_boundary(&input).expect("stamped");
        assert_eq!(sealed, ["/work/project", "/work/checkout"]);
        assert_eq!(
            writable,
            [
                "/work/project/.archon/lab/reports/summary.json",
                "/work/checkout/node_modules"
            ]
        );
        assert_eq!(crate::v2::reuse_identity::reuse_input_hash(&input), before);

        // Only toolchain directories are re-opened, whether or not the
        // checkout is the project: a gitignored `data/` or `.archon/` can hold
        // live data and stays sealed.
        let mut input = serde_json::json!({});
        stamp_write_boundary(&mut input, Some(project), project, &[], &workspace);
        let (_, writable) = crate::agent_dispatch_port::write_boundary(&input).unwrap();
        assert_eq!(writable, ["/work/project/node_modules"]);
    }

    fn plan(files: &[&str], scopes: &[&str]) -> WritePlan {
        let root = std::path::Path::new("/repo");
        WritePlan {
            run_id: "run".into(),
            stage_id: "stage".into(),
            item_id: "item".into(),
            canonical_root: root.to_path_buf(),
            isolated_root: root.join("iso"),
            target_files: files
                .iter()
                .map(|f| archon_write_plan::normalize_target(f, root).unwrap())
                .collect(),
            target_dir_scopes: scopes
                .iter()
                .map(|f| archon_write_plan::normalize_target(f, root).unwrap())
                .collect(),
            target_files_source: archon_write_plan::TargetFilesSource::Item,
            read_context_files: Vec::new(),
            verify_inputs: Vec::new(),
            baseline_id: "git:HEAD".into(),
            workspace_boundary_required: true,
            resource_keys: Default::default(),
        }
    }

    #[test]
    fn the_stamp_carries_files_and_directory_scopes_and_reads_back_through_the_port() {
        let mut input = serde_json::json!({ "item": { "id": "x" } });
        stamp(
            &mut input,
            &plan(
                &["crates/engine/src/b.rs", "crates/engine/src/a.rs"],
                &["artifacts/runs"],
            ),
        );
        assert_eq!(
            declared_targets(&input),
            vec![
                "artifacts/runs/".to_string(),
                "crates/engine/src/a.rs".to_string(),
                "crates/engine/src/b.rs".to_string(),
            ]
        );
        // The item the agent is rendered is untouched.
        assert_eq!(input["item"], serde_json::json!({ "id": "x" }));
        let mut empty = serde_json::json!({ "item": {} });
        stamp(&mut empty, &plan(&[], &[]));
        assert!(declared_targets(&empty).is_empty());
        assert!(empty.get(DECLARED_TARGETS_INPUT_KEY).is_none());
    }

    /// Issue-120: the grant's roots and every OTHER item's claims are
    /// stamped beside the declared set, and read back through the port.
    #[test]
    fn the_grantable_stamp_carries_the_roots_and_only_the_other_items_claims() {
        use crate::v2::write_scope_extension::WaveClaim;
        let mut input = serde_json::json!({ "item": { "id": "x" } });
        let own = plan(&["src/engine/a.rs"], &["src/engine/a"]);
        let wave = [
            WaveClaim::new("item", ["src/engine/a.rs".to_string()]),
            WaveClaim::new(
                "sibling",
                ["src/engine/b.rs".to_string(), "src/engine/b".to_string()],
            ),
        ];
        stamp_grantable(&mut input, &own, &wave);
        let (roots, claimed) = crate::agent_dispatch_port::grantable_scope(&input).unwrap();
        assert_eq!(roots, ["src/".to_string()]);
        assert_eq!(claimed, ["src/engine/b", "src/engine/b.rs"]);
        assert_eq!(input["item"], serde_json::json!({ "id": "x" }));
        assert!(crate::agent_dispatch_port::grantable_scope(&serde_json::json!({})).is_none());
    }
}
