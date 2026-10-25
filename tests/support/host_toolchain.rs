use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Find cargo on a configured policy search path.
#[allow(dead_code)]
pub(crate) fn cargo_on_path(path: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join("cargo"))
        .find(|candidate| candidate.is_file())
}

/// Resolve cargo for bin tests that run with an empty HOME.
#[allow(dead_code)]
pub(crate) fn bin_host_cargo() -> Option<PathBuf> {
    if let Some(cargo) = std::env::var_os("CARGO").map(PathBuf::from) {
        if let Some(found) =
            resolve_name_or_path(cargo.as_os_str()).filter(|path| !is_rustup_proxy(path))
        {
            return Some(found);
        }
    }

    let toolchain = Command::new("rustup")
        .args(["which", "cargo"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
        .filter(|cargo| cargo.is_file() && !is_rustup_proxy(cargo));
    if toolchain.is_some() {
        return toolchain;
    }

    let search = std::env::var_os("PATH")?;
    std::env::split_paths(&search)
        .map(|dir| dir.join("cargo"))
        .find(|candidate| candidate.is_file() && !is_rustup_proxy(candidate))
}

fn resolve_name_or_path(cargo: &std::ffi::OsStr) -> Option<PathBuf> {
    let path = PathBuf::from(cargo);
    if path.components().count() > 1 {
        return path.is_file().then_some(path);
    }
    let search = std::env::var_os("PATH")?;
    std::env::split_paths(&search)
        .map(|dir| dir.join(&path))
        .find(|candidate| candidate.is_file())
}

fn is_rustup_proxy(path: &std::path::Path) -> bool {
    let named_proxy = path
        .canonicalize()
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_owned()))
        .is_some_and(|name| name == "rustup" || name == "rustup-init");
    #[cfg(unix)]
    let linked_proxy = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|dir| dir.join("rustup"))
        .find(|candidate| candidate.is_file())
        .is_some_and(|rustup| same_file_identity(path, &rustup));
    #[cfg(unix)]
    {
        named_proxy || linked_proxy
    }
    #[cfg(not(unix))]
    {
        named_proxy
    }
}

#[cfg(unix)]
fn same_file_identity(left: &std::path::Path, right: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.metadata()
        .ok()
        .zip(right.metadata().ok())
        .is_some_and(|(left, right)| left.dev() == right.dev() && left.ino() == right.ino())
}

/// A leak of host HOME or RUSTUP_HOME changes the answer through these files,
/// so the listing isolation test fails when either host value is exposed.
/// This is a deterministic fake proxy for tests that inspect listing failures.
#[allow(dead_code)]
pub(crate) fn fake_rustup_proxy() -> PathBuf {
    static FIXTURE: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = FIXTURE.get_or_init(|| {
        let dir = tempfile::tempdir().expect("proxy fixture directory");
        let script = dir.path().join("cargo");
        let source = "#!/bin/sh
rustup_home=\"${RUSTUP_HOME:-}\"
if [ -z \"$rustup_home\" ]; then rustup_home=\"${HOME:-}/.rustup\"; fi
if [ ! -f \"$rustup_home/settings.toml\" ]; then
  echo \"error: rustup could not choose a version of cargo to run, because one wasn't specified explicitly, and no default is configured.\" >&2; exit 1
fi
if [ -n \"${RUSTUP_TOOLCHAIN:-}\" ]; then
  echo \"error: toolchain '$RUSTUP_TOOLCHAIN' is not installed\" >&2; exit 1
fi
if [ \"$1\" = --list ]; then
  printf 'Commands:\\n    build    Build\\n    test     Run tests\\n'; exit 0
fi
echo 'error: no such command: `nextest`' >&2; exit 101
";
        std::fs::write(&script, source).expect("write fake proxy fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("make fake proxy executable");
        }
        dir
    });
    dir.path().join("cargo")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_proxy_uses_home_when_rustup_home_is_empty() {
        let proxy = fake_rustup_proxy();
        let home = tempfile::tempdir().unwrap();
        let rustup = home.path().join(".rustup");
        std::fs::create_dir_all(&rustup).unwrap();
        std::fs::write(rustup.join("settings.toml"), "").unwrap();
        let output = Command::new(proxy)
            .arg("--list")
            .env("HOME", home.path())
            .env("RUSTUP_HOME", "")
            .env_remove("RUSTUP_TOOLCHAIN")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_is_identified_by_device_and_inode() {
        let dir = tempfile::tempdir().unwrap();
        let rustup = dir.path().join("rustup");
        let link = dir.path().join("cargo");
        std::fs::write(&rustup, "fixture").unwrap();
        std::fs::hard_link(&rustup, &link).unwrap();
        assert!(same_file_identity(&link, &rustup));
    }
}
