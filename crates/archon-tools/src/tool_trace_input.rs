//! What of a tool call's input a persisted session trace may keep
//! (Issue 276).
//!
//! A tool input is agent-written and can carry anything: a Write's whole
//! file, an Edit's old and new text, a URL with a key in its query. Word
//! redaction does not help there (`PASSWORD=hunter2` keeps `hunter2`), and a
//! value cut for size can lose the end marker a shape rule needs (a PEM
//! block without its END line). So a trace keeps an allow-list of keys
//! that name WHAT was touched, never the content, and redacts each kept
//! value BEFORE it is cut:
//!
//! - paths (`file_path`, `notebook_path`, `path`): ONLY the credential-shape
//!   check (`redact_secret_values`), no word or assignment redaction, so a
//!   path stays the path that was read; a plain-word secret in a path is
//!   kept;
//! - search text (`pattern`, `glob`): URLs cut as below, credential shapes
//!   replaced, then the value of every `KEY=VALUE` (or `KEY: VALUE`) whose
//!   key names a credential replaced;
//! - `url`: scheme, host and path only, never user info, query or fragment;
//! - `offset` / `limit`: numbers only;
//! - `command`, for `Bash` only: never its text. A shell command can carry
//!   a credential in more forms than any rule can know (`mysql -phunter2`,
//!   `curl -u a:b`, an escaped JSON body, `redis-cli AUTH x`), so the trace
//!   keeps only the program's name and one allow-listed sub-command
//!   (`program`, see [`command_program`]) and the number of words after the
//!   first (`arg_count`). No digest of the command is kept: an unkeyed hash
//!   of a short command can be guessed offline.
//!
//! Every other key is dropped and counted.
use archon_observability::redaction::redact_secret_values;
use archon_observability::secret_values::{REDACTED_VALUE, is_credential_name};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

/// The only second words a command's program may keep: common sub-commands
/// that name an action, never a value.
const SUB_COMMANDS: &[&str] = &[
    "test", "build", "check", "run", "status", "diff", "log", "fmt", "clippy", "install", "add",
    "commit", "push", "pull", "fetch", "show", "list", "ls",
];

/// Keys whose values name what a call touched.
const PATH_KEYS: &[&str] = &["file_path", "notebook_path", "path"];
/// Keys whose values are search text.
const SEARCH_KEYS: &[&str] = &["pattern", "glob"];
/// Keys whose numeric values bound a read.
const NUMBER_KEYS: &[&str] = &["offset", "limit"];

/// A tool input reduced to what a trace may keep.
#[derive(Debug, Clone, PartialEq)]
pub struct SafeInput {
    /// The kept keys, redacted and cut; an object, or null for a non-object
    /// input.
    pub input: Value,
    /// Keys left out because they are not on the allow-list.
    pub dropped_keys: usize,
    /// Whether a kept value was cut to `max_value_bytes`.
    pub cut: bool,
}

/// Reduce `input` of a call to `tool_name` as the module says, each kept
/// string at most `max_value_bytes` long (cut on a character boundary and
/// ended with an ellipsis, after redaction).
pub fn safe_input(tool_name: &str, input: &Value, max_value_bytes: usize) -> SafeInput {
    let Some(object) = input.as_object() else {
        let dropped_keys = usize::from(!input.is_null());
        return SafeInput {
            input: Value::Null,
            dropped_keys,
            cut: false,
        };
    };
    let mut kept = Map::new();
    let mut dropped_keys = 0;
    let mut cut = false;
    for (key, value) in object {
        let redacted = match (key.as_str(), value) {
            (key, Value::String(text)) if PATH_KEYS.contains(&key) => redact_secret_values(text),
            (key, Value::String(text)) if SEARCH_KEYS.contains(&key) => redact_tool_text(text),
            ("url", Value::String(url)) => redact_secret_values(&strip_url(url)),
            ("command", Value::String(command)) if tool_name == "Bash" => {
                let program = command_program(command);
                let kept_program = clip(&program, max_value_bytes);
                cut |= kept_program.len() != program.len();
                kept.insert("program".into(), json!(kept_program));
                let words = command.split_whitespace().count();
                kept.insert("arg_count".into(), json!(words.saturating_sub(1)));
                continue;
            }
            (key, Value::Number(_)) if NUMBER_KEYS.contains(&key) => {
                kept.insert(key.to_string(), value.clone());
                continue;
            }
            _ => {
                dropped_keys += 1;
                continue;
            }
        };
        let clipped = clip(&redacted, max_value_bytes);
        cut |= clipped.len() != redacted.len();
        kept.insert(key.clone(), Value::String(clipped));
    }
    SafeInput {
        input: Value::Object(kept),
        dropped_keys,
        cut,
    }
}

