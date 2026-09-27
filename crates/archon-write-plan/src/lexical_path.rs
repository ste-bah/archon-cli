//! Platform-independent spelling for paths carried in task and diagnostic text.
//! These helpers do not resolve symlinks or establish filesystem containment.

/// Normalize separators and Windows verbatim prefixes without losing the root.
pub fn portable(path: &str) -> String {
    let path = path.replace('\\', "/");
    if let Some(rest) = path.strip_prefix("//?/UNC/") {
        format!("//{rest}")
    } else {
        path.strip_prefix("//?/").unwrap_or(&path).to_string()
    }
}

/// Rooted POSIX/UNC or drive-qualified paths are never repository-relative.
pub fn rooted(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with(['/', '\\'])
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// Strip a complete root, not a directory whose name merely shares its prefix.
pub fn under_root(path: &str, root: &str) -> Option<String> {
    let path = portable(path);
    let root = portable(root);
    let root = root.trim_end_matches('/');
    path.strip_prefix(root)?
        .strip_prefix('/')
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_keep_their_identity_in_text_from_either_platform() {
        for (path, root, expected) in [
            ("/repo/src/a.rs", "/repo", Some("src/a.rs")),
            (r"C:\repo\src\a.rs", "C:/repo", Some("src/a.rs")),
            (r"\\?\C:\repo\src\a.rs", "C:/repo", Some("src/a.rs")),
            ("C:/repo/src/a.rs", r"\\?\C:\repo", Some("src/a.rs")),
            (r"\\?\UNC\server\share\a.rs", "//server/share", Some("a.rs")),
            ("/repository/a.rs", "/repo", None),
            ("D:/repo/a.rs", "C:/repo", None),
        ] {
            assert!(rooted(path));
            assert_eq!(under_root(path, root).as_deref(), expected);
        }
        assert!(!rooted("src/a.rs"));
        assert!(rooted("C:src/a.rs"));
    }
}
