//! Disposable sibling copies, preserving filesystem failure classes.
use super::*;
pub(super) fn link_repository_copy(target: &Path, destination: &Path) -> Result<(), Unrun> {
    #[cfg(test)]
    let injected = crate::command::workflow_host_fault_test_hooks::fire(
        crate::command::workflow_host_fault_test_hooks::Point::Link,
        destination,
    );
    #[cfg(not(test))]
    let injected: std::io::Result<()> = Ok(());
    #[cfg(unix)]
    let result = injected.and_then(|()| std::os::unix::fs::symlink(target, destination));
    #[cfg(windows)]
    let result = injected.and_then(|()| {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, destination)
        } else {
            std::os::windows::fs::symlink_file(target, destination)
        }
    });
    result.map_err(|error| {
        let context = format!(
            "could not link repository copy {} at sibling alias {}",
            target.display(),
            destination.display()
        );
        Unrun::error(error.into(), context)
    })
}

pub(super) fn clone_head(source: &Path, destination: &Path) -> Result<String, Unrun> {
    let expected = git_head(source)?;
    let output = archon_shell::spawn::command("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "clone",
            "--quiet",
            "--shared",
            "--",
        ])
        .arg(
            archon_shell::paths::try_plain(source.to_path_buf()).map_err(|e| {
                let text = format!("git clone {}: {e}", source.display());
                Unrun::error(e.into(), text)
            })?,
        )
        .arg(
            archon_shell::paths::try_plain(destination.to_path_buf()).map_err(|e| {
                let text = format!("git clone {}: {e}", destination.display());
                Unrun::error(e.into(), text)
            })?,
        )
        .output()
        .map_err(|error| {
            let text = format!("git clone {}: {error}", source.display());
            Unrun::error(error.into(), text)
        })?;
    if !output.status.success() {
        return Err(Unrun::unknown(format!(
            "git clone {}: {}",
            source.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let copied = git_head(destination)?;
    let current = git_head(source)?;
    if copied != expected || current != expected {
        return Err(Unrun::unknown(format!(
            "sibling revision changed while copying {} (expected {expected}, copied {copied}, current {current})",
            source.display()
        )));
    }
    Ok(expected)
}

pub(super) fn git_head(path: &Path) -> Result<String, Unrun> {
    let output = archon_shell::spawn::command("git")
        .args(["-C"])
        .arg(
            archon_shell::paths::try_plain(path.to_path_buf()).map_err(|e| {
                let text = format!("git rev-parse HEAD in {}: {e}", path.display());
                Unrun::error(e.into(), text)
            })?,
        )
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| {
            let text = format!("git rev-parse HEAD in {}: {error}", path.display());
            Unrun::error(error.into(), text)
        })?;
    if !output.status.success() {
        return Err(Unrun::unknown(format!(
            "git rev-parse HEAD in {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(super) fn copy_directory(root: &Path, source: &Path, destination: &Path) -> Result<(), Unrun> {
    let relative = source
        .strip_prefix(root)
        .expect("copy root contains source");
    let target = destination.join(relative);
    let metadata = source.symlink_metadata().map_err(|error| {
        let text = format!("{}: {error}", source.display());
        Unrun::error(error.into(), text)
    })?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        // A named sibling root is needed whole; skip build-output folders inside it.
        if source != root
            && SKIP
                .iter()
                .any(|name| source.file_name().is_some_and(|file| file == *name))
        {
            return Ok(());
        }
        std::fs::create_dir_all(&target).map_err(|error| {
            let text = format!("{}: {error}", target.display());
            Unrun::error(error.into(), text)
        })?;
        for item in std::fs::read_dir(source).map_err(|error| {
            let text = format!("{}: {error}", source.display());
            Unrun::error(error.into(), text)
        })? {
            let item = item.map_err(|error| {
                let text = format!("{}: {error}", source.display());
                Unrun::error(error.into(), text)
            })?;
            copy_directory(root, &item.path(), destination)?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                let text = format!("{}: {error}", parent.display());
                Unrun::error(error.into(), text)
            })?;
        }
        std::fs::copy(source, &target).map_err(|error| {
            let text = format!("{}: {error}", target.display());
            Unrun::error(error.into(), text)
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_command_sibling_link_keeps_transient_io() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("source");
        let destination = temp.path().join("alias");
        let fault = crate::command::workflow_host_fault_test_hooks::inject(
            crate::command::workflow_host_fault_test_hooks::Point::Link,
            &destination,
            Box::new(|| Err(crate::command::workflow_host_fault_test_hooks::transient_native_io())),
        );
        let error = link_repository_copy(&target, &destination).unwrap_err();
        assert!(fault.fired());
        assert_eq!(
            error.1,
            crate::command::workflow_host_fault::Class::Transient
        );
        assert!(error.0.contains("alias"));
    }
}