/// Free text a trace keeps (a search pattern): URLs cut to
/// scheme, host and path, credential shapes replaced (first, so a `Bearer`
/// credential is matched whole), then credential assignments' values.
pub fn redact_tool_text(text: &str) -> String {
    let text = URL.replace_all(text, |caps: &regex::Captures<'_>| strip_url(&caps[0]));
    let text = redact_secret_values(&text);
    ASSIGNMENT
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            if is_credential_name(&caps["key"]) {
                format!("{}{}{}", &caps["key"], &caps["sep"], REDACTED_VALUE)
            } else {
                caps[0].to_string()
            }
        })
        .into_owned()
}

/// The program of a shell command: the base name of its first word, and a
/// second word only when it is one of [`SUB_COMMANDS`]. A first word that
/// is an option, an assignment or anything but `[A-Za-z0-9_./-]` keeps
/// nothing. `cargo test -p x` is `cargo test`; `/usr/bin/redis-cli AUTH x`
/// is `redis-cli`; `DB_PASSWORD=x ./run` is empty.
pub fn command_program(command: &str) -> String {
    let mut words = command.split_whitespace();
    let Some(first) = words.next().filter(|word| {
        !word.starts_with('-')
            && word
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '/' | '-'))
    }) else {
        return String::new();
    };
    let program = first.rsplit('/').next().unwrap_or_default();
    match words.next().filter(|word| SUB_COMMANDS.contains(word)) {
        Some(sub) => format!("{program} {sub}"),
        None => program.to_string(),
    }
}

/// `scheme://host/path` of `url`: no user info, query or fragment.
///
/// The authority ends at the first `/` after `scheme://`, and only an `@`
/// inside it ends user info (at its last `@`): an `@` in the path or query
/// is never user info (`https://h/x?u=a@b/c` is `https://h/x`). User info
/// is removed BEFORE the cut at `?` or `#`, so a `?` or `#` inside a
/// password cannot keep part of it (`postgres://app:Pa#ss@db/main` is
/// `postgres://db/main`); an address whose `?` or `#` comes before an `@`
/// in that first segment is read the same way, losing its host rather
/// than risk keeping a password. Text without a scheme is cut at its first
/// `?` or `#`.
pub fn strip_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.split(['?', '#']).next().unwrap_or_default().to_string();
    };
    let authority = &rest[..rest.find('/').unwrap_or(rest.len())];
    let rest = match authority.rfind('@') {
        Some(at) => &rest[at + 1..],
        None => rest,
    };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    format!("{scheme}://{rest}")
}

static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"[A-Za-z][A-Za-z0-9+.\-]*://[^\s'"<>`]+"#).expect("constant URL regex")
});

/// `KEY=VALUE`, `KEY: VALUE`, `"KEY": "VALUE"` and `--key=value`; the value
/// is a quoted string or one shell word.
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?P<key>[A-Za-z_][A-Za-z0-9_.\-]*)(?P<sep>["']?\s*[=:]\s*)(?P<value>"[^"]*"|'[^']*'|[^\s;&|"'`]+)"#,
    )
    .expect("constant assignment regex")
});

/// At most `max` bytes of `text`, cut on a character boundary and ended
/// with an ellipsis when cut.
pub fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max.saturating_sub('\u{2026}'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\u{2026}", &text[..end])
}

#[cfg(test)]
#[path = "tool_trace_input_tests.rs"]
mod tests;
