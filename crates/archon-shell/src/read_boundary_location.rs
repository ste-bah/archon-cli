use std::path::{Component, Path, PathBuf};

fn resolved(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let spelled = super::path_spellings(&absolute)
        .pop()
        .unwrap_or_else(|| path.to_path_buf());
    let mut normalized = PathBuf::new();
    for part in spelled.components() {
        match part {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            part => normalized.push(part.as_os_str()),
        }
    }
    normalized
}

pub(super) fn inside_work(path: &Path) -> bool {
    let path = resolved(path);
    if path
        .ancestors()
        .any(|ancestor| ancestor.join(".git").symlink_metadata().is_ok())
    {
        return true;
    }
    let roots = crate::spawn::work_roots();
    roots.iter().any(|root| path.starts_with(resolved(root)))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn missing_locations_in_unregistered_trees_are_detected_through_aliases() {
        assert!(resolved(Path::new("uncreated-location-probe")).is_absolute());
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("tree");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join(".git"), "indirection").unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        for path in [
            root.join("absent"),
            root.join("nested/../absent"),
            alias.join("absent"),
        ] {
            assert!(inside_work(&path), "{}", path.display());
        }
        assert!(!inside_work(&temp.path().join("outside")));
    }
}
