use std::io::Read;

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use super::tool_types::{FileSystemObservation, ObservedEntry};
use super::*;

impl Agent {
    pub(super) fn observe_filesystem_before_mutation(
        &self,
        effect: archon_tools::tool::WorkingTreeEffect,
    ) -> Result<Option<FileSystemObservation>, String> {
        effect
            .requires_filesystem_observation()
            .then(|| filesystem_observation(&self.config.working_dir, true))
            .transpose()
    }

    pub(super) fn changed_files_after_mutation(
        &self,
        before: &FileSystemObservation,
    ) -> Result<Vec<String>, String> {
        let after = filesystem_observation(&before.root, true)?;
        let mut changed = after
            .entries
            .iter()
            .filter(|(path, entry)| before.entries.get(*path) != Some(*entry))
            .map(|(path, _)| observed_relative_display(path))
            .collect::<Result<Vec<_>, _>>()?;
        changed.extend(
            before
                .entries
                .keys()
                .filter(|path| !after.entries.contains_key(*path))
                .map(|path| observed_relative_display(path))
                .collect::<Result<Vec<_>, _>>()?,
        );
        changed.sort();
        changed.dedup();
        Ok(changed)
    }
}

fn filesystem_observation(
    root: &std::path::Path,
    reject_unsafe_symlinks: bool,
) -> Result<FileSystemObservation, String> {
    let canonical_root = root.canonicalize().map_err(|error| {
        format!(
            "cannot resolve working directory {}: {error}",
            root.display()
        )
    })?;
    let mut entries = std::collections::BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| format!("cannot walk {}: {error}", root.display()))?;
        let path = entry.path();
        if path == root {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("cannot relativize {}: {error}", path.display()))?
            .to_path_buf();
        let file_type = entry.file_type();
        let observed = if file_type.is_file() {
            ObservedEntry::File(observed_file(path)?)
        } else if file_type.is_dir() {
            ObservedEntry::Directory
        } else if file_type.is_symlink() {
            let target = std::fs::read_link(path)
                .map_err(|error| format!("cannot read symlink {}: {error}", path.display()))?;
            if reject_unsafe_symlinks {
                validate_symlink_target(&canonical_root, path, &target)?;
            }
            ObservedEntry::Symlink(target)
        } else {
            continue;
        };
        entries.insert(relative, observed);
    }
    Ok(FileSystemObservation {
        root: root.to_path_buf(),
        entries,
    })
}

fn observed_relative_display(path: &std::path::Path) -> Result<String, String> {
    path.to_str()
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| {
            format!(
                "non-UTF-8 path cannot be durably recorded: {}",
                path.display()
            )
        })
}

fn validate_symlink_target(
    canonical_root: &std::path::Path,
    link: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), String> {
    let resolved = link
        .parent()
        .unwrap_or(canonical_root)
        .join(target)
        .canonicalize()
        .map_err(|error| format!("cannot resolve symlink {}: {error}", link.display()))?;
    if !resolved.starts_with(canonical_root) {
        return Err(format!(
            "symlink {} resolves outside working directory: {}",
            link.display(),
            resolved.display()
        ));
    }
    Ok(())
}

fn observed_file(path: &std::path::Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let bytes = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
