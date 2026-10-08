//! A child that closes inherited descriptors cannot use a pipe jobserver.
use std::ffi::{OsStr, OsString};
use std::process::Command;

const VARIABLES: &[&str] = &["MAKEFLAGS", "MFLAGS", "GNUMAKEFLAGS", "CARGO_MAKEFLAGS"];

pub(crate) fn inherit(command: &mut Command) {
    for name in VARIABLES {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, strip_fd_jobserver(&value));
        }
    }
}

/// For a library command that inherits this process's environment: give each
/// jobserver variable that it does not set or remove explicitly the policy
/// value of this process's own.
pub(crate) fn inherit_unset(command: &mut Command) {
    let explicit: Vec<_> = command
        .get_envs()
        .map(|(name, _)| name.to_owned())
        .collect();
    for name in VARIABLES {
        if explicit.iter().any(|set| names(set, name)) {
            continue;
        }
        if let Some(value) = std::env::var_os(name) {
            command.env(name, strip_fd_jobserver(&value));
        }
    }
}

/// Sanitize explicitly configured flags, including a library's copied ambient
/// environment. Never reintroduce variables removed by `env_clear`/`env_remove`.
pub fn sanitize_environment(command: &mut Command) {
    let values: Vec<_> = command
        .get_envs()
        .filter_map(|(name, value)| {
            is_variable(name)
                .then(|| value.map(|value| (name.to_owned(), strip_fd_jobserver(value))))
                .flatten()
        })
        .collect();
    for (name, value) in values {
        command.env(name, value);
    }
}

/// Apply the descriptor policy while a library copies or overrides an env value.
pub fn sanitize_variable(name: &OsStr, value: OsString) -> OsString {
    if is_variable(name) {
        strip_fd_jobserver(&value)
    } else {
        value
    }
}

fn is_variable(name: &OsStr) -> bool {
    VARIABLES.iter().any(|variable| names(name, variable))
}

/// Environment names compare case-insensitively on Windows only.
fn names(name: &OsStr, variable: &str) -> bool {
    name.to_str().is_some_and(|name| {
        if cfg!(windows) {
            name.eq_ignore_ascii_case(variable)
        } else {
            name == variable
        }
    })
}

/// Preserve other flags, escaped assignment values, non-Unicode bytes, and
/// FIFO/named jobservers: only a pair of inherited numeric descriptors is stale.
pub fn strip_fd_jobserver(value: &OsStr) -> OsString {
    let bytes = value.as_encoded_bytes();
    let mut words = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        if bytes[start].is_ascii_whitespace() {
            start += 1;
            continue;
        }
        let mut end = start;
        let mut quote = None;
        while end < bytes.len() {
            let byte = bytes[end];
            if quote.is_none() && byte.is_ascii_whitespace() {
                break;
            }
            if byte == b'\\' && end + 1 < bytes.len() {
                end += 1;
            } else if quote == Some(byte) {
                quote = None;
            } else if quote.is_none() && matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            }
            end += 1;
        }
        words.push(&bytes[start..end]);
        start = end;
    }
    let mut kept = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        // GNU make appends variable definitions after its option terminator.
        if word == b"--" {
            kept.extend_from_slice(&words[index..]);
            break;
        }
        let option = [
            b"--jobserver-auth".as_slice(),
            b"--jobserver-fds".as_slice(),
        ];
        if option.iter().any(|option| {
            word.strip_prefix(*option)
                .and_then(|tail| tail.strip_prefix(b"="))
                .is_some_and(fd_pair)
        }) {
            index += 1;
        } else if option.contains(&word) && words.get(index + 1).is_some_and(|word| fd_pair(word)) {
            index += 2;
        } else {
            kept.push(word);
            index += 1;
        }
    }
    if kept.len() == words.len() {
        return value.to_owned();
    }
    let bytes = kept.join(&b' ');
    // SAFETY: splitting at ASCII whitespace/option boundaries and joining with
    // ASCII preserves the self-synchronizing OsStr encoding, even for non-UTF8.
    unsafe { OsString::from_encoded_bytes_unchecked(bytes) }
}

fn fd_pair(value: &[u8]) -> bool {
    let mut numbers = value.split(|byte| *byte == b',');
    let number = |value: &[u8]| {
        let digits = value.strip_prefix(b"-").unwrap_or(value);
        !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
    };
    matches!((numbers.next(), numbers.next(), numbers.next()),
        (Some(a), Some(b), None) if number(a) && number(b))
}
