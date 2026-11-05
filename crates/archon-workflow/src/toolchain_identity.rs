//! Content identity of programs available through the execution site's PATH.
//! Enumerating the search path is a generic fallback: custom runners receive
//! the same invalidation as built-in toolchains, without executing any tool.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;

pub fn fingerprint(
    search_path: &OsStr,
    digest: impl Fn(&[u8]) -> String,
) -> BTreeMap<String, String> {
    let mut tools = BTreeMap::new();
    let mut contents = BTreeMap::new();
    for directory in std::env::split_paths(search_path) {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            tools.insert(
                directory.display().to_string(),
                "unreadable directory".into(),
            );
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_program(&path) {
                continue;
            }
            // Keep every PATH entry, including shadowed programs: shell wrappers
            // may select a different entry after altering their environment.
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            let identity = contents
                .entry(canonical.clone())
                .or_insert_with(|| match std::fs::read(&path) {
                    Ok(bytes) => format!("{}:{}", canonical.display(), digest(&bytes)),
                    Err(error) => format!("{}:unreadable:{:?}", canonical.display(), error.kind()),
                })
                .clone();
            tools.insert(path.display().to_string(), identity);
        }
    }
    tools
}

fn is_program(path: &PathBuf) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_and_removal_of_custom_program_change_identity() {
        let root = std::env::temp_dir().join(format!("tool-identity-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let tool = root.join("custom-runner");
        std::fs::write(&tool, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let capture = || fingerprint(root.as_os_str(), |bytes| format!("{bytes:?}"));
        let before = capture();
        assert_eq!(before.len(), 1);
        std::fs::write(&tool, "new").unwrap();
        assert_ne!(before, capture());
        std::fs::remove_file(tool).unwrap();
        assert!(capture().is_empty());
        std::fs::remove_dir(root).unwrap();
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    #[test]
    fn every_tool_name_receives_the_same_content_invalidation() {
        let root = std::env::temp_dir().join(format!("tool-adapters-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for name in ["node", "python", "go", "java", "cargo", "custom-runner"] {
            let tool = root.join(name);
            std::fs::write(&tool, "version-one").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            let capture = || fingerprint(root.as_os_str(), |bytes| format!("{bytes:?}"));
            let before = capture();
            std::fs::write(&tool, "version-two").unwrap();
            assert_ne!(before, capture(), "{name}");
            std::fs::remove_file(tool).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
}
