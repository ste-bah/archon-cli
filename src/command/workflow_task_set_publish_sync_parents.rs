use super::{Path, Result, sync_parent};

/// Flush each distinct parent directory of `paths` once.
pub(crate) fn sync_parents<'a>(paths: impl Iterator<Item = &'a Path>) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for path in paths {
        if seen.insert(path.parent().map(Path::to_path_buf)) {
            sync_parent(path)?;
        }
    }
    Ok(())
}
