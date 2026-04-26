//! TASK-#207 SLASH-FILES — directory walker for the file-picker overlay.
//!
//! `read_dir_entries(path)` enumerates one directory level (no
//! recursion), filters out:
//!   - dotfiles / dot-directories (`.git`, `.cargo`, `.archon`, …)
//!   - common build-artifact directories: `target`, `node_modules`,
//!     `dist`, `build`
//!   - dangling symlinks (where `Metadata::file_type()` errors)
//!
//! and returns a `Vec<FileEntry>` sorted dirs-first, alphabetical
//! within each kind. The mission's `.gitignore`-aware walk is
//! deferred — `globset` is in workspace deps, so the follow-up can
//! parse `.gitignore` lazily without adding a new dep.

use std::fs;
use std::io;
use std::path::Path;

use super::FileEntry;

/// Hardcoded skip-list of directory basenames that are virtually
/// always build artifacts or VCS metadata. Skipped at every level.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    "dist",
    "build",
    ".cache",
    ".venv",
    "__pycache__",
];

/// Read one level of `path` and return a sorted `FileEntry` list.
///
/// Dotfiles are filtered out (basename starts with `.`). Hardcoded
/// build/VCS dirs (`SKIP_DIRS`) are filtered out by basename match.
/// Entries whose `metadata()` errors (dangling symlinks, permission
/// denied) are silently skipped — the picker shows what it CAN
/// safely list, not what's syntactically present.
pub fn read_dir_entries(path: &Path) -> io::Result<Vec<FileEntry>> {
    let read = fs::read_dir(path)?;
    let mut entries: Vec<FileEntry> = Vec::new();
    for raw in read.flatten() {
        let name_os = raw.file_name();
        let name = match name_os.to_str() {
            Some(s) => s.to_string(),
            None => continue, // non-UTF-8 filename — skip
        };
        if name.starts_with('.') {
            continue;
        }
        if SKIP_DIRS.iter().any(|d| *d == name) {
            continue;
        }
        let meta = match raw.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let file_type = meta.file_type();
        let is_dir = file_type.is_dir();
        // Skip non-regular non-dir entries (sockets, fifos, char dev, etc.).
        if !is_dir && !file_type.is_file() {
            continue;
        }
        entries.push(FileEntry {
            name,
            path: raw.path(),
            is_dir,
        });
    }
    // Dirs first, then files, alphabetical within each kind.
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_tempdir(label: &str) -> std::path::PathBuf {
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("file_picker_walker_{label}_{pid}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn missing_dir_returns_err() {
        let r = read_dir_entries(std::path::Path::new("/nonexistent/zzz_missing"));
        assert!(r.is_err());
    }

    #[test]
    fn lists_files_and_dirs_dirs_first() {
        let root = unique_tempdir("dirs_first");
        fs::create_dir(root.join("zsubdir")).unwrap();
        fs::write(root.join("a-file.txt"), b"x").unwrap();
        fs::write(root.join("b-file.txt"), b"y").unwrap();
        let entries = read_dir_entries(&root).unwrap();
        // Order: zsubdir (dir, despite z), a-file.txt, b-file.txt.
        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "zsubdir");
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].name, "a-file.txt");
        assert_eq!(entries[2].name, "b-file.txt");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn filters_dotfiles_and_skip_dirs() {
        let root = unique_tempdir("filters");
        fs::create_dir(root.join(".git")).unwrap();
        fs::create_dir(root.join("target")).unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        fs::create_dir(root.join("real-dir")).unwrap();
        fs::write(root.join(".hidden"), b"x").unwrap();
        fs::write(root.join("visible.txt"), b"y").unwrap();
        let entries = read_dir_entries(&root).unwrap();
        // Only real-dir + visible.txt should survive.
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "real-dir");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "visible.txt");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn alphabetical_case_insensitive_within_kind() {
        let root = unique_tempdir("alpha_ci");
        fs::write(root.join("Apple.md"), b"x").unwrap();
        fs::write(root.join("banana.md"), b"y").unwrap();
        fs::write(root.join("cherry.md"), b"z").unwrap();
        let entries = read_dir_entries(&root).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Apple.md", "banana.md", "cherry.md"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_dir_returns_empty_vec() {
        let root = unique_tempdir("empty");
        let entries = read_dir_entries(&root).unwrap();
        assert!(entries.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
