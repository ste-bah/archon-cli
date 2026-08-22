//! The environment handed to spawned shells.
//!
//! Two different environments, because there are two different callers.
//!
//! **Agent shells** get the host environment minus Archon's own credentials.
//! There is no allowlist of permitted names: an allowlist can only name what
//! this crate's authors predicted, and everything else is dropped *silently* —
//! invisible from inside the child, which reads `std::env::var`, sees nothing,
//! and reports the honest but wrong conclusion that the operator never set the
//! value. It cost a working data pipeline to learn that: a list naming `PATH`,
//! `HOME` and thirty-odd OS variables stripped a downstream project's API key
//! and service URL, its capability probe reported credentials missing, and its
//! fail-closed guard correctly refused to write anything.
//!
//! What IS withheld is the short, knowable set of credentials belonging to
//! Archon itself — [`ENGINE_CREDENTIAL_VARS`]. The engine owns those names by
//! definition, so telling them apart from a downstream project's credentials
//! needs no cleverness and no configuration. An agent has no business reading
//! the key that pays for it, and `bash_sensitive_env_stripped` says so.
//!
//! **Hooks** get [`isolated_env`] instead: a strict allowlist of OS process
//! variables and nothing else. A hook is user-configured and fires
//! automatically rather than being work an agent asked for, so Issue #92 gives
//! it an explicit context and no inherited state. Both spawn paths shared one
//! function once, which is how a change aimed at agent shells silently widened
//! what hooks could see.
//!
//! Materializing the environment as a `Vec` rather than letting the child
//! inherit it implicitly is what makes either policy enforceable: callers layer
//! defaults and overlays on top, and an explicit vector paired with
//! `Command::env_clear` makes the child's environment exactly what those layers
//! computed, on every platform.

/// Credentials belonging to Archon itself, withheld from every spawned child.
///
/// Not a general secret filter — a pattern rule over names like `*_KEY` would
/// catch a downstream project's provider credentials too, which is the whole
/// thing the passthrough exists to allow. These are only the names this engine
/// authenticates with, matched case-insensitively.
pub const ENGINE_CREDENTIAL_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ARCHON_API_KEY",
    "ARCHON_OAUTH_TOKEN",
    "ARCHON_DOCS_OPENAIKEY",
    "ARCHON_MEMORY_OPENAIKEY",
];

/// OS process variables a hook may inherit, and nothing else (Issue #92).
const HOOK_PASSTHROUGH_VARS: &[&str] = &[
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
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "CARGO_HOME",
    "RUSTUP_HOME",
];

/// The host environment minus Archon's own credentials, for agent shells.
pub fn host_env() -> Vec<(String, String)> {
    collect_env(std::env::vars())
}

/// OS process variables only, for hooks (Issue #92).
pub fn isolated_env() -> Vec<(String, String)> {
    allowlist_env(std::env::vars())
}

fn collect_env<K, V>(vars: impl IntoIterator<Item = (K, V)>) -> Vec<(String, String)>
where
    K: Into<String>,
    V: Into<String>,
{
    vars.into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .filter(|(key, _)| {
            !ENGINE_CREDENTIAL_VARS
                .iter()
                .any(|owned| owned.eq_ignore_ascii_case(key))
        })
        .collect()
}

fn allowlist_env<K, V>(vars: impl IntoIterator<Item = (K, V)>) -> Vec<(String, String)>
where
    K: Into<String>,
    V: Into<String>,
{
    vars.into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .filter(|(key, _)| {
            let upper = key.to_uppercase();
            HOOK_PASSTHROUGH_VARS.contains(&upper.as_str())
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

/// Set `key` to `value`, replacing any existing entry regardless of case.
///
/// The counterpart to [`ensure_env_default`], for values the engine computes
/// and the host cannot be right about. The workflow run id is the case that
/// forced it into existence: the child now inherits the host environment, and
/// an operator with a run-id variable left over in their shell profile would
/// otherwise have that stale value win over the id of the run actually
/// executing — silently, since a plausible-looking id reads as success.
pub(crate) fn set_env_override(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some((_, existing)) = env
        .iter_mut()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
    {
        *existing = value.to_string();
        return;
    }
    env.push((key.to_string(), value.to_string()));
}

#[cfg(test)]
mod tests {
    use super::{allowlist_env, collect_env};

    /// The regression the agent-shell path exists to prevent: variables the
    /// engine's authors never predicted must reach the child. Every one of
    /// these was silently dropped by the allowlist that used to apply here.
    #[test]
    fn unpredicted_host_variables_reach_the_agent_shell() {
        let host = [
            ("PATH", "/usr/bin"),
            ("POLYGON_API_KEY", "provider-credential"),
            ("OPENBB_API_URL", "http://127.0.0.1:6900"),
            ("ARCHON_STOOQ_CSV_URL", "http://example.invalid/csv"),
            ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
            ("HARMLESS_CUSTOM_FLAG", "enabled"),
        ];
        let env = collect_env(host);
        for (key, value) in host {
            assert!(
                env.iter().any(|(k, v)| k == key && v == value),
                "{key} must reach the agent shell: {env:?}"
            );
        }
    }

    /// The engine's own credentials never do — an agent has no business
    /// reading the key that pays for it. Matched case-insensitively so a
    /// casing variant cannot slip through.
    #[test]
    fn engine_credentials_are_withheld_from_the_agent_shell() {
        let env = collect_env([
            ("ANTHROPIC_API_KEY", "sk-secret"),
            ("anthropic_api_key", "sk-secret-lowercase"),
            ("ARCHON_OAUTH_TOKEN", "tok"),
            ("POLYGON_API_KEY", "provider-credential"),
        ]);
        assert_eq!(
            env,
            vec![(
                "POLYGON_API_KEY".to_string(),
                "provider-credential".to_string()
            )],
            "only the non-engine credential survives"
        );
    }

    /// Hooks stay on the Issue #92 allowlist: OS process variables only.
    #[test]
    fn hook_environment_is_os_variables_only() {
        let env = allowlist_env([
            ("PATH", "/usr/bin"),
            ("PSModulePath", r"C:\Modules"),
            ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            ("POLYGON_API_KEY", "provider-credential"),
            ("ISSUE92_FORBIDDEN", "must-not-reach-hooks"),
        ]);
        let kept: Vec<&str> = env.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(kept, vec!["PATH", "PSModulePath", "ProgramFiles(x86)"]);
    }

    #[test]
    fn env_defaults_do_not_duplicate_windows_case_variants() {
        let mut env = vec![("Path".to_string(), r"C:\Windows".to_string())];
        super::ensure_env_default(&mut env, "PATH", "unexpected");
        assert_eq!(env, vec![("Path".to_string(), r"C:\Windows".to_string())]);
    }

    #[test]
    fn overrides_replace_an_existing_value_across_case_variants() {
        let mut env = vec![("Path".to_string(), r"C:\Windows".to_string())];
        super::set_env_override(&mut env, "PATH", "/usr/bin");
        assert_eq!(env, vec![("Path".to_string(), "/usr/bin".to_string())]);
    }

    #[test]
    fn overrides_insert_when_absent() {
        let mut env = Vec::new();
        super::set_env_override(&mut env, "ARCHON_WORKFLOW_RUN_ID", "wf-1");
        assert_eq!(
            env,
            vec![("ARCHON_WORKFLOW_RUN_ID".to_string(), "wf-1".to_string())]
        );
    }
}
