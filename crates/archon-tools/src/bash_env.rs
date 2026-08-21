//! The environment handed to spawned shells.
//!
//! Every spawned child gets the host environment, verbatim. There is no
//! compiled-in list of variable names here, and deliberately so: a hardcoded
//! allowlist can only ever name what this crate's authors happened to predict,
//! and every variable they did not predict is dropped *silently*. That failure
//! is invisible from inside the child — a command reads `std::env::var` and
//! sees nothing, so it reports the honest but wrong conclusion that the
//! operator never set the value.
//!
//! It cost a working data pipeline to learn that. An allowlist naming `PATH`,
//! `HOME` and thirty-odd OS variables stripped a downstream project's API key
//! and service URL from every agent shell. That project's capability probe then
//! reported credentials missing and its fail-closed guard correctly refused to
//! write anything — a correct decision reached from starved input. The same
//! list had already grown three of that project's variable names inside
//! `archon-core`, which is what a hardcoded registry does under pressure: it
//! accretes its callers' domain, one patch at a time.
//!
//! Materializing the host environment as a `Vec` rather than letting the child
//! inherit it implicitly is still worth doing. Callers layer defaults and
//! provider overlays on top of what this returns, and an explicit vector paired
//! with `Command::env_clear` makes the child's environment exactly what those
//! layers computed, on every platform, rather than whatever the parent happened
//! to hold.

/// The host environment, as name/value pairs, in no particular order.
pub fn host_env() -> Vec<(String, String)> {
    collect_env(std::env::vars())
}

fn collect_env<K, V>(vars: impl IntoIterator<Item = (K, V)>) -> Vec<(String, String)>
where
    K: Into<String>,
    V: Into<String>,
{
    vars.into_iter()
        .map(|(key, value)| (key.into(), value.into()))
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
    use super::collect_env;

    /// The regression this module exists to prevent.
    ///
    /// Each of these was dropped by the allowlist that used to live here. The
    /// provider keys are the ones that broke the data lake; the Windows
    /// toolchain paths are the ones whose absence surfaced as a linker error
    /// rather than a missing-variable error, because rustc could not run
    /// `vswhere.exe` and fell back to a bare `link.exe` that resolved to Git's
    /// coreutils `link`.
    #[test]
    fn every_host_variable_reaches_the_child() {
        let host = [
            ("PATH", "/usr/bin"),
            ("Path", r"C:\Windows"),
            ("HOME", "/home/test"),
            ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
            ("POLYGON_API_KEY", "secret"),
            ("OPENBB_API_URL", "http://127.0.0.1:6900"),
            ("ARCHON_STOOQ_CSV_URL", "http://example.invalid/csv"),
            (
                "PSModulePath",
                r"C:\Program Files\WindowsPowerShell\Modules",
            ),
            ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            ("HARMLESS_CUSTOM_FLAG", "enabled"),
        ];

        let env = collect_env(host);

        assert_eq!(env.len(), host.len());
        for (key, value) in host {
            assert!(
                env.iter()
                    .any(|(got_key, got_value)| got_key == key && got_value == value),
                "{key} must reach the child: {env:?}"
            );
        }
    }

    /// Values are forwarded byte-for-byte. A key whose value contains `=`, a
    /// newline, or non-ASCII is common in real environments and must not be
    /// reshaped on the way through.
    #[test]
    fn values_are_forwarded_verbatim() {
        let env = collect_env([
            ("WITH_EQUALS", "a=b=c"),
            ("WITH_NEWLINE", "line one\nline two"),
            ("WITH_UNICODE", "café ☕"),
            ("EMPTY", ""),
        ]);

        assert_eq!(
            env,
            vec![
                ("WITH_EQUALS".to_string(), "a=b=c".to_string()),
                ("WITH_NEWLINE".to_string(), "line one\nline two".to_string()),
                ("WITH_UNICODE".to_string(), "café ☕".to_string()),
                ("EMPTY".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn env_defaults_do_not_duplicate_windows_case_variants() {
        let mut env = vec![("Path".to_string(), r"C:\Windows".to_string())];
        super::ensure_env_default(&mut env, "PATH", "unexpected");
        assert_eq!(env, vec![("Path".to_string(), r"C:\Windows".to_string())]);
    }

    /// `host_env` reads the real process environment, not a fabricated one.
    ///
    /// Asserts a variable every platform sets rather than comparing counts:
    /// Rust runs tests on threads of one process, so a concurrent test that
    /// sets or removes a variable would make a count equality flake.
    #[test]
    fn host_env_reads_the_real_process_environment() {
        let env = super::host_env();
        assert!(
            env.iter().any(|(key, _)| key.eq_ignore_ascii_case("PATH")),
            "PATH must come from the real environment: {env:?}"
        );
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
