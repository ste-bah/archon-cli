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
    let mut numeric_argument = None;
    let mut path_argument = false;
    let mut eval_argument = false;
    for (index, word) in words.filter(|word| !word.is_empty()).enumerate() {
        if eval_argument {
            eval_argument = false;
            continue;
        }
        // A leading '-' starts another option, leaving the bare -I intact.
        // Otherwise consume one include path; discard -I too if it is unsafe.
        let path = path_argument && !word.starts_with('-');
        path_argument = false;
        if path && !include_path(word) {
            kept.pop();
            continue;
        }
        // Eval executes make code. Consume its separate argument even if that
        // argument looks like an otherwise permitted option.
        let name = word.split('=').next().unwrap_or(word);
        let eval = name.len() >= 4 && "--eval".starts_with(name);
        // Value options consume the rest of a short word, including any E in
        // an include directory. Only the preceding flag letters can mean eval.
        let short = if word.starts_with("--") {
            None
        } else {
            word.strip_prefix('-').or((index == 0).then_some(word))
        };
        let eval_flags = short.map(|flags| {
            let run = flags.split(['j', 'l', 'O', 'I']).next().unwrap_or(flags);
            (
                run.contains('E'),
                run.len() == flags.len() && run.ends_with('E'),
            )
        });
        if eval || eval_flags.is_some_and(|(eval, _)| eval) {
            eval_argument = if eval {
                !word.contains('=')
            } else {
                eval_flags.is_some_and(|(_, separate)| separate)
            };
            numeric_argument = None;
            continue;
        }
        if word == "--" {
            break;
        }
        let numeric = numeric_argument.is_some_and(|load| numeric_value(word, load));
        let cluster = index == 0 && short_flags(word, true).is_some();
        let option = word.starts_with('-') && safe_option(word);
        // Retain the credential-shape check even for validated option values.
        let credential = word.split('=').take(word.matches('=').count()).any(|name| {
            !value_option(name)
                && (credential_name(name) || name.to_ascii_uppercase().ends_with("_AUTH"))
        });
        let keep = (option || cluster || numeric || path) && !word.contains('\\') && !credential;
        numeric_argument = if keep {
            match word {
                "--jobs" => Some(false),
                "--load-average" | "--max-load" => Some(true),
                _ if cluster => short_flags(word, true).flatten(),
                _ => word
                    .strip_prefix('-')
                    .and_then(|s| short_flags(s, false))
                    .flatten(),
            }
        } else {
            None
        };
        if keep {
            path_argument = (cluster || option)
                && !word.contains('=')
                && !word.starts_with("--")
                && word
                    .trim_start_matches('-')
                    .strip_suffix('I')
                    .is_some_and(|flags| flags.chars().all(|ch| "bBdeikLmnpqrRsStvw".contains(ch)));
            kept.push(word);
        }
    }
    (!kept.is_empty()).then(|| kept.join(" ").into())
}

// Only these exact value options may carry '='. Validate their values before
// retaining them; unknown assignment-shaped options can carry credentials/code.
fn value_option(name: &str) -> bool {
    matches!(
        name,
        "--jobserver-auth"
            | "--jobserver-fds"
            | "-j"
            | "--jobs"
            | "-l"
            | "--load-average"
            | "--max-load"
            | "--debug"
            | "--output-sync"
            | "-O"
            | "--shuffle"
            | "--sync-mutex"
    )
}

fn numeric_value(value: &str, load: bool) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|byte| byte.is_ascii_digit());
    if load && let Some((whole, fraction)) = value.split_once('.') {
        digits(whole) && digits(fraction)
    } else {
        digits(value)
    }
}

// Short options consist of flags followed by at most one value option. A bare
// first-word cluster may end in j/l or I to consume a numeric word or path.
fn short_flags(word: &str, bare: bool) -> Option<Option<bool>> {
    let offset = word.find(|ch| !"bBdeikLmnpqrRsStvw".contains(ch));
    let Some(offset) = offset else {
        return (!word.is_empty()).then_some(None);
    };
    if !b"jlOI".contains(&word.as_bytes()[offset]) {
        return None;
    }
    let (name, value) = word[offset..].split_at(1);
    if matches!(name, "j" | "l") && value.is_empty() {
        return Some(Some(name == "l"));
    }
    if name == "I" && value.is_empty() {
        return Some(None);
    }
    if bare {
        return None;
    }
    let valid = match name {
        "j" | "l" => numeric_value(value, name == "l"),
        "O" => value.is_empty() || output_sync(value),
        "I" => include_path(value),
        _ => false,
    };
    valid.then_some(None)
}

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
}

