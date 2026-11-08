//! Persistent user configuration and session startup paths.
use std::path::{Path, PathBuf};

pub(super) fn user_paths(home: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = [
        ".profile",
        ".bash_profile",
        ".bash_login",
        ".bashrc",
        ".bash_logout",
        ".zshenv",
        ".zprofile",
        ".zshrc",
        ".zlogin",
        ".config/fish/config.fish",
        ".config/powershell",
        #[cfg(windows)]
        "Documents/PowerShell",
        #[cfg(windows)]
        "Documents/WindowsPowerShell",
    ]
    .iter()
    .map(|path| home.join(path))
    .collect();
    #[cfg(target_os = "macos")]
    paths.push(home.join("Library/LaunchAgents"));
    #[cfg(target_os = "linux")]
    paths.extend(startup_directories(home));
    #[cfg(windows)]
    paths.push(home.join("AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup"));
    paths
}

pub(super) fn additional_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    // These are captured from the operator, never from the child environment.
    for name in ["GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM"] {
        if let Some(path) = std::env::var_os(name).filter(|path| !path.is_empty()) {
            let path = PathBuf::from(path);
            // A neutral device cannot hold planted persistent configuration.
            #[cfg(windows)]
            let neutral = path
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("NUL");
            #[cfg(not(windows))]
            let neutral = path == Path::new("/dev/null");
            if !neutral {
                paths.push(path);
            }
        }
    }
    #[cfg(target_os = "linux")]
    paths.extend(additional_startup_directories());
    #[cfg(windows)]
    for name in ["ProgramData", "APPDATA"] {
        if let Some(root) = std::env::var_os(name) {
            paths.push(PathBuf::from(root).join("Microsoft/Windows/Start Menu/Programs/Startup"));
        }
    }
    paths
        .into_iter()
        .filter_map(|path| match std::path::absolute(&path) {
            Ok(path) => Some(path),
            Err(error) => { tracing::warn!(path = %path.display(), %error, "operator config path could not be resolved"); None }
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn absolute_xdg(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).and_then(|value| xdg_path(PathBuf::from(value)))
}

#[cfg(any(test, target_os = "linux"))]
fn xdg_path(path: PathBuf) -> Option<PathBuf> {
    path.is_absolute().then_some(path)
}

fn defaults(home: &Path) -> [PathBuf; 2] {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    [home.join(".gitconfig"), root.join("git/config")]
}

pub(super) fn write_paths() -> Vec<PathBuf> {
    let mut paths = user_paths(&crate::credential_paths::home());
    for name in ["HOME", "USERPROFILE"] {
        if let Some(home) = std::env::var_os(name) {
            paths.extend(user_paths(Path::new(&home)));
            paths.extend(defaults(Path::new(&home)));
        }
    }
    paths.extend(defaults(&crate::credential_paths::home()));
    paths.extend(additional_paths());
    paths.extend(crate::spawn::host_protected_paths());
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(target_os = "linux")]
pub fn prepared_write_paths() -> Result<Vec<PathBuf>, String> {
    crate::write_boundary::landlock::LandlockSandbox::probe_seals()?;
    prepare().map(|(paths, _)| paths)
}

#[cfg(target_os = "linux")]
#[path = "read_boundary_prepare.rs"]
mod preparation;

#[cfg(target_os = "linux")]
pub(super) fn prepare() -> Result<(Vec<PathBuf>, Vec<String>), String> {
    // Missing paths are created only when preparation can preserve their semantics.
    let mut notes = Vec::new();
    let mut homes = vec![crate::credential_paths::home()];
    for name in ["HOME", "USERPROFILE"] {
        if let Some(home) = std::env::var_os(name) {
            homes.push(PathBuf::from(home));
        }
    }
    homes.sort();
    homes.dedup();
    let mut candidates = Vec::new();
    let mut directories = additional_startup_directories();
    let mut explicit = Vec::new();
    for home in homes {
        let [primary, secondary] = defaults(&home);
        if primary.exists() {
            explicit.push(secondary);
        } else if !secondary.exists() {
            explicit.push(primary);
        } else {
            notes.push(format!(
                "absent {} not sealed (preserve configuration precedence)",
                primary.display()
            ));
        }
        candidates.extend(defaults(&home));
        candidates.extend(user_paths(&home));
        directories.extend(startup_directories(&home));
    }
    candidates.extend(additional_paths());
    candidates.extend(super::operator_paths());
    for path in directories {
        match path.try_exists() {
            Ok(true) => {}
            Ok(false) if preparation::inside_work(&path) => {
                notes.push(format!(
                    "absent {} not sealed (inside work tree)",
                    path.display()
                ));
            }
            Ok(false) => {
                if let Err(reason) = prepare_private_directory(&path) {
                    notes.push(reason);
                }
            }
            Err(error) => notes.push(format!("{} not sealed ({error})", path.display())),
        }
    }
    let mut paths = Vec::new();
    for path in candidates {
        match path.try_exists() {
            Ok(true) => paths.push(path),
            Ok(false) => notes.push(format!("absent {} not sealed", path.display())),
            Err(error) => notes.push(format!("{} not sealed ({error})", path.display())),
        }
    }
    // Materialize absent explicit files before constructing their seals.
    for name in ["GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM"] {
        if let Some(path) = std::env::var_os(name).filter(|path| !path.is_empty()) {
            let path = match std::path::absolute(&path) {
                Ok(path) => path,
                Err(error) => {
                    notes.push(format!(
                        "{} not sealed ({error})",
                        Path::new(&path).display()
                    ));
                    continue;
                }
            };
            if path != Path::new("/dev/null") {
                explicit.push(path);
            }
        }
    }
    explicit.extend(crate::spawn::host_protected_paths());
    for path in explicit {
        if preparation::materialize(&path, &mut notes) {
            paths.push(path);
        }
    }
    let grant_root = crate::credential_paths::operator_grant_root();
    if !grant_root.exists() && preparation::inside_work(&grant_root) {
        notes.push(format!(
            "absent {} not sealed (inside work tree)",
            grant_root.display()
        ));
    } else if let Err(reason) = prepare_private_directory(&grant_root) {
        notes.push(reason);
    } else {
        paths.push(grant_root);
    }
    paths.sort();
    paths.dedup();
    notes.retain(|note| {
        !paths
            .iter()
            .any(|path| note == &format!("absent {} not sealed", path.display()))
    });
    Ok((paths, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_roots_must_be_nonempty_and_absolute() {
        assert!(xdg_path(PathBuf::new()).is_none());
        assert!(xdg_path(PathBuf::from("relative/config")).is_none());
        let root = std::env::temp_dir();
        assert_eq!(xdg_path(root.clone()), Some(root));
    }

    #[test]
    fn default_configuration_locations_receive_write_rules() {
        let paths = write_paths();
        for path in defaults(&crate::credential_paths::home()) {
            assert!(paths.contains(&path), "{}", path.display());
        }
    }

    #[test]
    fn persistent_configuration_paths_are_write_only() {
        let home = Path::new("synthetic-home");
        let paths = user_paths(home);
        for path in user_paths(home) {
            assert!(
                !crate::read_boundary::denied_paths(home).contains(&path),
                "{}",
                path.display()
            );
        }
        for name in [".profile", ".bashrc", ".zshrc"] {
            assert!(paths.contains(&home.join(name)));
        }
        #[cfg(target_os = "macos")]
        assert!(paths.contains(&home.join("Library/LaunchAgents")));
        #[cfg(target_os = "linux")]
        for path in [
            ".config/systemd",
            ".local/share/systemd",
            ".config/environment.d",
            ".config/autostart",
        ] {
            assert!(paths.contains(&home.join(path)));
        }
        #[cfg(windows)]
        {
            assert!(paths.contains(
                &home.join("AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup")
            ));
            let (_, report) =
                crate::read_boundary::command_with_report(std::ffi::OsStr::new("probe"));
            assert!(report.contains("Startup folders and registry Run keys are not contained"));
            assert!(report.contains("not-contained"));
        }
    }
}

#[cfg(all(test, unix))]
mod gone_cwd_tests {
    #[test]
    fn config_paths_do_not_panic_when_current_directory_is_gone() {
        const PROBE: &str = "ARCHON_CONFIG_GONE_CWD_TEST";
        if std::env::var_os(PROBE).is_none() {
            let output = crate::spawn::command(std::env::current_exe().unwrap())
                .args(["--exact", "read_boundary::config::gone_cwd_tests::config_paths_do_not_panic_when_current_directory_is_gone", "--nocapture"])
                .env(PROBE, "1").env("GIT_CONFIG_GLOBAL", "relative-global.config")
                .env("XDG_CONFIG_HOME", "relative-xdg").output().unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        std::env::set_current_dir(directory.path()).unwrap();
        std::fs::remove_dir(directory.path()).unwrap();
        assert!(std::env::current_dir().is_err());
        assert!(
            super::additional_paths()
                .iter()
                .all(|path| path.is_absolute())
        );
    }
}

#[cfg(target_os = "linux")]
fn prepare_private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .and_then(|()| std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)))
        .map_err(|error| {
            format!(
                "private directory {} could not be prepared: {error}",
                path.display()
            )
        })
}

#[cfg(target_os = "linux")]
fn startup_directories(home: &Path) -> Vec<PathBuf> {
    [
        ".config/systemd",
        ".local/share/systemd",
        ".config/environment.d",
        ".config/autostart",
    ]
    .iter()
    .map(|path| home.join(path))
    .collect()
}

#[cfg(target_os = "linux")]
fn additional_startup_directories() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(root) = absolute_xdg("XDG_CONFIG_HOME") {
        paths.extend([
            root.join("systemd"),
            root.join("environment.d"),
            root.join("autostart"),
        ]);
    }
    if let Some(root) = absolute_xdg("XDG_DATA_HOME") {
        paths.push(root.join("systemd"));
    }
    paths
}

#[cfg(all(test, target_os = "linux"))]
fn prepare_configuration_file(path: &Path) -> Result<(), String> {
    preparation::materialize(path, &mut Vec::new())
        .then_some(())
        .ok_or_else(|| "not sealed".into())
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn absent_explicit_file_is_created_privately_without_following_links() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("parent/nested/config");
    assert!(prepare_configuration_file(&path).is_err());
    assert!(!path.parent().unwrap().exists());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    prepare_configuration_file(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let link = temp.path().join("link");
    let target = temp.path().join("absent");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    prepare_configuration_file(&link).unwrap();
    assert!(link.is_symlink());
    assert!(!target.exists());
}
