use std::path::Path;

pub(super) fn write_archive(path: &Path, bytes: &[u8]) -> bool {
    use std::io::Write;
    for index in 0.. {
        let candidate = if index == 0 {
            path.to_path_buf()
        } else {
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            path.with_extension(format!("{extension}.{index}"))
        };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&candidate) {
            Ok(mut file) => {
                if file.write_all(bytes).is_ok() {
                    return true;
                }
                drop(file);
                let _ = std::fs::remove_file(&candidate);
                return false;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read(&candidate).is_ok_and(|archived| archived == bytes) {
                    return true;
                }
            }
            Err(_) => return false,
        }
    }
    false
}

/// Write `bytes` to `path`, readable by its owner only.
pub(super) fn write_owner_only(path: &Path, bytes: &[u8]) -> bool {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let restrict = |file: &std::fs::File| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
        }
        #[cfg(not(unix))]
        {
            let _ = file;
            Ok(())
        }
    };
    options
        .open(path)
        .and_then(|mut file| restrict(&file).and_then(|()| file.write_all(bytes)))
        .is_ok()
}
