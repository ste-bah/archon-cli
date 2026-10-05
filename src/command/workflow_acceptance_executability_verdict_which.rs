//! Finding a program on a search path the way the platform's launcher does
//! (Issues 328, 331), for every check, on every platform.
//!
//! A search path is split on the platform's own separator -- `:` on Unix,
//! `;` on Windows, where an entry may be quoted -- and on Windows a program
//! named without one of the executable extensions `PATHEXT` lists (`.EXE`,
//! `.CMD`, ...) is also looked for with each of them, as `bash` is found as
//! `bash.exe`. Windows file names compare without case, which its file
//! system already does. The splitting and naming are pure functions of
//! their inputs, so both platforms' rules are tested everywhere.

use std::path::{Path, PathBuf};

/// The extensions Windows tries when `PATHEXT` is unset.
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Where the program `name` is on the search path `path`, on this platform.
pub(super) fn find(path: &str, name: &str) -> Option<PathBuf> {
    let windows = cfg!(windows);
    let pathext = std::env::var("PATHEXT").ok();
    let extensions = extensions(windows, pathext.as_deref());
    (search_dirs(path, windows).into_iter())
        .flat_map(|dir| {
            candidates(name, &extensions)
                .into_iter()
                .map(move |file| dir.join(file))
        })
        .find(|program| program.is_file())
}

/// The directories of the search path `path`, split as `windows` does or
/// not; empty entries are left out.
pub(super) fn search_dirs(path: &str, windows: bool) -> Vec<PathBuf> {
    if !windows {
        return (path.split(':').filter(|dir| !dir.is_empty()))
            .map(PathBuf::from)
            .collect();
    }
    let (mut dirs, mut dir, mut quoted) = (Vec::new(), String::new(), false);
    for c in path.chars() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => dirs.push(std::mem::take(&mut dir)),
            c => dir.push(c),
        }
    }
    dirs.push(dir);
    (dirs.into_iter().filter(|dir| !dir.is_empty()))
        .map(PathBuf::from)
        .collect()
}

/// The executable extensions tried for a bare name: none off Windows; on
/// Windows those `pathext` lists (its default when unset or empty).
pub(super) fn extensions(windows: bool, pathext: Option<&str>) -> Vec<String> {
    if !windows {
        return Vec::new();
    }
    let listed = pathext.filter(|text| !text.trim().is_empty());
    (listed.unwrap_or(DEFAULT_PATHEXT).split(';'))
        .map(str::trim)
        .filter(|ext| ext.starts_with('.') && ext.len() > 1)
        .map(str::to_string)
        .collect()
}

/// The file names `name` may have: itself, then itself with each of
/// `extensions` it does not already end with (compared without case).
pub(super) fn candidates(name: &str, extensions: &[String]) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    let mut names = vec![name.to_string()];
    if !extensions
        .iter()
        .any(|ext| lower.ends_with(&ext.to_ascii_lowercase()))
    {
        names.extend(extensions.iter().map(|ext| format!("{name}{ext}")));
    }
    names
}

/// A program file's name without the executable extension this platform
/// adds (`cargo.exe` is `cargo` on Windows): the name its subcommands use.
pub(super) fn bare_name(name: &str) -> String {
    let pathext = std::env::var("PATHEXT").ok();
    strip_extension(name, &extensions(cfg!(windows), pathext.as_deref()))
}

/// `name` without the first of `extensions` it ends with (without case).
pub(super) fn strip_extension(name: &str, extensions: &[String]) -> String {
    let lower = name.to_ascii_lowercase();
    (extensions.iter())
        .find(|ext| lower.ends_with(&ext.to_ascii_lowercase()) && name.len() > ext.len())
        .map_or_else(
            || name.to_string(),
            |ext| name[..name.len() - ext.len()].to_string(),
        )
}

/// Whether the program word `word` names a file by a path rather than a
/// name the search path resolves.
pub(super) fn is_path(word: &str) -> bool {
    word.contains('/') || (cfg!(windows) && word.contains('\\'))
}

/// Whether the program path `word` is absolute: from the root (as the POSIX
/// shell a check runs in reads it) or by this platform's own rule (a drive).
pub(super) fn is_absolute(word: &str) -> bool {
    word.starts_with('/') || Path::new(word).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_search_path_splits_on_semicolons_and_honours_quotes() {
        let dirs = search_dirs(
            r#"C:\Program Files\Git\usr\bin;;"C:\Tools;x\bin";C:\Windows\system32"#,
            true,
        );
        let dirs: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        assert_eq!(
            dirs,
            vec![
                r"C:\Program Files\Git\usr\bin",
                r"C:\Tools;x\bin",
                r"C:\Windows\system32"
            ]
        );
        let unix = search_dirs("/usr/local/bin::/usr/bin", false);
        assert_eq!(
            unix,
            vec![PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")]
        );
    }

    #[test]
    fn a_bare_name_is_tried_with_each_pathext_extension_on_windows_only() {
        let exts = extensions(true, Some(".COM;.EXE;.BAT;.CMD"));
        assert_eq!(
            candidates("bash", &exts),
            vec!["bash", "bash.COM", "bash.EXE", "bash.BAT", "bash.CMD"]
        );
        assert_eq!(
            candidates("bash.exe", &exts),
            vec!["bash.exe"],
            "case-blind"
        );
        assert_eq!(candidates("python3.11", &exts)[2], "python3.11.EXE");
        assert_eq!(extensions(true, None), extensions(true, Some("")));
        assert_eq!(extensions(true, None).len(), 4);
        assert!(extensions(false, Some(".EXE")).is_empty());
        assert_eq!(candidates("bash", &extensions(false, None)), vec!["bash"]);
        assert_eq!(strip_extension("cargo.EXE", &exts), "cargo");
        assert_eq!(strip_extension("cargo", &exts), "cargo");
        assert_eq!(strip_extension("cargo.exe", &[]), "cargo.exe");
    }

    #[test]
    fn a_program_on_the_host_path_is_found_and_a_missing_one_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "tool328.exe"
        } else {
            "tool328"
        };
        std::fs::write(dir.path().join(name), "").unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        let path = path.to_string_lossy();
        assert_eq!(find(&path, "tool328"), Some(dir.path().join(name)));
        assert_eq!(find(&path, "absent328"), None);
        assert!(is_path("scripts/new.sh") && !is_path("bash"));
        assert!(is_absolute("/opt/x") && !is_absolute("bin/x"));
    }
}
