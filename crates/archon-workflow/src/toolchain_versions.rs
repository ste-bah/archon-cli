//! Version adapters complement executable content identity for runtime shims.
use std::collections::BTreeMap;
use std::path::Path;

const QUERIES: &[(&str, &[&str])] = &[
    ("cargo", &["-V"]),
    ("rustc", &["-Vv"]),
    ("node", &["--version"]),
    ("npm", &["--version"]),
    ("python", &["--version"]),
    ("python3", &["--version"]),
    ("go", &["version"]),
    ("java", &["-version"]),
    ("javac", &["-version"]),
    ("dotnet", &["--version"]),
    ("ruby", &["--version"]),
    ("php", &["--version"]),
    ("bun", &["--version"]),
    ("deno", &["--version"]),
];

/// Queries run only for programs present at the site's PATH. Both output
/// streams matter: some version adapters write exclusively to stderr.
pub fn fingerprint(environment: &BTreeMap<String, String>, cwd: &Path) -> BTreeMap<String, String> {
    let search_path = environment.get("PATH").map(String::as_str).unwrap_or("");
    let mut identity = crate::toolchain_identity::fingerprint(
        search_path.as_ref(),
        crate::task_set_contract::content_digest,
    );
    for (name, args) in QUERIES {
        let executable = std::env::split_paths(search_path)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file());
        let Some(executable) = executable else {
            continue;
        };
        let mut command = archon_shell::spawn::command(executable);
        let output = archon_shell::spawn::replace_environment(&mut command, environment)
            .args(*args)
            .current_dir(cwd)
            .output();
        let version = output
            .ok()
            .filter(|out| out.status.success())
            .map(|out| {
                format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                )
            })
            .unwrap_or_else(|| "version unavailable".into());
        identity.insert(
            format!("version:{name}"),
            crate::task_set_contract::content_digest(version.as_bytes()),
        );
    }
    identity
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn unchanged_shim_still_invalidates_when_its_selected_runtime_changes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let selected = dir.path().join("selected-version");
        let shim = dir.path().join("node");
        std::fs::write(&shim, "#!/bin/sh\n/bin/cat \"$SELECTED_VERSION\" >&2\n").unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let environment = BTreeMap::from([
            ("PATH".into(), dir.path().display().to_string()),
            ("SELECTED_VERSION".into(), selected.display().to_string()),
        ]);
        std::fs::write(&selected, "runtime-one").unwrap();
        let before = fingerprint(&environment, dir.path());
        std::fs::write(&selected, "runtime-two").unwrap();
        let after = fingerprint(&environment, dir.path());
        assert_eq!(
            before[&shim.display().to_string()],
            after[&shim.display().to_string()]
        );
        assert_ne!(before["version:node"], after["version:node"]);
        assert_eq!(before.len(), 2, "no absent tool should be queried");
    }
}
