use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) fn command_path(default: &str, env_key: &str) -> OsString {
    if let Some(path) = non_empty_env(env_key) {
        return path;
    }
    if has_path_separator(default) {
        return OsString::from(default);
    }
    find_on_path(default, std::env::var_os("PATH"))
        .or_else(|| find_in_common_dirs(default))
        .map(PathBuf::into_os_string)
        .unwrap_or_else(|| OsString::from(default))
}

fn non_empty_env(env_key: &str) -> Option<OsString> {
    std::env::var_os(env_key).filter(|value| !value.to_string_lossy().is_empty())
}

fn has_path_separator(value: &str) -> bool {
    value.contains('/') || value.contains('\\')
}

fn find_on_path(command: &str, path_env: Option<OsString>) -> Option<PathBuf> {
    let path_env = path_env?;
    std::env::split_paths(&path_env)
        .map(|dir| dir.join(command))
        .find(|path| executable_exists(path))
}

fn find_in_common_dirs(command: &str) -> Option<PathBuf> {
    common_tool_dirs()
        .iter()
        .map(|dir| Path::new(dir).join(command))
        .find(|path| executable_exists(path))
}

fn common_tool_dirs() -> &'static [&'static str] {
    &[
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/opt/local/bin",
        "/usr/bin",
        "/bin",
        "/snap/bin",
    ]
}

fn executable_exists(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins() {
        let env_key = "ARCHON_TEST_TOOL_PATH_OVERRIDE";
        let override_path = OsString::from("/custom/bin/pdftotext");
        unsafe {
            std::env::set_var(env_key, &override_path);
        }
        let resolved = command_path("pdftotext", env_key);
        unsafe {
            std::env::remove_var(env_key);
        }
        assert_eq!(resolved, override_path);
    }

    #[test]
    fn path_search_resolves_existing_tool() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("pdftoppm");
        std::fs::write(&tool, "").unwrap();
        let resolved = find_on_path("pdftoppm", Some(dir.path().as_os_str().to_os_string()));
        assert_eq!(resolved.as_deref(), Some(tool.as_path()));
    }
}
