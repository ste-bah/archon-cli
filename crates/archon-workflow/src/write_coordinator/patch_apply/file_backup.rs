use std::path::Path;

#[derive(Debug)]
pub(super) struct FileBackups {
    entries: Vec<FileBackup>,
}

#[derive(Debug)]
enum FileBackup {
    Present {
        relative: String,
        bytes: Vec<u8>,
        mode: Option<u32>,
    },
    Missing {
        relative: String,
    },
}

impl FileBackups {
    pub(super) fn capture(root: &Path, changed_files: &[String]) -> std::io::Result<Self> {
        let mut entries = Vec::new();
        for relative in changed_files {
            let path = root.join(relative);
            match std::fs::read(&path) {
                Ok(bytes) => entries.push(FileBackup::Present {
                    relative: relative.clone(),
                    bytes,
                    mode: file_mode(&path),
                }),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    entries.push(FileBackup::Missing {
                        relative: relative.clone(),
                    });
                }
                Err(err) => return Err(err),
            }
        }
        Ok(Self { entries })
    }

    pub(super) fn restore(&self, root: &Path) -> std::io::Result<()> {
        for entry in &self.entries {
            restore_entry(root, entry)?;
        }
        Ok(())
    }
}

fn restore_entry(root: &Path, entry: &FileBackup) -> std::io::Result<()> {
    match entry {
        FileBackup::Present {
            relative,
            bytes,
            mode,
        } => restore_present(&root.join(relative), bytes, *mode),
        FileBackup::Missing { relative } => remove_if_present(&root.join(relative)),
    }
}

fn restore_present(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    set_mode(path, mode)
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(unix)]
fn file_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|meta| meta.permissions().mode())
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Option<u32> {
    None
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: Option<u32>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: Option<u32>) -> std::io::Result<()> {
    Ok(())
}
