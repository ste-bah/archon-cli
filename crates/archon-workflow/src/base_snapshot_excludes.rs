use super::{BaseSnapshot, SnapshotEntry, UnverifiedReason, key_path, outside_key};
use crate::repository_record::normalize_relative;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub fn snapshot_excludes(
    repository: &Path,
    project: &Path,
    combined: bool,
    snapshot: &BaseSnapshot,
) -> Vec<PathBuf> {
    let prefix = project
        .strip_prefix(repository)
        .ok()
        .map(|p| normalize_relative(&p.to_string_lossy()));
    let policy = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["config", "--bool", "core.ignorecase"])
        .output();
    let insensitive = match policy {
        Ok(output) if output.status.success() => output.stdout.trim_ascii() != b"false",
        // Exit 1 with no output: the key is unset, and git's default is false.
        Ok(output) if output.status.code() == Some(1) && output.stdout.is_empty() => false,
        result => {
            eprintln!("snapshot exclusions use conservative case matching: {result:?}");
            true
        }
    };
    let equal = |a: &str, b: &str| a == b || insensitive && a.eq_ignore_ascii_case(b);
    let mut excludes = BTreeSet::new();
    for (key, entry) in &snapshot.entries {
        if outside_key(key)
            || matches!(
                entry,
                SnapshotEntry::Unverified {
                    reason: UnverifiedReason::CaseMismatch,
                    ..
                }
            )
        {
            continue;
        }
        excludes.insert(key_path(key));
        if !combined && let Some(prefix) = &prefix {
            let parts: Vec<_> = key.split('/').collect();
            let parents: Vec<_> = prefix.split('/').filter(|p| !p.is_empty()).collect();
            if parts.len() >= parents.len() && parts.iter().zip(&parents).all(|(a, b)| equal(a, b))
            {
                excludes.insert(key_path(&parts[parents.len()..].join("/")));
            }
            if !matches!(entry, SnapshotEntry::Tracked { .. })
                && parts.len() <= parents.len()
                && parts.iter().zip(&parents).all(|(a, b)| equal(a, b))
            {
                excludes.insert(PathBuf::new());
            }
        }
    }
    if insensitive {
        for exclude in excludes.clone() {
            let mut candidates = vec![PathBuf::new()];
            for component in exclude.components() {
                let mut next = Vec::new();
                for relative in candidates {
                    match std::fs::read_dir(project.join(&relative)) {
                        Ok(items) => {
                            for item in items {
                                let item = match item {
                                    Ok(item) => item,
                                    Err(error) => {
                                        eprintln!(
                                            "snapshot exclusions cover {} after directory entry failure: {error}",
                                            relative.display()
                                        );
                                        excludes.insert(relative.clone());
                                        continue;
                                    }
                                };
                                if item
                                    .file_name()
                                    .to_string_lossy()
                                    .eq_ignore_ascii_case(&component.as_os_str().to_string_lossy())
                                {
                                    next.push(relative.join(item.file_name()));
                                }
                            }
                        }
                        Err(error) => {
                            eprintln!(
                                "snapshot exclusions cover {} after directory read failure: {error}",
                                relative.display()
                            );
                            excludes.insert(relative);
                        }
                    }
                }
                candidates = next;
            }
            excludes.extend(candidates);
        }
    }
    excludes.into_iter().collect()
}
