//! Race-resistant host-side movement of mutation inputs.
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct SiteRoot {
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(unix)]
    directory: std::sync::Arc<std::os::fd::OwnedFd>,
    #[cfg(windows)]
    directory: std::sync::Arc<std::fs::File>,
    #[cfg(windows)]
    identity: (u32, u32, u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MoveOutcome {
    Moved,
    Absent,
    SymlinkComponent,
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RestoreOutcome {
    Restored,
    OriginalRecreated,
    AsideMissing,
    UnsafeToRestore(String),
}

impl SiteRoot {
    #[cfg(unix)]
    pub(crate) fn capture(path: &Path) -> std::io::Result<Self> {
        // Resolve host-owned symlinks above the captured root once. The
        // descriptor walk below this point still rejects every symlink.
        let path = path.canonicalize()?;
        let directory = open_root(&path)?;
        Ok(Self {
            directory: std::sync::Arc::new(directory.into()),
        })
    }

    #[cfg(windows)]
    pub(crate) fn capture(path: &Path) -> std::io::Result<Self> {
        let path = path.canonicalize()?;
        let directory = open_windows_root(&path)?;
        let identity = windows_file_identity(&directory)?;
        Ok(Self {
            path,
            directory: std::sync::Arc::new(directory),
            identity,
        })
    }
}

#[cfg(unix)]
pub(crate) fn move_aside(root: &SiteRoot, rel: &Path, aside_rel: &Path) -> MoveOutcome {
    use std::os::fd::AsRawFd;
    let result = (|| -> std::io::Result<MoveOutcome> {
        let root_fd = open_verified_root(root)?;
        let (parent, leaf) = split_rel(rel)?;
        let (aside_parent, aside_leaf) = split_rel(aside_rel)?;
        let source = walk_parent(&root_fd, &parent, false)?;
        let name = c_name(&leaf)?;
        let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
        if unsafe {
            libc::fstatat(
                source.as_raw_fd(),
                name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(MoveOutcome::Absent);
            }
            return Err(error);
        }
        if stat.st_mode & libc::S_IFMT == libc::S_IFLNK {
            return Ok(MoveOutcome::SymlinkComponent);
        }
        let target = walk_parent(&root_fd, &aside_parent, true)?;
        let aside_name = c_name(&aside_leaf)?;
        if unsafe {
            libc::renameat(
                source.as_raw_fd(),
                name.as_ptr(),
                target.as_raw_fd(),
                aside_name.as_ptr(),
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(MoveOutcome::Moved)
    })();
    result.unwrap_or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            MoveOutcome::Absent
        } else if matches!(
            error.raw_os_error(),
            Some(libc::ELOOP) | Some(libc::ENOTDIR)
        ) {
            MoveOutcome::SymlinkComponent
        } else {
            MoveOutcome::Failed(error.to_string())
        }
    })
}

#[cfg(unix)]
pub(crate) fn restore(root: &SiteRoot, aside_rel: &Path, rel: &Path) -> RestoreOutcome {
    use std::os::fd::AsRawFd;
    let result = (|| -> std::io::Result<RestoreOutcome> {
        let root_fd = open_verified_root(root)?;
        let (aside_parent, aside_leaf) = split_rel(aside_rel)?;
        let (target_parent, target_leaf) = split_rel(rel)?;
        let aside_dir = match walk_parent(&root_fd, &aside_parent, false) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RestoreOutcome::AsideMissing);
            }
            Err(error) => return Err(error),
        };
        let aside_name = c_name(&aside_leaf)?;
        if !entry_exists(aside_dir.as_raw_fd(), &aside_name)? {
            return Ok(RestoreOutcome::AsideMissing);
        }
        let target_dir = walk_parent(&root_fd, &target_parent, false)?;
        let target_name = c_name(&target_leaf)?;
        if entry_exists(target_dir.as_raw_fd(), &target_name)? {
            return Ok(RestoreOutcome::OriginalRecreated);
        }
        if unsafe {
            libc::renameat(
                aside_dir.as_raw_fd(),
                aside_name.as_ptr(),
                target_dir.as_raw_fd(),
                target_name.as_ptr(),
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(RestoreOutcome::Restored)
    })();
    result.unwrap_or_else(|error| RestoreOutcome::UnsafeToRestore(error.to_string()))
}

pub(crate) fn snapshot_parent(root: &SiteRoot, rel: &Path) -> std::io::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let root_fd = open_verified_root(root)?;
        let (parent, _) = split_rel(rel)?;
        let directory = walk_parent(&root_fd, &parent, false)?;
        let metadata = directory.metadata()?;
        Ok(format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.mode(),
            metadata.nlink(),
            metadata.uid(),
            metadata.gid(),
            metadata.size(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec()
        ))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        windows_components_are_safe(root, rel, false)?;
        let parent = rel.parent().unwrap_or_else(|| Path::new(""));
        let metadata = std::fs::metadata(root.path.join(parent))?;
        Ok(format!(
            "{}:{}:{}:{}",
            metadata.file_attributes(),
            metadata.file_size(),
            metadata.creation_time(),
            metadata.last_write_time()
        ))
    }
}

#[cfg(unix)]
fn open_root(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    options.open(path)
}

#[cfg(unix)]
fn open_verified_root(root: &SiteRoot) -> std::io::Result<std::fs::File> {
    Ok(std::fs::File::from(root.directory.try_clone()?))
}

