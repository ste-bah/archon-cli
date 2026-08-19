//! Slash command parser.
//!
//! Pure: takes raw user input (e.g. `"/effort high"`) and emits a
//! [`ParsedCommand`] describing the command name, its positional arguments
//! and its flags. No I/O, no async, no app state. Dispatch and registry live
//! in separate modules (TASK-AGS-622 / TASK-AGS-623).
//!
//! [`CommandParser::parse`] is the entry point, and the only one:
//! `dispatcher.rs` calls it at both its call sites and maps each
//! [`ParseError`] variant to a distinct TUI error.
//!
//! # What used to be here
//!
//! TASK-AGS-801 reconciled this module against a spec, and did it by adding
//! the spec's shape alongside the shipped shape rather than replacing it: an
//! `Arg` newtype, three `ParsedCommand` accessors, and a free
//! `parse() -> Option<ParsedCommand>` kept "for back-compat with the
//! dispatcher's `parser::parse` call sites". The dispatcher had already moved
//! to `CommandParser::parse` by then, so nothing called any of it, and the
//! module carried a file-level `#![allow(dead_code)]` to keep the compiler
//! quiet about a surface that existed only to match a document.
//!
//! It is gone. The behaviour it described was real and is still tested —
//! those tests now run against `CommandParser::parse`, which is what
//! production uses. Three of them asserted the free function's *stricter*
//! contract (no leading `/` rejected, empty rejected, bare `/` rejected);
//! the first is deliberately not how `CommandParser` behaves and the other
//! two were already covered, so they went with it.
//!
//! Without the `allow`, an unused item in here is now a build failure rather
//! than a comment explaining itself.

use std::collections::HashMap;

use thiserror::Error;

/// A parsed slash command: the command name, its positional arguments
/// and any `--key[=value]` flags, in declaration order.
///
/// The `name` is the token immediately following the leading `/`, with
/// no case normalization applied — the dispatcher is responsible for
/// case-folding when looking the command up in the registry.
///
/// `args` is `Vec<String>` because `dispatcher.rs` treats `&parsed.args` as
/// `&[String]`. Every field is public and read directly; there are no
/// accessor methods, which is why the ones that existed and were never
/// called could be deleted without a caller noticing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ParsedCommand {
    pub name: String,
    /// Original tokens after the slash command name, preserving flag
    /// tokens and ordering. Dispatch uses this so command handlers that
    /// mirror CLI syntax receive the same argv shape the user typed.
    pub raw_args: Vec<String>,
    pub args: Vec<String>,
    pub flags: HashMap<String, String>,
}

/// Errors returned by [`CommandParser::parse`].
///
/// Each failure mode is a distinct variant so the TUI error layer
/// (TASK-AGS-804) can tell them apart — `dispatcher.rs` matches on all four
/// and emits a different message for each.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum ParseError {
    /// Input was empty or whitespace-only.
    #[error("input is empty")]
    Empty,
    /// Input was just `/` with no command name.
    #[error("missing command name after `/`")]
    MissingName,
    /// A `--` token was structurally malformed (e.g. `--=value`).
    #[error("malformed flag: {0}")]
    MalformedFlag(String),
    /// Tokenizer reached end-of-input while still inside a `"..."` pair.
    #[error("unclosed quoted string")]
    UnclosedQuote,
}

