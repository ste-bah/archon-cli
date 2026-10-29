//! Recovery of absent skeleton bytes from a digest-authenticated local cache.
//! Existing, mismatching live bytes remain evidence of corruption. Recovery
//! runs under the existing exclusive chain lock, after journal settlement.
use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
};
use archon_workflow::task_skeleton::TaskSkeletonLock;
use std::io::Write;
use std::path::{Path, PathBuf};

fn cache_path(project: &Path, digest: &str) -> Result<PathBuf> {
    // The existing digest protocol is a 32-byte hash in hexadecimal. A lock
    // must never turn a cache lookup into an arbitrary relative path.
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "invalid frozen skeleton digest; restore its lock from the accepted publication"
        ));
    }
    Ok(project
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR)
        .join("skeletons")
        .join(digest))
}

fn predecessor_path(pin_path: &Path) -> PathBuf {
    pin_path
        .parent()
        .expect("pin directory")
        .join("predecessors")
        .join(pin_path.file_name().expect("pin filename"))
}

pub(crate) fn authenticated_bytes(
    project: &Path,
    root: &Path,
    lock: &TaskSkeletonLock,
) -> Result<Vec<u8>> {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project, root);
    let pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&super::verification::read(&pin_path, true)?)?;
    let anchor = if pin.skeleton_digest.is_some() {
        pin
    } else {
        let predecessor = predecessor_path(&pin_path);
        serde_json::from_slice(&super::verification::read(&predecessor, true)?)
            .with_context(|| format!("parsing skeleton predecessor {}; restore that predecessor pin from its accepted publication", predecessor.display()))?
    };
    if anchor.skeleton_digest.as_deref() != Some(lock.digest.as_str())
        || anchor.acceptance_digest != lock.acceptance_digest
        || anchor.skeleton_gate.as_ref() != Some(&lock.gate)
        || Path::new(&anchor.task_root).canonicalize()? != root.canonicalize()?
    {
        return Err(anyhow!(
            "{} is not authenticated by host pin {}; restore the lock and pin from the same accepted publication",
            root.join(TASK_SKELETON_LOCK_FILE).display(),
            pin_path.display()
        ));
    }
    if lock.algorithm != "blake3" {
        return Err(anyhow!(
            "unsupported algorithm in {}; restore that lock from its accepted publication",
            root.join(TASK_SKELETON_LOCK_FILE).display()
        ));
    }
    let live = root.join(TASK_SKELETON_FILE);
    match super::verification::read(&live, true) {
        Ok(bytes) if content_digest(&bytes) == lock.digest => return Ok(bytes),
        Err(error)
            if error.chain().any(|error| {
                error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() != std::io::ErrorKind::NotFound)
            }) =>
        {
            return Err(error);
        }
        _ => {}
    }
    let cache = cache_path(project, &lock.digest)?;
    let bytes = super::verification::read(&cache, true).with_context(|| {
        format!(
            "restore {} from its accepted publication; authenticated cache {} is unavailable",
            live.display(),
            cache.display()
        )
    })?;
    if content_digest(&bytes) != lock.digest {
        return Err(anyhow!(
            "{} does not authenticate frozen digest {}; restore {} from the accepted publication",
            cache.display(),
            lock.digest,
            live.display()
        ));
    }
    Ok(bytes)
}