// Drive prefixes belong only to include directories, never FIFO/mutex paths.
fn include_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let path = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        &path[2..]
    } else {
        path
    };
    !path.is_empty()
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-+~@,".contains(&byte))
}

fn output_sync(value: &str) -> bool {
    matches!(value, "none" | "line" | "target" | "recurse")
}

fn safe_option(word: &str) -> bool {
    let Some((name, value)) = word.split_once('=') else {
        return if word.starts_with("--") {
            // Exact no-argument and optional-argument GNU make options only.
            matches!(
                word,
                "--always-make"
                    | "--environment-overrides"
                    | "--ignore-errors"
                    | "--keep-going"
                    | "--no-keep-going"
                    | "--stop"
                    | "--just-print"
                    | "--dry-run"
                    | "--recon"
                    | "--print-data-base"
                    | "--question"
                    | "--no-builtin-rules"
                    | "--no-builtin-variables"
                    | "--silent"
                    | "--quiet"
                    | "--no-silent"
                    | "--touch"
                    | "--trace"
                    | "--print-directory"
                    | "--no-print-directory"
                    | "--warn-undefined-variables"
                    | "--check-symlink-times"
                    | "--debug"
                    | "--jobs"
                    | "--load-average"
                    | "--max-load"
                    | "--output-sync"
                    | "--shuffle"
            )
        } else {
            word.strip_prefix('-')
                .is_some_and(|s| short_flags(s, false).is_some())
        };
    };
    // Only exact allowlisted names can carry values; abbreviations are rejected.
    if !value_option(name) {
        return false;
    }
    match name {
        "-j" | "--jobs" => numeric_value(value, false),
        "-l" | "--load-average" | "--max-load" => numeric_value(value, true),
        "--jobserver-auth" | "--jobserver-fds" => {
            if let Some(path) = value.strip_prefix("fifo:") {
                path.starts_with('/') && safe_path(path)
            } else if let Some(pid) = value.strip_prefix("gmake_semaphore_") {
                numeric_value(pid, false)
            } else {
                value.split_once(',').is_some_and(|(read, write)| {
                    numeric_value(read, false) && numeric_value(write, false)
                })
            }
        }
        "--sync-mutex" => {
            if let Some(path) = value.strip_prefix("fnm:") {
                path.starts_with('/') && safe_path(path)
            } else {
                value.strip_prefix("0x").is_some_and(|hex| {
                    !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            }
        }
        "--debug" => value.split(',').all(|token| {
            matches!(
                token.to_ascii_lowercase().as_str(),
                "all"
                    | "a"
                    | "b"
                    | "v"
                    | "i"
                    | "j"
                    | "m"
                    | "n"
                    | "p"
                    | "w"
                    | "basic"
                    | "verbose"
                    | "implicit"
                    | "jobs"
                    | "makefile"
                    | "none"
                    | "print"
                    | "why"
            )
        }),
        "--output-sync" | "-O" => output_sync(value) || (name == "-O" && value.is_empty()),
        "--shuffle" => {
            matches!(value, "random" | "reverse" | "none" | "identity")
                || numeric_value(value, false)
        }
        _ => false,
    }
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
pub fn command(program: impl AsRef<OsStr>) -> (Command, crate::read_boundary::LaunchReport) {
    command_with_mode(program, Mode::Inherited)
}

pub fn command_with_mode(
    program: impl AsRef<OsStr>,
    mode: Mode<'_>,
) -> (Command, crate::read_boundary::LaunchReport) {
    let (mut command, report) = crate::read_boundary::command(program.as_ref());
    apply_mode(&mut command, mode);
    (command, report)
}

/// Async counterpart, with identical policy.
pub fn tokio_command(
    program: impl AsRef<OsStr>,
) -> (tokio::process::Command, crate::read_boundary::LaunchReport) {
    tokio_command_with_report(program)
}

/// Retain the launch outcome for inspection after spawning.
pub fn command_with_report(
    program: impl AsRef<OsStr>,
) -> (Command, crate::read_boundary::LaunchReport) {
    let (mut command, report) = crate::read_boundary::command_with_report(program.as_ref());
    apply(&mut command);
    (command, report)
}

pub fn tokio_command_with_report(
    program: impl AsRef<OsStr>,
) -> (tokio::process::Command, crate::read_boundary::LaunchReport) {
    let (command, report) = command_with_report(program);
    (command.into(), report)
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
#[path = "execution_environment_tests.rs"]
mod tests;
