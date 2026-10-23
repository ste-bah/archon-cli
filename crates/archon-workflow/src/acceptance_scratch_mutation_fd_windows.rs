use std::path::{Component, Path};

use super::SiteRoot;

pub(super) fn valid_relative(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

pub(super) fn is_unsafe_windows_component(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

pub(super) fn windows_components_are_safe(
    root: &SiteRoot,
    relative: &Path,
    include_leaf: bool,
) -> std::io::Result<()> {
    verify_windows_root(root)?;
    let components = relative.components().collect::<Vec<_>>();
    let count = components.len().saturating_sub(usize::from(!include_leaf));
    let mut current = root.path.clone();
    for component in components.into_iter().take(count) {
        let Component::Normal(part) = component else {
            return Err(std::io::Error::other("unsafe path component"));
        };
        current.push(part);
        let metadata = std::fs::symlink_metadata(&current)?;
        if is_unsafe_windows_component(&metadata) || !metadata.is_dir() {
            return Err(std::io::Error::other("unsafe path component"));
        }
    }
    Ok(())
}

pub(super) fn ensure_windows_parent(root: &SiteRoot, relative: &Path) -> std::io::Result<()> {
    verify_windows_root(root)?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let mut current = root.path.clone();
    for component in parent.components() {
        let Component::Normal(part) = component else {
            return Err(std::io::Error::other("unsafe path component"));
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if is_unsafe_windows_component(&metadata) || !metadata.is_dir() => {
                return Err(std::io::Error::other("unsafe path component"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current)?;
                let metadata = std::fs::symlink_metadata(&current)?;
                if is_unsafe_windows_component(&metadata) || !metadata.is_dir() {
                    return Err(std::io::Error::other("unsafe path component"));
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(super) fn open_windows_root(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    options.open(path)
}

pub(super) fn windows_file_identity(file: &std::fs::File) -> std::io::Result<(u32, u32, u32)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}

pub(super) fn verify_windows_root(root: &SiteRoot) -> std::io::Result<()> {
    let current = open_windows_root(&root.path)?;
    if windows_file_identity(&root.directory)? != root.identity
        || windows_file_identity(&current)? != root.identity
    {
        return Err(std::io::Error::other(
            "captured mutation root identity changed",
        ));
    }
    let root_meta = std::fs::symlink_metadata(&root.path)?;
    if is_unsafe_windows_component(&root_meta)
        || !root_meta.is_dir()
        || root.path.canonicalize()? != root.path
    {
        return Err(std::io::Error::other("unsafe mutation root"));
    }
    Ok(())
}
