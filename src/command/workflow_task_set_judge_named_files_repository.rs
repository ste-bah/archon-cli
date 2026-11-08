use super::*;

pub(super) fn repository_regular_blob_size(
    repository: &Path,
    commit: &str,
    path: &str,
) -> Option<u64> {
    let entry = repository_tree_entry(repository, commit, path)?;
    if !(entry.starts_with(b"100644 blob ") || entry.starts_with(b"100755 blob ")) {
        return None;
    }
    std::str::from_utf8(&entry)
        .ok()?
        .split_whitespace()
        .nth(3)?
        .parse()
        .ok()
}

pub(super) fn repository_regular_blob(
    repository: &Path,
    commit: &str,
    path: &str,
) -> Option<Vec<u8>> {
    let entry = repository_tree_entry(repository, commit, path)?;
    // Git regular files are blobs with mode 100644 or 100755. In particular,
    // mode 120000 is a symlink blob and must never be followed or shown.
    if !(entry.starts_with(b"100644 blob ") || entry.starts_with(b"100755 blob ")) {
        return None;
    }
    #[cfg(test)]
    budget_tests::record_body_read();
    let output = archon_shell::spawn::git_command(repository)
        .args(["cat-file", "blob", &format!("{commit}:{path}")])
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

pub(super) fn repository_tree_entry(
    repository: &Path,
    commit: &str,
    path: &str,
) -> Option<Vec<u8>> {
    let tree_entry = archon_shell::spawn::git_command(repository)
        .args([
            "ls-tree",
            "-l",
            "-z",
            commit,
            "--",
            &format!(":(literal){path}"),
        ])
        .output()
        .ok()?;
    if !tree_entry.status.success() {
        return None;
    }
    let entry = tree_entry
        .stdout
        .split(|byte| *byte == 0)
        .find_map(|record| {
            let separator = record.iter().position(|byte| *byte == b'\t')?;
            let (metadata, entry_path_with_separator) = record.split_at(separator);
            let entry_path = entry_path_with_separator.get(1..)?;
            (entry_path == path.as_bytes()).then_some(metadata.to_vec())
        })?;
    Some(entry)
}
