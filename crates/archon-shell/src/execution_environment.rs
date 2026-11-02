//! Code-owned environment for agent-authored programs. This is deliberately
//! separate from host provider transports, which require their own credentials.
use std::ffi::{OsStr, OsString};
use std::process::Command;

const INHERITED: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "USERNAME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "HOMEDRIVE",
    "HOMEPATH",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_COLLATE",
    "LC_MESSAGES",
    "LC_MONETARY",
    "LC_NUMERIC",
    "LC_TIME",
    "TZ",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "JAVA_HOME",
    "DOTNET_ROOT",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "MAKEFLAGS",
    "MFLAGS",
    "GNUMAKEFLAGS",
    "CARGO_MAKEFLAGS",
];

/// Only exact, case-insensitive safe names can inherit ambient values.
pub fn inherited(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter(|(key, _)| {
            key.to_str()
                .is_some_and(|key| INHERITED.iter().any(|safe| safe.eq_ignore_ascii_case(key)))
        })
        .filter_map(|(key, value)| {
            let build_flags = ["MAKEFLAGS", "MFLAGS", "GNUMAKEFLAGS", "CARGO_MAKEFLAGS"]
                .iter()
                .any(|name| key.to_string_lossy().eq_ignore_ascii_case(name));
            if build_flags
                && value.to_string_lossy().split_whitespace().any(|word| {
                    word.split_once('=').is_some_and(|(name, _)| {
                        credential_name(name) || name.to_ascii_uppercase().ends_with("_AUTH")
                    })
                })
            {
                return None;
            }
            let value = crate::jobserver::sanitize_variable(&key, value);
            Some((key, value))
        })
        .collect()
}

/// The environment before launch-owned bindings are added. Never log values.
pub fn environment() -> Vec<(OsString, OsString)> {
    inherited(std::env::vars_os())
}

/// Apply the shared policy. Add explicit launch-owned bindings AFTER this call.
pub fn apply(command: &mut Command) {
    apply_mode(command, Mode::Inherited);
}

/// Exact mode preserves the caller's complete, already validated site policy.
pub enum Mode<'a> {
    Inherited,
    Exact(&'a std::collections::BTreeMap<String, String>),
}

pub fn apply_mode(command: &mut Command, mode: Mode<'_>) {
    match mode {
        Mode::Inherited => crate::spawn::replace_environment(command, environment()),
        Mode::Exact(bindings) => crate::spawn::replace_environment(command, bindings),
    };
}

/// An agent command with the environment and available read boundary applied.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    command_with_mode(program, Mode::Inherited)
}

pub fn command_with_mode(program: impl AsRef<OsStr>, mode: Mode<'_>) -> Command {
    let mut command = crate::read_boundary::command(program.as_ref());
    apply_mode(&mut command, mode);
    command
}

/// Async counterpart, with identical policy.
pub fn tokio_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    tokio::process::Command::from(command(program))
}

/// Host-client connection bindings; never passed into the agent payload.
pub const SSH_CLIENT_BINDINGS: &[&str] = &["SSH_AUTH_SOCK"];
/// Docker CLI routing and authentication locations, used by the local client.
pub const DOCKER_CLIENT_BINDINGS: &[&str] = &[
    "DOCKER_HOST",
    "DOCKER_CONTEXT",
    "DOCKER_CONFIG",
    "DOCKER_TLS_VERIFY",
    "DOCKER_CERT_PATH",
];

/// A local transport client needs host authentication, but must not forward
/// arbitrary exports. Callers name only the client-owned connection bindings.
/// Filesystem isolation of the client would break its host authentication;
/// the remote payload's isolation is a separate backend responsibility.
pub fn transport_environment(command: &mut Command, bindings: &[&str]) {
    apply(command);
    for name in bindings {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    tracing::warn!(
        environment_policy = "safe-allowlist+transport-bindings",
        credential_reads = "host-client-authentication",
        process_reads = "not-contained",
        "agent transport client launch: local read isolation unavailable; payload isolation is backend-owned"
    );
}

/// A payload selected by an existing forwarding allowlist still cannot carry
/// credentials. Keep this rule shared with the inherited environment policy.
pub fn credential_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["TOKEN", "SECRET", "KEY", "PASSWORD", "CREDENTIAL"]
        .iter()
        .any(|part| upper.contains(part))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jobserver_flags_survive_without_descriptors_or_credential_assignments() {
        let vars = inherited([
            (
                "MAKEFLAGS".into(),
                "-- ARCHON_TEST_SECRET=sk-test-355".into(),
            ),
            ("MFLAGS".into(), "--jobserver-auth=3,4 --silent".into()),
            (
                "GNUMAKEFLAGS".into(),
                "--jobserver-auth=fifo:/tmp/archon-jobserver -- X=keep".into(),
            ),
        ]);
        assert!(!vars.iter().any(|(key, _)| key == "MAKEFLAGS"));
        assert!(
            vars.iter()
                .any(|(key, value)| key == "MFLAGS" && value == "--silent")
        );
        assert!(vars.iter().any(|(key, value)| key == "GNUMAKEFLAGS"
            && value == "--jobserver-auth=fifo:/tmp/archon-jobserver -- X=keep"));
    }

    #[test]
    fn exact_environment_preserves_site_bindings_without_ambient_defaults() {
        let site = std::collections::BTreeMap::from([
            ("PATH".into(), "/site/bin".into()),
            ("HOME".into(), "/site/home".into()),
            ("CARGO_HOME".into(), "/site/cargo".into()),
        ]);
        let command = command_with_mode("site-check", Mode::Exact(&site));
        let actual = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.unwrap().to_string_lossy().into_owned(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(actual, site);
    }

    #[test]
    fn only_safe_names_inherit() {
        let vars = [
            "Path",
            "HOME",
            "LANG",
            "TMPDIR",
            "ARCHON_TEST_SECRET",
            "OPENAI_API_KEY",
            "BASH_ENV",
            "LD_PRELOAD",
        ];
        let result = inherited(vars.map(|key| (key.into(), "synthetic".into())));
        assert_eq!(
            result
                .iter()
                .map(|(key, _)| key.to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["Path", "HOME", "LANG", "TMPDIR"]
        );
    }
}

/// Host transport probes use the same environment as their command launches.
pub fn transport_command(program: impl AsRef<OsStr>, bindings: &[&str]) -> Command {
    let mut command = crate::spawn::command(program);
    transport_environment(&mut command, bindings);
    command
}
