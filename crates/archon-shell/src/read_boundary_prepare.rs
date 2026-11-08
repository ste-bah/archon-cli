use std::path::Path;

pub(super) use super::super::location::inside_work;

pub(super) fn materialize(path: &Path, notes: &mut Vec<String>) -> bool {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    if !literal(path) {
        notes.push(format!(
            "{} not sealed (uninterpolated include)",
            path.display()
        ));
        return false;
    }
    if path.symlink_metadata().is_ok() {
        return true;
    }
    let reason = if inside_work(path) {
        Some("inside work tree")
    } else {
        None
    };
    if let Some(reason) = reason {
        notes.push(format!("absent {} not sealed ({reason})", path.display()));
        return false;
    }
    let writable = path
        .parent()
        .and_then(|parent| std::ffi::CString::new(parent.as_os_str().as_bytes()).ok())
        .is_some_and(|parent| unsafe {
            libc::faccessat(
                libc::AT_FDCWD,
                parent.as_ptr(),
                libc::W_OK,
                libc::AT_EACCESS,
            ) == 0
        });
    if !writable {
        let reason = if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            "parent directory absent"
        } else {
            "parent not writable by this user"
        };
        notes.push(format!("absent {} not sealed ({reason})", path.display()));
        return false;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => {
            let note = format!(
                "created empty {} to seal it; retained persistently to preserve concurrent seals",
                path.display()
            );
            tracing::warn!("{note}");
            notes.push(note);
            match file.set_permissions(std::fs::Permissions::from_mode(0o600)) {
                Ok(()) => true,
                Err(error) => {
                    notes.push(format!(
                        "{} not sealed (private permissions could not be set: {error})",
                        path.display()
                    ));
                    false
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            path.symlink_metadata().is_ok()
        }
        Err(error) => {
            notes.push(format!("absent {} not sealed ({error})", path.display()));
            false
        }
    }
}

fn literal(path: &Path) -> bool {
    !path.components().any(|part| {
        let text = part.as_os_str().to_string_lossy();
        text.contains("%(")
            || text.matches('%').count() >= 2
            || text.contains('$')
            || text.starts_with('~')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unregistered_work_trees_and_parent_components_are_not_materialized() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("tree");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join(".git"), "indirection").unwrap();
        let mut notes = Vec::new();
        for path in [root.join("absent"), root.join("nested/../absent")] {
            assert!(!materialize(&path, &mut notes));
            assert!(!path.exists());
        }
        assert!(notes.iter().all(|note| note.contains("inside work tree")));
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        assert!(!materialize(&alias.join("absent"), &mut notes));
        assert!(!root.join("absent").exists());
    }

    #[test]
    fn missing_files_require_literal_writable_parent_outside_work() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config");
        let mut notes = Vec::new();
        assert!(materialize(&path, &mut notes));
        assert_eq!(std::fs::read(&path).unwrap(), b"");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for path in [
            temp.path().join("missing/config"),
            temp.path().join("%(prefix)/config"),
            temp.path().join("~other/config"),
            std::env::current_dir().unwrap().join("absent-config"),
        ] {
            assert!(!materialize(&path, &mut notes));
            assert!(!path.exists());
        }
        assert!(
            notes
                .iter()
                .any(|note| note.contains("parent directory absent"))
        );
        let link = temp.path().join("link");
        let target = temp.path().join("absent");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(materialize(&link, &mut notes));
        assert!(!target.exists());
    }
}
