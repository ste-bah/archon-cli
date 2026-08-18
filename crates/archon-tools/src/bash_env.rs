const PASSTHROUGH_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "USERNAME",
    "USERPROFILE",
    "SHELL",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_COLLATE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "FORCE_COLOR",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "EDITOR",
    "VISUAL",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "PSMODULEPATH",
    // Where Windows keeps installed toolchains. rustc locates the MSVC linker
    // by running `vswhere.exe` out of `%ProgramFiles(x86)%`, so without these
    // the lookup fails and it falls back to invoking a bare `link.exe` off
    // PATH. On any machine with Git installed that resolves to Git's coreutils
    // `link`, which answers with "link: extra operand" and a failed build —
    // the symptom looked like a linker bug and was an environment hole.
    //
    // Same category as SYSTEMROOT and COMSPEC above: fixed OS paths, not
    // credentials.
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "CARGO_HOME",
    "RUSTUP_HOME",
];

pub fn sanitized_env() -> Vec<(String, String)> {
    sanitize_env(std::env::vars())
}

fn sanitize_env<K, V>(vars: impl IntoIterator<Item = (K, V)>) -> Vec<(String, String)>
where
    K: Into<String>,
    V: Into<String>,
{
    vars.into_iter()
        .filter_map(|(key, value)| {
            let key = key.into();
            let upper = key.to_uppercase();
            let allowed = PASSTHROUGH_VARS.contains(&upper.as_str());
            allowed.then(|| (key, value.into()))
        })
        .collect()
}

/// Set `key` to `value` only if it is not already present.
///
/// Case-insensitive on purpose: Windows environment blocks carry `Path`, not
/// `PATH`, and a case-sensitive check would append a second entry that shadows
/// the first. `workflow_resource_env` shares this rather than keeping its own
/// copy, so the resource defaults get the same Windows behaviour.
pub(crate) fn ensure_env_default(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if !env
        .iter()
        .any(|(existing, _)| existing.eq_ignore_ascii_case(key))
    {
        env.push((key.to_string(), value.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_env;

    #[test]
    fn strict_allowlist_excludes_unknown_variables_and_ssh_agent() {
        let env = sanitize_env([
            ("PATH", "/usr/bin"),
            ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
            ("UNRECOGNIZED_CREDENTIAL", "secret"),
            ("LC_AWS_SECRET_ACCESS_KEY", "secret"),
            ("HARMLESS_CUSTOM_FLAG", "enabled"),
        ]);

        assert_eq!(env, vec![("PATH".to_string(), "/usr/bin".to_string())]);
    }

    #[test]
    fn strict_allowlist_preserves_windows_powershell_module_path() {
        let env = sanitize_env([
            (
                "PSModulePath",
                r"C:\\Program Files\\WindowsPowerShell\\Modules",
            ),
            ("UNRECOGNIZED_CREDENTIAL", "secret"),
        ]);

        assert_eq!(
            env,
            vec![(
                "PSModulePath".to_string(),
                r"C:\\Program Files\\WindowsPowerShell\\Modules".to_string(),
            )]
        );
    }

    #[test]
    fn env_defaults_do_not_duplicate_windows_case_variants() {
        let mut env = vec![("Path".to_string(), r"C:\\Windows".to_string())];
        super::ensure_env_default(&mut env, "PATH", "unexpected");
        assert_eq!(env, vec![("Path".to_string(), r"C:\\Windows".to_string())]);
    }

    /// Toolchain discovery on Windows goes through `%ProgramFiles(x86)%`.
    ///
    /// Dropping these did not produce a missing-variable error — it produced a
    /// linker error. rustc could not run `vswhere.exe`, gave up on locating
    /// MSVC, and invoked a bare `link.exe`, which on any machine with Git
    /// installed resolves to Git's coreutils `link` and fails with "extra
    /// operand". Three `command_evidence` tests failed that way on every
    /// Windows CI run, looking like a linker bug rather than an environment
    /// hole, so this is pinned by name.
    #[test]
    fn strict_allowlist_preserves_windows_toolchain_discovery_paths() {
        let env = sanitize_env([
            ("ProgramFiles", r"C:\Program Files"),
            ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            ("ProgramW6432", r"C:\Program Files"),
            ("ProgramData", r"C:\ProgramData"),
            ("UNRECOGNIZED_CREDENTIAL", "secret"),
        ]);

        let kept: Vec<&str> = env.iter().map(|(key, _)| key.as_str()).collect();
        for required in [
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "ProgramData",
        ] {
            assert!(
                kept.contains(&required),
                "{required} must survive: {kept:?}"
            );
        }
        assert!(!kept.contains(&"UNRECOGNIZED_CREDENTIAL"));
    }

    #[test]
    fn strict_allowlist_preserves_required_unix_and_windows_process_vars() {
        let env = sanitize_env([
            ("Path", r"C:\\Windows"),
            ("HOME", "/home/test"),
            ("LANG", "en_US.UTF-8"),
            ("SYSTEMROOT", r"C:\\Windows"),
            ("COMSPEC", r"C:\\Windows\\System32\\cmd.exe"),
            ("PATHEXT", ".COM;.EXE"),
            ("USERPROFILE", r"C:\\Users\\test"),
        ]);

        assert_eq!(env.len(), 7);
        for required in ["Path", "SYSTEMROOT", "COMSPEC", "PATHEXT", "USERPROFILE"] {
            assert!(env.iter().any(|(key, _)| key == required));
        }
    }
}
