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
    "SHELL",
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
    "PSMODULEPATH",
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
            let value = if build_flags {
                option_flags(&value)?
            } else {
                value
            };
            let value = crate::jobserver::sanitize_variable(&key, value);
            (!build_flags || !value.is_empty()).then_some((key, value))
        })
        .collect()
}

/// MAKEFLAGS starts with bare flag letters, then options, then `--` and
/// variable definitions. No inherited variable definition belongs in a payload.
fn option_flags(value: &OsStr) -> Option<OsString> {
    let value = value.to_str()?;
    // Make escapes whitespace with backslashes in variable values. Keep each
    // such value together so its fragments cannot become apparent options.
    let mut escaped = false;
    let words = value.split(|ch: char| {
        if escaped {
            escaped = false;
            false
        } else if ch == '\\' {
            escaped = true;
            false
        } else {
            ch.is_whitespace()
        }
    });
    let mut kept = Vec::new();
    let mut numeric_argument = false;
    let mut eval_argument = false;
    for (index, word) in words.filter(|word| !word.is_empty()).enumerate() {
        if eval_argument {
            eval_argument = false;
            continue;
        }
        // Eval executes make code. Consume its separate argument even if that
        // argument looks like an otherwise permitted option.
        if word == "--eval"
            || (word.starts_with('-') && !word.starts_with("--") && word.contains('E'))
        {
            eval_argument = word == "--eval" || word.ends_with('E');
            numeric_argument = false;
            continue;
        }
        if word.starts_with("--eval=") {
            numeric_argument = false;
            continue;
        }
        if word == "--" {
            break;
        }
        let numeric = numeric_argument
            && !word.is_empty()
            && word
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
            && word.bytes().any(|byte| byte.is_ascii_digit());
        let cluster = index == 0
            && word
                .bytes()
                .all(|byte| b"bBdeikLmnpqrRsStvw".contains(&byte));
        // Only these value options may carry '=': jobserver endpoints, job
        // counts, load limits, debug selectors and output synchronization.
        // Other assignment-shaped options may smuggle credentials or make code.
        let option = word.starts_with('-')
            && word.split_once('=').is_none_or(|(name, _)| {
                matches!(
                    name,
                    "--jobserver-auth"
                        | "--jobserver-fds"
                        | "-j"
                        | "--jobs"
                        | "-l"
                        | "--load-average"
                        | "--debug"
                        | "--output-sync"
                        | "-O"
                )
            });
        if option || cluster || numeric {
            kept.push(word);
        }
        numeric_argument = matches!(
            word,
            "-j" | "--jobs" | "-l" | "--load-average" | "--max-load"
        );
    }
    (!kept.is_empty()).then(|| kept.join(" ").into())
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

/// Host transport probes use the same environment as their command launches.
pub fn transport_command(program: impl AsRef<OsStr>, bindings: &[&str]) -> Command {
    let mut command = crate::spawn::command(program);
    transport_environment(&mut command, bindings);
    command
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
            && value == "--jobserver-auth=fifo:/tmp/archon-jobserver"));
    }

    #[test]
    fn build_flags_keep_options_and_drop_all_assignments() {
        for (input, expected) in [
            (
                "-j4 --jobserver-auth=3,4 -- FOO=bar SECRET_X=y",
                Some("-j4 --jobserver-auth=3,4"),
            ),
            ("ks -- FOO=bar", Some("ks")),
            ("ks -- ", Some("ks")),
            ("-j 4 -k -s --silent -w", Some("-j 4 -k -s --silent -w")),
            (
                "-j8 FOO=bar SECRET_X=y SERVICE_AUTH=z --jobserver-fds=3,4",
                Some("-j8 --jobserver-fds=3,4"),
            ),
            ("--eval=TOKEN=abc -k", Some("-k")),
            ("--eval TOKEN=abc -s", Some("-s")),
            ("-E TOKEN=abc -s", Some("-s")),
            ("-E -k -s", Some("-s")),
            ("-ETOKEN=abc -kE TOKEN=abc -s", Some("-s")),
            ("--unknown=TOKEN=abc -x=abc --silent", Some("--silent")),
            ("--eval=TOKEN=abc -E TOKEN=abc", None),
            (
                "--jobserver-auth=3,4 --jobserver-fds=5,6 -j4 -j 8 --jobs=2 -l2.5 -l 3 --load-average=4 --debug=a --output-sync=target -Oline -O=target",
                Some(
                    "--jobserver-auth=3,4 --jobserver-fds=5,6 -j4 -j 8 --jobs=2 -l2.5 -l 3 --load-average=4 --debug=a --output-sync=target -Oline -O=target",
                ),
            ),
            ("FOO=bar SECRET_X=y SERVICE_AUTH=z", None),
            ("-- FOO=bar SECRET_X=y", None),
            ("FOO=bar\\ -secret -k", Some("-k")),
            ("unexpected words -s", Some("-s")),
            ("", None),
        ] {
            assert_eq!(
                option_flags(OsStr::new(input)),
                expected.map(OsString::from),
                "{input}"
            );
        }
        for key in ["MAKEFLAGS", "MFLAGS", "GNUMAKEFLAGS", "CARGO_MAKEFLAGS"] {
            assert_eq!(
                inherited([(key.into(), "-k FOO=bar SECRET_X=y SERVICE_AUTH=z".into())]),
                vec![(OsString::from(key), OsString::from("-k"))],
            );
            assert!(inherited([(key.into(), "--jobserver-auth=3,4 -- FOO=bar".into())]).is_empty());
        }
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