#[cfg(unix)]
fn split_rel(path: &Path) -> std::io::Result<(PathBuf, std::ffi::OsString)> {
    if path.is_absolute() {
        return Err(invalid_path("absolute path rejected"));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => return Err(invalid_path("current-directory component rejected")),
            Component::ParentDir => {
                return Err(invalid_path("parent-directory component rejected"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(invalid_path("absolute path rejected"));
            }
        }
    }
    let Some((leaf, parents)) = parts.split_last() else {
        return Err(invalid_path("empty path rejected"));
    };
    let parent = parents
        .iter()
        .fold(PathBuf::new(), |path, part| path.join(part));
    Ok((parent, leaf.to_os_string()))
}

#[cfg(unix)]
fn invalid_path(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}

#[cfg(unix)]
fn c_name(name: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(name.as_bytes()).map_err(|_| invalid_path("NUL in path component"))
}

#[cfg(unix)]
fn walk_parent(
    root: &std::fs::File,
    relative: &Path,
    create: bool,
) -> std::io::Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let duplicate = unsafe { libc::dup(root.as_raw_fd()) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut current = unsafe { std::fs::File::from_raw_fd(duplicate) };
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(invalid_path("unsafe path component"));
        };
        let name = c_name(part)?;
        let next = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if next >= 0 {
            current = unsafe { std::fs::File::from_raw_fd(next) };
            continue;
        }
        let error = std::io::Error::last_os_error();
        if !create || error.kind() != std::io::ErrorKind::NotFound {
            return Err(error);
        }
        if unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            let mkdir_error = std::io::Error::last_os_error();
            if mkdir_error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(mkdir_error);
            }
        }
        let next = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if next < 0 {
            return Err(std::io::Error::last_os_error());
        }
        current = unsafe { std::fs::File::from_raw_fd(next) };
    }
    Ok(current)
}

#[cfg(unix)]
fn entry_exists(parent: libc::c_int, name: &std::ffi::CString) -> std::io::Result<bool> {
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe { libc::fstatat(parent, name.as_ptr(), &mut stat, libc::AT_SYMLINK_NOFOLLOW) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(windows)]
pub(crate) fn move_aside(root: &SiteRoot, rel: &Path, aside_rel: &Path) -> MoveOutcome {
    if !valid_relative(rel) || !valid_relative(aside_rel) {
        return MoveOutcome::Failed("unsafe relative path".into());
    }
    // This runs before the check starts, so its reparse points cannot race validation.
    if let Err(error) = windows_components_are_safe(root, rel, false) {
        return MoveOutcome::Failed(error.to_string());
    }
    let source = root.path.join(rel);
    match std::fs::symlink_metadata(&source) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => MoveOutcome::Absent,
        Err(error) => MoveOutcome::Failed(error.to_string()),
        Ok(meta) if is_unsafe_windows_component(&meta) => MoveOutcome::SymlinkComponent,
        Ok(_) => {
            let aside = root.path.join(aside_rel);
            if let Err(error) = ensure_windows_parent(root, aside_rel) {
                return MoveOutcome::Failed(error.to_string());
            }
            match std::fs::rename(source, aside) {
                Ok(()) => MoveOutcome::Moved,
                Err(error) => MoveOutcome::Failed(error.to_string()),
            }
        }
    }
}

#[cfg(windows)]
pub(crate) fn restore(root: &SiteRoot, aside_rel: &Path, rel: &Path) -> RestoreOutcome {
    // Restore runs after the job object confirms the check has no live processes.
    if !valid_relative(rel) || !valid_relative(aside_rel) {
        return RestoreOutcome::UnsafeToRestore("unsafe relative path".into());
    }
    if let Err(error) = windows_components_are_safe(root, rel, false)
        .and_then(|()| windows_components_are_safe(root, aside_rel, false))
    {
        return RestoreOutcome::UnsafeToRestore(error.to_string());
    }
    let target = root.path.join(rel);
    let aside = root.path.join(aside_rel);
    match std::fs::symlink_metadata(&target) {
        Ok(meta) if is_unsafe_windows_component(&meta) => {
            return RestoreOutcome::UnsafeToRestore("unsafe target component".into());
        }
        Ok(_) => return RestoreOutcome::OriginalRecreated,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return RestoreOutcome::UnsafeToRestore(error.to_string()),
    }
    match std::fs::symlink_metadata(&aside) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RestoreOutcome::AsideMissing;
        }
        Err(error) => return RestoreOutcome::UnsafeToRestore(error.to_string()),
        Ok(meta) if is_unsafe_windows_component(&meta) => {
            return RestoreOutcome::UnsafeToRestore("unsafe aside component".into());
        }
        Ok(_) => {}
    }
    match std::fs::rename(aside, target) {
        Ok(()) => RestoreOutcome::Restored,
        Err(error) => RestoreOutcome::UnsafeToRestore(error.to_string()),
    }
}

#[cfg(windows)]
fn valid_relative(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

#[cfg(windows)]
fn is_unsafe_windows_component(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(windows)]
fn windows_components_are_safe(
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

#[cfg(windows)]
fn ensure_windows_parent(root: &SiteRoot, relative: &Path) -> std::io::Result<()> {
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

#[cfg(windows)]
fn open_windows_root(path: &Path) -> std::io::Result<std::fs::File> {
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

#[cfg(windows)]
fn windows_file_identity(file: &std::fs::File) -> std::io::Result<(u32, u32, u32)> {
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

#[cfg(windows)]
fn verify_windows_root(root: &SiteRoot) -> std::io::Result<()> {
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

#[cfg(test)]
#[path = "acceptance_scratch_mutation_fd_tests.rs"]
mod tests;
#[cfg(all(test, windows))]
#[path = "acceptance_scratch_mutation_fd_windows_tests.rs"]
mod windows_tests;
