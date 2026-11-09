//! Ownership markers live beside disposable roots, never inside them.
use super::*;
use std::time::Duration;
pub(super) fn marker(root: &Path) -> PathBuf {
    let mut name = root.as_os_str().to_os_string();
    name.push(".owner");
    PathBuf::from(name)
}
pub(super) fn write(root: &Path) -> Result<(), Unrun> {
    std::fs::write(
        marker(root),
        serde_json::json!({"pid": std::process::id()}).to_string(),
    )
    .map_err(|e| Unrun::error(e.into(), "writing disposable owner".into()))
}
pub(super) fn remove(root: &Path) -> Result<(), String> {
    remove_with_delays(root, &[0, 200, 1000, 3000])
}
pub(super) fn remove_now(root: &Path) -> Result<(), String> {
    remove_with_delays(root, &[0])
}
fn remove_with_delays(root: &Path, delays: &[u64]) -> Result<(), String> {
    let mut last = None;
    for &delay in delays {
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        let removal = match root.symlink_metadata() {
            Ok(meta) if meta.is_file() => std::fs::remove_file(root),
            Ok(_) => std::fs::remove_dir_all(root),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        };
        match removal {
            Ok(()) => {
                last = None;
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                last = None;
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    if let Some(e) = last {
        return Err(format!("copy_not_removed: {}: {e}", root.display()));
    }
    match std::fs::remove_file(marker(root)) {
        Ok(()) => Ok(()),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(())
        }
        Err(e) => Err(format!("{}: {e}", marker(root).display())),
    }
}
pub(super) fn sweep(parent: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut failures = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(COPY_PREFIX) || name == WARM_TARGETS {
            continue;
        }
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
        let stale = meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age.as_secs() > STALE_COPY_SECS);
        if meta.is_file() {
            if let Some(stem) = name.strip_suffix(".owner")
                && stale
                && !parent.join(stem).exists()
                && let Err(e) = std::fs::remove_file(entry.path())
            {
                failures.push(format!("{}: {e}", entry.path().display()));
            }
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        let dead = std::fs::read(marker(&entry.path()))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get("pid").and_then(|p| p.as_u64()))
            .and_then(|p| u32::try_from(p).ok())
            .is_some_and(|pid| {
                pid != std::process::id() && !archon_workflow::process_liveness::process_alive(pid)
            });
        if (dead || stale)
            && let Err(e) = remove(&entry.path())
        {
            failures.push(e);
        }
    }
    failures
}
