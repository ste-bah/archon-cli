//! Batch G2: the host roots no agent session may write, as ONE list.
//!
//! Issue-124 sealed the project root and the canonical checkout around an
//! isolated write branch; Batch G drew a stricter boundary around read-only
//! calls and sealed more there: the acceptance policy's roots (whose scratch
//! parent holds the observation evidence the host takes verdicts from), the
//! host's agent transcript store and its configuration. The two lists were
//! built in two crates and drifted: a write branch's shell could rewrite an
//! `observation.json` the host then read as an acceptance verdict.
//!
//! Every boundary the host draws (a write branch's `_write_boundary` stamp
//! and a read-only call's scope) seals exactly [`sealed_host_roots`]; each
//! then re-opens only what that kind of call may write (a write branch its
//! worktree, a read-only call nothing but host temp and toolchain dirs).

use std::path::{Path, PathBuf};

/// Every root the host owns for the run at `run_root`, absolute, sorted and
/// deduplicated:
///
/// - the project root and the canonical checkout the call was given;
/// - the run store itself;
/// - the run's recorded acceptance policy roots: its repository, its project
///   and its scratch parent, under which every acceptance scratch and every
///   evidence directory the host reads verdicts from is made;
/// - the host's agent transcript store and configuration
///   ([`user_host_stores`]).
///
/// A relative or empty entry is dropped: it would be judged against whatever
/// directory this process happens to be in.
pub fn sealed_host_roots(
    run_root: Option<&Path>,
    project_root: Option<&Path>,
    canonical_root: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = [project_root, canonical_root, run_root]
        .into_iter()
        .flatten()
        .map(Path::to_path_buf)
        .collect();
    if let Some(run_root) = run_root {
        roots.extend(recorded_policy_roots(run_root));
    }
    roots.extend(user_host_stores());
    roots.retain(|root| root.is_absolute() && root.parent().is_some());
    roots.sort();
    roots.dedup();
    roots
}

/// The repository, project and scratch-parent roots of the run's recorded
/// acceptance policy (`v2/generated-metadata.json`), when it recorded one.
pub fn recorded_policy_roots(run_root: &Path) -> Vec<PathBuf> {
    let Ok(bytes) = std::fs::read(run_root.join("v2/generated-metadata.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    let Some(policy) = value.pointer("/observer_snapshot/native_execution/policy") else {
        return Vec::new();
    };
    ["repository", "project", "scratch_parent"]
        .iter()
        .filter_map(|key| policy.get(*key).and_then(serde_json::Value::as_str))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect()
}

/// The host's own stores under the user's home: the agent transcript store
/// (`~/.archon/sessions`) and the host configuration
/// (`~/.archon/config.toml`). Empty when no home directory is known.
pub fn user_host_stores() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|home| !home.is_empty()))
        .map(PathBuf::from)
        .filter(|home| home.is_absolute());
    home.map(|home| {
        ["sessions", "config.toml"]
            .iter()
            .map(|store| home.join(".archon").join(store))
            .collect()
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_carries_every_host_root_once_and_drops_relative_entries() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let run = project.join(".archon/workflows/wf-x");
        let evidence = temp.path().join("observations");
        let policy = serde_json::json!({"observer_snapshot": {"native_execution": {"policy": {
            "repository": temp.path().join("checkout"), "project": project,
            "scratch_parent": evidence, "task_root": "relative/tasks",
        }}}});
        std::fs::create_dir_all(run.join("v2")).unwrap();
        std::fs::write(
            run.join("v2/generated-metadata.json"),
            serde_json::to_vec(&policy).unwrap(),
        )
        .unwrap();
        let roots = sealed_host_roots(
            Some(&run),
            Some(&project),
            Some(Path::new("relative/checkout")),
        );
        for expected in [&project, &run, &evidence, &temp.path().join("checkout")] {
            assert_eq!(
                roots.iter().filter(|r| r == &expected).count(),
                1,
                "{roots:?}"
            );
        }
        assert!(roots.iter().all(|root| root.is_absolute()));
        for store in user_host_stores() {
            assert!(roots.contains(&store), "{store:?} in {roots:?}");
        }
    }

    #[test]
    fn a_run_without_a_recorded_policy_still_seals_the_given_roots() {
        // An absolute path on either platform: a relative one is dropped by
        // `sealed_host_roots`, and `/p` is not absolute on Windows.
        let project: &Path = if cfg!(windows) {
            Path::new(r"C:\p")
        } else {
            Path::new("/p")
        };
        let missing: &Path = if cfg!(windows) {
            Path::new(r"C:\nonexistent\run")
        } else {
            Path::new("/nonexistent/run")
        };
        let roots = sealed_host_roots(None, Some(project), None);
        assert!(roots.contains(&project.to_path_buf()));
        assert!(recorded_policy_roots(missing).is_empty());
    }
}
