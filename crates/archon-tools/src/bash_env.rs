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

pub(super) fn ensure_env_default(env: &mut Vec<(String, String)>, key: &str, value: &str) {
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