fn atomic_write(project: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    crate::command::workflow_task_set::validate_existing_parents(path, project)?;
    let parent = path.parent().expect("artifact parent");
    crate::command::workflow_task_set::create_dir_all_durably(parent)?;
    crate::command::workflow_task_set::validate_existing_parents(path, project)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        crate::command::workflow_task_set::sync_parent(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub(super) fn restore_missing(project: &Path, root: &Path) -> Result<()> {
    let path = root.join(TASK_SKELETON_FILE);
    let lock_path = root.join(TASK_SKELETON_LOCK_FILE);
    // A dangling link is existing state too; never follow or replace it as a
    // missing artifact. Only a genuinely absent destination can be recovered.
    if std::fs::symlink_metadata(&path).is_ok()
        || !lock_path.try_exists()?
        || !root
            .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE)
            .try_exists()?
    {
        return Ok(());
    }
    let lock: TaskSkeletonLock =
        serde_json::from_slice(&super::verification::read(&lock_path, true)?)
            .with_context(|| format!("parsing frozen skeleton lock {}", lock_path.display()))?;
    let bytes = authenticated_bytes(project, root, &lock)?;
    atomic_write(root, &path, &bytes)
}

pub(crate) fn backfill(project: &Path, root: &Path) -> Result<()> {
    let path = root.join(TASK_SKELETON_FILE);
    let bytes = super::verification::read(&path, true)?;
    // The caller just verified this chain under the same lock. Derive the
    // cache address from those exact bytes, never from a journal's targets.
    let pin = super::read_pin(project, root)?;
    if pin.skeleton_digest.as_deref() != Some(content_digest(&bytes).as_str()) {
        return Err(anyhow!(
            "{} differs from host pin; restore the accepted skeleton before retaining recovery bytes",
            path.display()
        ));
    }
    let cache = cache_path(project, &content_digest(&bytes))?;
    if std::fs::read(&cache).is_ok_and(|cached| cached == bytes) {
        return Ok(());
    }
    atomic_write(project, &cache, &bytes)
}

/// Preserve host-owned predecessor evidence before an acceptance rebind clears
/// the successor fields. A supplied lock or cache can never create this anchor.
pub(crate) fn remember_pin(pin_path: &Path, root: &Path) -> Result<()> {
    let bytes = std::fs::read(pin_path)?;
    let pin: archon_workflow::task_set_contract::AcceptancePin = serde_json::from_slice(&bytes)?;
    let lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(root.join(TASK_SKELETON_LOCK_FILE))?)?;
    if pin.skeleton_digest.as_deref() != Some(lock.digest.as_str())
        || pin.skeleton_gate.as_ref() != Some(&lock.gate)
        || pin.acceptance_digest != lock.acceptance_digest
    {
        return Err(anyhow!(
            "cannot retain skeleton ownership without its host pin"
        ));
    }
    atomic_write(
        pin_path.parent().expect("pin directory"),
        &predecessor_path(pin_path),
        &bytes,
    )
}

/// Retain only host-owned evidence while the publisher holds its exclusive
/// lock. Called before replacing any pin and again after applying its successor
/// but before commit. A failed successor retention rolls the publication back;
/// a failed outgoing retention leaves every publication target untouched.
/// Publications without a host pin target carry no skeleton ownership.
pub(crate) fn retain_publication_pin(
    pin_path: &Path,
    root: &Path,
    files: &[(PathBuf, Vec<u8>)],
) -> Result<()> {
    use archon_workflow::task_set_contract::AcceptancePin;
    if !files.iter().any(|(path, _)| path == pin_path) {
        return Ok(());
    }
    let bytes = match std::fs::read(pin_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("reading outgoing host pin before publication"),
    };
    let pin: AcceptancePin = match serde_json::from_slice(&bytes) {
        Ok(pin) => pin,
        // The transaction primitive also publishes opaque metadata. Only a
        // typed host pin can introduce a chain or replace its ownership.
        Err(_)
            if !files.iter().any(|(path, bytes)| {
                path == pin_path && serde_json::from_slice::<AcceptancePin>(bytes).is_ok()
            }) =>
        {
            return Ok(());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("parsing outgoing host pin {}", pin_path.display()));
        }
    };
    if pin.skeleton_digest.is_none() {
        return Ok(());
    }
    remember_pin(pin_path, root).with_context(|| {
        format!(
            "retaining skeleton ownership before publication of {}; repair predecessor storage and retry publication",
            pin_path.display()
        )
    })
}
