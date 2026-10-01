//! Reading what a task body promises for one claim: the lines that name the
//! claimed obligation, and the files, symbols and commands those lines name.
//!
//! # What ties an observable to a claim
//!
//! A line of the body (outside the metadata block) that names the obligation
//! id, directly or inside a written range (`REQ-A-050…053`, `REQ-A-100/101`,
//! `REQ-A-001, 002`). Every backticked span on such a line is classified; a
//! symbol on a line that names exactly one repository path is read as living
//! in that path, and a symbol on a list item that names no path inherits the
//! single path its parent list item names. Nothing else is guessed: a span that
//! is neither a path, an identifier nor a runnable command is prose and is not
//! an observable.

use std::sync::OnceLock;

use regex::Regex;

/// One line of a task body, with the `##` section it sits in.
#[derive(Debug, Clone)]
pub(super) struct BodyLine {
    pub(super) text: String,
    pub(super) section: String,
    /// Inside the leading metadata fence: never an observable line.
    pub(super) metadata: bool,
}

/// A backticked span, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Span {
    /// A path-shaped span, cleaned (`./`, `:line` and a trailing `/` removed).
    Path {
        raw: String,
        directory: bool,
    },
    /// The last segment of an identifier, qualified path or call.
    Symbol(String),
    Command(String),
}

/// Words that, on a line naming a path, say the author knows the path is not
/// there or must not be touched; such a line promises nothing about the path.
const NEGATIONS: &[&str] = &[
    "do not",
    "don't",
    "must not",
    "never",
    "absent",
    "does not exist",
    "no longer",
    "removed",
    "deleted",
];

/// Sections whose paths are other tasks' surfaces, not this claim's.
const FOREIGN_PATH_SECTIONS: &[&str] = &["files forbidden to change"];

pub(super) fn body_lines(raw: &str) -> Vec<BodyLine> {
    let mut lines = Vec::new();
    let mut section = String::new();
    let mut fence: Option<bool> = None;
    let mut seen_metadata = false;
    for text in raw.lines() {
        let trimmed = text.trim_start();
        if trimmed.starts_with("```") {
            match fence {
                Some(metadata) => {
                    lines.push(line(text, &section, metadata));
                    fence = None;
                }
                None => {
                    let metadata =
                        !seen_metadata && trimmed.trim_start_matches('`').starts_with("yaml");
                    seen_metadata |= metadata;
                    fence = Some(metadata);
                    lines.push(line(text, &section, metadata));
                }
            }
            continue;
        }
        if fence.is_none()
            && let Some(heading) = trimmed.strip_prefix("## ")
        {
            section = heading.trim().to_ascii_lowercase();
        }
        lines.push(line(text, &section, fence == Some(true)));
    }
    lines
}

fn line(text: &str, section: &str, metadata: bool) -> BodyLine {
    BodyLine {
        text: text.to_string(),
        section: section.to_string(),
        metadata,
    }
}

fn id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b([A-Z][A-Z0-9]*(?:-[A-Z][A-Z0-9]*)*)-([0-9]+)\b").expect("literal regex")
    })
}

fn continuation_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\s*(…|\.\.\.|\.\.|–|—|/|,|&|and\b|to\b|through\b)\s*(?:([A-Z][A-Z0-9]*(?:-[A-Z][A-Z0-9]*)*)-)?([0-9]+)\b",
        )
        .expect("literal regex")
    })
}

/// Does `line` name obligation `id`, alone or inside a written range or list.
pub(super) fn mentions(line: &str, id: &str) -> bool {
    let Some((prefix, number)) = split_id(id) else {
        return line.contains(id);
    };
    for found in id_re().captures_iter(line) {
        if &found[1] != prefix || found[2].len() != number.len() {
            continue;
        }
        let width = found[2].len();
        let first: u64 = found[2].parse().unwrap_or(u64::MAX);
        let mut ranges = vec![(first, first)];
        let mut rest = &line[found.get(0).map_or(0, |m| m.end())..];
        while let Some(next) = continuation_re().captures(rest) {
            if next.get(2).is_some_and(|named| named.as_str() != prefix) || next[3].len() != width {
                break;
            }
            let value: u64 = next[3].parse().unwrap_or(u64::MAX);
            match &next[1] {
                "/" | "," | "&" | "and" => ranges.push((value, value)),
                _ => {
                    if let Some(last) = ranges.last_mut() {
                        last.1 = value.max(last.0);
                    }
                }
            }
            rest = &rest[next.get(0).map_or(rest.len(), |m| m.end())..];
        }
        let wanted: u64 = number.parse().unwrap_or(u64::MAX);
        if ranges
            .iter()
            .any(|(low, high)| (*low..=*high).contains(&wanted))
        {
            return true;
        }
    }
    false
}

fn split_id(id: &str) -> Option<(&str, &str)> {
    let (prefix, number) = id.rsplit_once('-')?;
    (!prefix.is_empty() && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
        .then_some((prefix, number))
}

fn backtick_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"`([^`]+)`").expect("literal regex"))
}

fn ident_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)(\(.*\))?$")
            .expect("literal regex")
    })
}