/// Return up to `limit` suggestions from `known` whose Levenshtein
/// distance to `unknown` is `< 3` (i.e. at most 2 edits), sorted
/// ascending by distance.
///
/// Ties are broken by input order (stable sort on the `(distance, item)`
/// pair). Used by TASK-AGS-804's ERR-SLASH-01 unknown-command error
/// formatter to suggest "did you mean /model?" style hints.
///
/// ## R-item: threshold reconciliation (R6)
///
/// TASK-AGS-801 spec text reads "Levenshtein distance ≤ 3" but its
/// validation criterion 6 (`suggest("modl", ["model", "cost", "memory"],
/// 3)` -> `["model"]`) requires `cost` (distance 3 from `modl`) to be
/// excluded. Honoring the criterion means the effective threshold is
/// `< 3` (i.e. `≤ 2`). Criterion 7 (`suggest("xyz", ["model", "cost"],
/// 3)` -> `[]`, distances 4 and 4) passes under either threshold, so
/// the strict boundary is set by criterion 6. This is documented as
/// R-item R6 in the commit body.
pub(crate) fn suggest<'a>(
    unknown: &str,
    known: impl IntoIterator<Item = &'a str>,
    limit: usize,
) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let mut scored: Vec<(usize, &str)> = known
        .into_iter()
        .map(|candidate| (strsim::levenshtein(unknown, candidate), candidate))
        .filter(|(dist, _)| *dist < 3)
        .collect();
    scored.sort_by_key(|(dist, _)| *dist);
    scored
        .into_iter()
        .take(limit)
        .map(|(_, s)| s.to_string())
        .collect()
}

/// Split `body` into tokens using a quote-aware scanner.
///
/// - `"` toggles an in-quotes state and is itself discarded.
/// - Whitespace outside quotes delimits tokens; runs of whitespace
///   collapse.
/// - Whitespace inside quotes is preserved verbatim.
/// - A trailing non-empty buffer at EOF is pushed as a final token.
/// - EOF while still inside quotes returns [`ParseError::UnclosedQuote`].
fn tokenize(body: &str) -> Result<Vec<String>, ParseError> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut had_quoted_content = false;

    for ch in body.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            had_quoted_content = true;
            continue;
        }
        if ch.is_whitespace() && !in_quotes {
            if !current.is_empty() || had_quoted_content {
                tokens.push(std::mem::take(&mut current));
                had_quoted_content = false;
            }
            continue;
        }
        current.push(ch);
    }

    if in_quotes {
        return Err(ParseError::UnclosedQuote);
    }

    if !current.is_empty() || had_quoted_content {
        tokens.push(current);
    }

    Ok(tokens)
}

/// The parser.
///
/// - A leading `/` is optional: `"model"` parses the same as `"/model"`.
/// - Tokenization is quote-aware: pairs of `"` delimit a single token that
///   may contain whitespace, and the quotes are stripped from the token.
///   Runs of whitespace between tokens collapse.
/// - The first token becomes [`ParsedCommand::name`], with no case
///   normalization — the dispatcher case-folds when looking up the registry.
/// - Tokens beginning with `--` go into [`ParsedCommand::flags`]:
///   `--key=value` sets `flags["key"] = "value"`, and a bare `--key` sets
///   `flags["key"] = "true"`. A single leading `-` (e.g. `-v`) is a
///   positional arg, not a flag.
/// - Every token after the name, flags included, is preserved in order in
///   [`ParsedCommand::raw_args`], so handlers that mirror CLI syntax receive
///   the argv the user actually typed.
pub(crate) struct CommandParser;

