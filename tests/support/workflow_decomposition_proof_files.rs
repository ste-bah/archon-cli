use super::*;
use std::io::Read;

pub fn snapshot_tree(root: &Path) -> Result<ProtectedSnapshot, String> {
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("protected root {} is unavailable: {error}", root.display()))?;
    let mut entries = Vec::new();
    collect_protected_entries(&canonical, &canonical, &mut entries)?;
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let digest = digest_json(&entries)?;
    Ok(ProtectedSnapshot {
        root: slash_path(&canonical),
        entries,
        digest,
    })
}

pub fn require_unchanged(
    before: &ProtectedSnapshot,
    after: &ProtectedSnapshot,
) -> Result<(), String> {
    if before != after {
        return Err("protected tree changed during proof".into());
    }
    Ok(())
}

fn collect_protected_entries(
    root: &Path,
    current: &Path,
    out: &mut Vec<ProtectedEntry>,
) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(current)
        .map_err(|error| format!("reading protected metadata {}: {error}", current.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "proof snapshot refuses symlink {}",
            current.display()
        ));
    }
    let relative = current
        .strip_prefix(root)
        .map_err(|error| error.to_string())?;
    let relative_path = if relative.as_os_str().is_empty() {
        ".".to_string()
    } else {
        slash_path(relative)
    };
    if metadata.is_dir() {
        out.push(ProtectedEntry {
            relative_path,
            kind: "directory".into(),
            mode: metadata_mode(&metadata),
            byte_len: 0,
            sha256: String::new(),
        });
        for entry in std::fs::read_dir(current)
            .map_err(|error| format!("reading {}: {error}", current.display()))?
        {
            collect_protected_entries(
                root,
                &entry.map_err(|error| error.to_string())?.path(),
                out,
            )?;
        }
    } else if metadata.is_file() {
        let (bytes, opened_metadata) = read_regular_nofollow_with_metadata(current)?;
        out.push(ProtectedEntry {
            relative_path,
            kind: "file".into(),
            mode: metadata_mode(&opened_metadata),
            byte_len: bytes.len() as u64,
            sha256: sha256(&bytes),
        });
    } else {
        return Err(format!(
            "proof snapshot refuses non-file object {}",
            current.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

fn read_regular_nofollow_with_metadata(
    path: &Path,
) -> Result<(Vec<u8>, std::fs::Metadata), String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("reading metadata {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("proof refuses symlink {}", path.display()));
        }
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("opening regular file {}: {error}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("reading metadata {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("proof requires regular file {}", path.display()));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("reading regular file {}: {error}", path.display()))?;
    Ok((bytes, metadata))
}

pub(crate) fn read_regular_nofollow(path: &Path) -> Result<Vec<u8>, String> {
    read_regular_nofollow_with_metadata(path).map(|(bytes, _)| bytes)
}

pub(crate) fn collect_regular_files(
    root: &Path,
    current: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(current)
        .map_err(|error| format!("reading {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let path = entry.path();
        if file_type.is_symlink() {
            return Err(format!("proof snapshot refuses symlink {}", path.display()));
        }
        if file_type.is_dir() {
            collect_regular_files(root, &path, out)?;
        } else if file_type.is_file() {
            out.push(
                path.strip_prefix(root)
                    .map_err(|error| error.to_string())?
                    .to_path_buf(),
            );
        } else {
            return Err(format!("proof refuses non-file object {}", path.display()));
        }
    }
    Ok(())
}