/// Every backticked span on `line`, classified; prose spans are dropped.
pub(super) fn spans(line: &str, runners: &dyn Fn(&str) -> bool) -> Vec<Span> {
    backtick_re()
        .captures_iter(line)
        .filter_map(|caps| classify(caps[1].trim(), runners))
        .collect()
}

pub(super) fn classify(span: &str, runners: &dyn Fn(&str) -> bool) -> Option<Span> {
    let first = span.split_whitespace().next().unwrap_or_default();
    if span.contains(char::is_whitespace) {
        return runners(first).then(|| Span::Command(span.to_string()));
    }
    if span.contains(['<', '>', '{', '}', '*', '$', '=', '|', '?', '"', '\'']) {
        return None;
    }
    if span.contains('/') && !span.contains("://") {
        let directory = span.ends_with('/');
        let mut raw = span.trim_end_matches('/').to_string();
        if let Some((path, tail)) = raw.rsplit_once(':')
            && !tail.is_empty()
            && tail.chars().all(|c| c.is_ascii_digit() || c == '-')
        {
            raw = path.to_string();
        }
        let raw = raw.strip_prefix("./").unwrap_or(&raw).to_string();
        return (!raw.is_empty()).then_some(Span::Path { raw, directory });
    }
    let caps = ident_re().captures(span)?;
    let name = caps[1].to_string();
    let qualified = span.contains("::") || caps.get(2).is_some();
    let inner = name.trim_matches('_');
    let snake = inner.contains('_') && !name.ends_with('_');
    let camel = name
        .char_indices()
        .skip(1)
        .any(|(i, c)| c.is_ascii_uppercase() && name[..i].chars().any(|p| p.is_ascii_lowercase()));
    (qualified || snake || camel).then_some(Span::Symbol(name))
}

/// Whether a line's paths and symbols are this claim's promise at all.
pub(super) fn line_promises(line: &BodyLine) -> bool {
    // Whole words only: `never_written.rs` is a name, not a "never".
    let words: String = line
        .text
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '\'' | '_' | '-' | '/') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let words = format!(
        " {} ",
        words.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    !FOREIGN_PATH_SECTIONS.contains(&line.section.as_str())
        && !NEGATIONS
            .iter()
            .any(|phrase| words.contains(&format!(" {phrase} ")))
}

/// The single path the nearest enclosing list item names, for a list item
/// that names none itself.
pub(super) fn parent_path(
    lines: &[BodyLine],
    index: usize,
    runners: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let indent = |text: &str| text.len() - text.trim_start().len();
    let own = indent(&lines[index].text);
    for candidate in lines[..index].iter().rev() {
        let text = candidate.text.as_str();
        if text.trim().is_empty() {
            continue;
        }
        if text.trim_start().starts_with('#') || candidate.metadata {
            return None;
        }
        if indent(text) < own && is_list_item(text) {
            let paths: Vec<String> = spans(text, runners)
                .into_iter()
                .filter_map(|span| match span {
                    Span::Path {
                        raw,
                        directory: false,
                    } => Some(raw),
                    _ => None,
                })
                .collect();
            return (paths.len() == 1).then(|| paths[0].clone());
        }
    }
    None
}

fn is_list_item(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed
            .split_once(". ")
            .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// What a declared verifier command asks the repository to have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CommandSubject {
    Package(String),
    /// `(package, kind, name)`: a `--test`, `--bin`, `--example` or `--bench`.
    Target(Option<String>, &'static str, String),
    Path(String),
}

pub(super) fn command_subjects(
    command: &str,
    runners: &dyn Fn(&str) -> bool,
) -> Vec<CommandSubject> {
    let argv: Vec<String> = command
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|c| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | ';' | ','))
                .to_string()
        })
        .collect();
    let mut subjects = Vec::new();
    if argv.first().map(String::as_str) == Some("cargo") {
        let package = flag_values(&argv, &["-p", "--package"]).into_iter().next();
        if let Some(name) = &package {
            subjects.push(CommandSubject::Package(name.clone()));
        }
        for (flag, kind) in [
            ("--test", "test"),
            ("--bin", "bin"),
            ("--example", "example"),
            ("--bench", "bench"),
        ] {
            for name in flag_values(&argv, &[flag]) {
                subjects.push(CommandSubject::Target(package.clone(), kind, name));
            }
        }
        for path in flag_values(&argv, &["--manifest-path"]) {
            subjects.push(CommandSubject::Path(path));
        }
        return subjects;
    }
    for token in argv.iter().skip(1) {
        if let Some(Span::Path { raw, .. }) = classify(token, runners) {
            subjects.push(CommandSubject::Path(raw));
        }
    }
    subjects
}

fn flag_values(argv: &[String], flags: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    for (index, token) in argv.iter().enumerate() {
        if token == "--" {
            break;
        }
        for flag in flags {
            if token == flag {
                if let Some(value) = argv.get(index + 1).filter(|v| !v.starts_with('-')) {
                    values.push(value.clone());
                }
            } else if let Some(value) = token.strip_prefix(&format!("{flag}=")) {
                values.push(value.to_string());
            }
        }
    }
    values
}