impl CommandParser {
    pub(crate) fn parse(input: &str) -> Result<ParsedCommand, ParseError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(ParseError::Empty);
        }
        let owned;
        let normalized: &str = if trimmed.starts_with('/') {
            trimmed
        } else {
            owned = format!("/{trimmed}");
            owned.as_str()
        };

        // Bare `/` after normalization: name is missing.
        if normalized == "/" {
            return Err(ParseError::MissingName);
        }

        let body = &normalized[1..];
        let tokens = tokenize(body)?;
        let mut iter = tokens.into_iter();
        let name = iter.next().ok_or(ParseError::MissingName)?;
        if name.is_empty() {
            return Err(ParseError::MissingName);
        }

        let raw_args: Vec<String> = iter.collect();
        let mut args: Vec<String> = Vec::new();
        let mut flags: HashMap<String, String> = HashMap::new();
        for tok in &raw_args {
            if let Some(rest) = tok.strip_prefix("--") {
                if rest.is_empty() {
                    return Err(ParseError::MalformedFlag(tok.clone()));
                }
                if let Some(eq_idx) = rest.find('=') {
                    let (k, v) = rest.split_at(eq_idx);
                    if k.is_empty() {
                        return Err(ParseError::MalformedFlag(tok.clone()));
                    }
                    flags.insert(k.to_string(), v[1..].to_string());
                } else {
                    flags.insert(rest.to_string(), "true".to_string());
                }
            } else {
                args.push(tok.clone());
            }
        }

        Ok(ParsedCommand {
            name,
            raw_args,
            args,
            flags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Tokenizer behaviour. These were written against the free
    // `parse() -> Option<ParsedCommand>`, which had no callers and has been
    // deleted; the behaviour they describe is real and is what production
    // runs, so they now exercise `CommandParser::parse` directly.
    //
    // Three of the originals asserted the free function's *stricter*
    // contract and did not survive the move: `rejects_non_slash_input`
    // (CommandParser deliberately accepts it — see
    // `commandparser_accepts_no_leading_slash`), `rejects_empty_input` and
    // `rejects_bare_slash` (both already covered below, with the specific
    // `ParseError` instead of a bare `None`).
    // ---------------------------------------------------------------

    #[test]
    fn parses_bare_slash_command() {
        assert_eq!(
            CommandParser::parse("/fast"),
            Ok(ParsedCommand {
                name: "fast".to_string(),
                raw_args: vec![],
                args: vec![],
                flags: HashMap::new(),
            }),
        );
    }

    #[test]
    fn parses_command_with_single_arg() {
        assert_eq!(
            CommandParser::parse("/effort high"),
            Ok(ParsedCommand {
                name: "effort".to_string(),
                raw_args: vec!["high".to_string()],
                args: vec!["high".to_string()],
                flags: HashMap::new(),
            }),
        );
    }

    #[test]
    fn parses_config_subcommand() {
        assert_eq!(
            CommandParser::parse("/config sources"),
            Ok(ParsedCommand {
                name: "config".to_string(),
                raw_args: vec!["sources".to_string()],
                args: vec!["sources".to_string()],
                flags: HashMap::new(),
            }),
        );
    }

    #[test]
    fn parses_quoted_argument_with_spaces() {
        assert_eq!(
            CommandParser::parse("/rules edit r1 \"some text with spaces\""),
            Ok(ParsedCommand {
                name: "rules".to_string(),
                raw_args: vec![
                    "edit".to_string(),
                    "r1".to_string(),
                    "some text with spaces".to_string(),
                ],
                args: vec![
                    "edit".to_string(),
                    "r1".to_string(),
                    "some text with spaces".to_string(),
                ],
                flags: HashMap::new(),
            }),
        );
    }

    #[test]
    fn tolerates_extra_whitespace() {
        assert_eq!(
            CommandParser::parse("/effort   high  "),
            Ok(ParsedCommand {
                name: "effort".to_string(),
                raw_args: vec!["high".to_string()],
                args: vec!["high".to_string()],
                flags: HashMap::new(),
            }),
        );
    }

    // ---------------------------------------------------------------
    // Flags.
    // ---------------------------------------------------------------

    #[test]
    fn parses_flag_with_value() {
        let parsed =
            CommandParser::parse("/model claude-4.5 --temperature=0.2").expect("should parse");
        assert_eq!(parsed.name, "model");
        assert_eq!(parsed.args, vec!["claude-4.5".to_string()]);
        assert_eq!(
            parsed.flags.get("temperature").map(String::as_str),
            Some("0.2")
        );
    }

    #[test]
    fn commandparser_accepts_no_leading_slash() {
        let parsed = CommandParser::parse("model").expect("no-slash should succeed");
        assert_eq!(parsed.name, "model");
        assert!(parsed.args.is_empty());
        assert!(parsed.flags.is_empty());
    }

    #[test]
    fn commandparser_empty_returns_error() {
        assert_eq!(CommandParser::parse(""), Err(ParseError::Empty));
        assert_eq!(CommandParser::parse("   "), Err(ParseError::Empty));
    }

    #[test]
    fn parses_quoted_arg_with_flag() {
        let parsed =
            CommandParser::parse("/export --format=json \"my session\"").expect("should parse");
        assert_eq!(parsed.name, "export");
        assert_eq!(parsed.args, vec!["my session".to_string()]);
        assert_eq!(parsed.flags.get("format").map(String::as_str), Some("json"));
    }

    #[test]
    fn parses_bare_flag_as_true() {
        let parsed = CommandParser::parse("/fork --detach").expect("should parse");
        assert_eq!(parsed.name, "fork");
        assert!(parsed.args.is_empty());
        assert_eq!(parsed.flags.get("detach").map(String::as_str), Some("true"));
    }

    #[test]
    fn suggest_returns_close_match() {
        let out = suggest("modl", ["model", "cost", "memory"], 3);
        assert_eq!(out, vec!["model".to_string()]);
    }

    #[test]
    fn suggest_returns_empty_when_too_far() {
        let out = suggest("xyz", ["model", "cost"], 3);
        assert!(out.is_empty(), "expected empty, got {out:?}");
    }

    // ---------------------------------------------------------------
    // Bonus tests covering edge cases called out in the spec.
    // ---------------------------------------------------------------

    #[test]
    fn commandparser_bare_slash_returns_missing_name() {
        assert_eq!(CommandParser::parse("/"), Err(ParseError::MissingName));
    }

    #[test]
    fn commandparser_unclosed_quote_returns_error() {
        assert_eq!(
            CommandParser::parse("/export \"unterminated"),
            Err(ParseError::UnclosedQuote),
        );
    }

    #[test]
    fn commandparser_malformed_flag_returns_error() {
        assert_eq!(
            CommandParser::parse("/fork --"),
            Err(ParseError::MalformedFlag("--".to_string())),
        );
        assert_eq!(
            CommandParser::parse("/fork --=value"),
            Err(ParseError::MalformedFlag("--=value".to_string())),
        );
    }

    #[test]
    fn a_flag_does_not_become_a_positional() {
        let parsed = CommandParser::parse("/effort high --quiet").expect("should parse");
        assert_eq!(parsed.args, vec!["high".to_string()]);
        assert!(parsed.flags.contains_key("quiet"));
    }

    #[test]
    fn suggest_respects_limit_and_order() {
        // distances: model=1, modal=1 (close), midel=2, xyz=3 (out)
        let out = suggest("modl", ["model", "modal", "midel"], 2);
        assert_eq!(out.len(), 2);
        // "model" and "modal" both have distance 1; stable sort keeps
        // input order, so "model" comes first.
        assert_eq!(out[0], "model");
        assert_eq!(out[1], "modal");
    }

    #[test]
    fn parses_multiple_flags_and_args() {
        let parsed =
            CommandParser::parse("/run foo --verbose --retries=3 bar").expect("should parse");
        assert_eq!(parsed.name, "run");
        assert_eq!(
            parsed.raw_args,
            vec![
                "foo".to_string(),
                "--verbose".to_string(),
                "--retries=3".to_string(),
                "bar".to_string()
            ]
        );
        assert_eq!(parsed.args, vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            parsed.flags.get("verbose").map(String::as_str),
            Some("true")
        );
        assert_eq!(parsed.flags.get("retries").map(String::as_str), Some("3"));
    }

    #[test]
    fn preserves_cli_shaped_raw_args_for_slash_mirrors() {
        let parsed = CommandParser::parse(
            "/video ingest https://example.test/watch?v=1 --frames hybrid --asr whisper-cpp --yes",
        )
        .expect("should parse");

        assert_eq!(
            parsed.raw_args,
            vec![
                "ingest".to_string(),
                "https://example.test/watch?v=1".to_string(),
                "--frames".to_string(),
                "hybrid".to_string(),
                "--asr".to_string(),
                "whisper-cpp".to_string(),
                "--yes".to_string()
            ]
        );
        assert_eq!(
            parsed.args,
            vec![
                "ingest".to_string(),
                "https://example.test/watch?v=1".to_string(),
                "hybrid".to_string(),
                "whisper-cpp".to_string()
            ]
        );
        assert_eq!(parsed.flags.get("frames").map(String::as_str), Some("true"));
        assert_eq!(parsed.flags.get("asr").map(String::as_str), Some("true"));
        assert_eq!(parsed.flags.get("yes").map(String::as_str), Some("true"));
    }
}
