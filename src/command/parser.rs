//! Slash command parser.
//!
//! Pure function: takes raw user input (e.g. `"/effort high"`) and emits
//! a [`ParsedCommand`] describing the command name and its arguments.
//! No I/O, no async, no app state. Dispatch and registry live in
//! separate modules (TASK-AGS-622 / TASK-AGS-623).
//!
//! ## TASK-AGS-801 drift-reconcile + gap-fill
//!
//! The phase-8 spec (`TASK-AGS-801.md`) specifies a `CommandParser` struct
//! returning `Result<ParsedCommand, ParseError>` with an `Arg` newtype in
//! `args` and a flag map. The shipped code (from TASK-AGS-621 stub ->
//! TASK-AGS-622 real-impl) exposes a free function `parse` returning
//! `Option<ParsedCommand>` with `args: Vec<String>` and no flag map.
//!
//! Stage 6 orchestrator decision (Q1=A):
//! - Keep `fn parse() -> Option<...>` AS-IS for back-compat (dispatcher
//!   already reads `&parsed.args` as `&[String]`).
//! - Add `CommandParser::parse() -> Result<...>` as a thin wrapper.
//! - Extend `ParsedCommand` with `flags: HashMap<String, String>` and
//!   teach the existing tokenizer to populate it.
//! - Expose `Arg(pub String)` as a bonus type but do NOT change
//!   `args: Vec<String>`.
//! - Keep `pub(crate)` visibility (binary crate, no out-of-tree
//!   consumers).
//!
//! See the TASK-AGS-801 commit body for the full R-item list (R1
//! relocation, R2/R3/R4 type-drift, R5 behavior-rewrite scoped to the
//! `CommandParser` wrapper).

use std::collections::HashMap;
use std::str::FromStr;

use thiserror::Error;

/// A parsed slash command: the command name, its positional arguments
/// and any `--key[=value]` flags, in declaration order.
///
/// The `name` is the token immediately following the leading `/`, with
/// no case normalization applied — the dispatcher is responsible for
/// case-folding when looking the command up in the registry.
///
/// `args` retains the shipped `Vec<String>` type to keep the blast
/// radius into `dispatcher.rs` (which already treats `&parsed.args` as
/// `&[String]`) at zero. The `Arg` newtype in this module is exposed
/// for callers that want typed coercion helpers; the parser itself does
/// not return `Vec<Arg>`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ParsedCommand {
    pub name: String,
    pub args: Vec<String>,
    pub flags: HashMap<String, String>,
}

impl ParsedCommand {
    /// Return the positional argument at `idx`, if any.
    pub fn arg(&self, idx: usize) -> Option<&str> {
        self.args.get(idx).map(String::as_str)
    }

    /// Return the value of the `--key[=value]` flag, if present.
    ///
    /// Bare `--flag` tokens are recorded with value `"true"`.
    pub fn flag(&self, key: &str) -> Option<&str> {
        self.flags.get(key).map(String::as_str)
    }

    /// Return `true` if the `--key` flag is present (regardless of value).
    pub fn has_flag(&self, key: &str) -> bool {
        self.flags.contains_key(key)
    }
}

/// Newtype wrapper for a positional argument.
///
/// Exposed as a bonus type per the TASK-AGS-801 spec so callers can
/// perform typed coercion without the parser having to decide which
/// `FromStr` implementation to use. The parser itself returns raw
/// `String`s in `ParsedCommand::args`; callers wrap them in `Arg`
/// on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Arg(pub String);

impl Arg {
    /// Borrow the underlying string.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Attempt to parse the arg into any `FromStr` type.
    pub fn parse<T: FromStr>(&self) -> Result<T, T::Err> {
        self.0.parse::<T>()
    }
}

/// Errors returned by [`CommandParser::parse`].
///
/// The shipped free function `parse` returns `Option<ParsedCommand>`
/// and collapses all of these into a single `None`. The `CommandParser`
/// wrapper maps each failure mode to a specific variant so the TUI
/// error layer (TASK-AGS-804) can distinguish them.
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

/// Parse a raw user input line into a [`ParsedCommand`].
///
/// Returns `None` if the input is empty, does not start with `/`, or
/// consists of only the `/` sigil with no command name.
///
/// Tokenization is quote-aware: pairs of `"` delimit a single token
/// that may contain whitespace. The quote characters themselves are
/// stripped from the emitted token. Extra whitespace between tokens is
/// tolerated. The leading `/` is stripped before tokenization.
///
/// Tokens beginning with `--` are routed into `ParsedCommand::flags`:
/// - `--key=value` sets `flags["key"] = "value"`
/// - `--key` alone sets `flags["key"] = "true"`
///
/// Tokens beginning with a single `-` (e.g. `-v`) are treated as
/// positional args, matching the shipped behaviour.
///
/// The first token becomes [`ParsedCommand::name`] with no case
/// normalization — the dispatcher is responsible for case-folding
/// when looking up commands in the registry.
pub(crate) fn parse(input: &str) -> Option<ParsedCommand> {
    let trimmed = input.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        return None;
    }

    let body = &trimmed[1..];
    if body.is_empty() {
        return None;
    }

    let tokens = tokenize(body).ok()?;
    let mut iter = tokens.into_iter();
    let name = iter.next()?;
    if name.is_empty() {
        return None;
    }

    let mut args: Vec<String> = Vec::new();
    let mut flags: HashMap<String, String> = HashMap::new();
    for tok in iter {
        if let Some(rest) = tok.strip_prefix("--") {
            // Bare `--` or `--=x` is malformed for the Option-returning
            // path; we swallow malformed flags by dropping them so the
            // shipped behaviour (which never errored) is preserved. The
            // `CommandParser::parse` wrapper surfaces the structured
            // error instead.
            if rest.is_empty() {
                continue;
            }
            if let Some(eq_idx) = rest.find('=') {
                let (k, v) = rest.split_at(eq_idx);
                if k.is_empty() {
                    continue;
                }
                flags.insert(k.to_string(), v[1..].to_string());
            } else {
                flags.insert(rest.to_string(), "true".to_string());
            }
        } else {
            args.push(tok);
        }
    }

    Some(ParsedCommand { name, args, flags })
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

/// Spec-mandated wrapper returning `Result<ParsedCommand, ParseError>`.
///
/// Thin adapter over the free function [`parse`]:
/// - Prepends `/` if missing (spec validation criterion 2: `"model"`
///   parses the same as `"/model"`).
/// - Maps `None` to the appropriate [`ParseError`] variant:
///   - empty/whitespace-only input -> `Empty`
///   - `"/"` with no name -> `MissingName`
///   - otherwise -> `Empty` (defensive default, never reached by the
///     tokenizer — the only paths that produce `None` today are
///     empty-input or missing-name).
/// - Surfaces structured flag errors (`--=value`, bare `--`) via
///   [`ParseError::MalformedFlag`] that the free function silently
///   drops.
/// - Surfaces [`ParseError::UnclosedQuote`] from the tokenizer.
pub(crate) struct CommandParser;

impl CommandParser {
    pub(crate) fn parse(input: &str) -> Result<ParsedCommand, ParseError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(ParseError::Empty);
        }
        // R5: the wrapper relaxes the leading-`/` requirement. The
        // shipped `parse` fn stays strict for back-compat with the
        // dispatcher's `parser::parse(input)` call sites.
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

        // Run the tokenizer directly so we can surface structured
        // errors (`UnclosedQuote`, `MalformedFlag`) instead of the
        // shipped `parse` fn's `None`.
        let body = &normalized[1..];
        let tokens = tokenize(body)?;
        let mut iter = tokens.into_iter();
        let name = iter.next().ok_or(ParseError::MissingName)?;
        if name.is_empty() {
            return Err(ParseError::MissingName);
        }

        let mut args: Vec<String> = Vec::new();
        let mut flags: HashMap<String, String> = HashMap::new();
        for tok in iter {
            if let Some(rest) = tok.strip_prefix("--") {
                if rest.is_empty() {
                    return Err(ParseError::MalformedFlag(tok));
                }
                if let Some(eq_idx) = rest.find('=') {
                    let (k, v) = rest.split_at(eq_idx);
                    if k.is_empty() {
                        return Err(ParseError::MalformedFlag(tok));
                    }
                    flags.insert(k.to_string(), v[1..].to_string());
                } else {
                    flags.insert(rest.to_string(), "true".to_string());
                }
            } else {
                args.push(tok);
            }
        }

        Ok(ParsedCommand { name, args, flags })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------
    // Pre-existing 8 tests (TASK-AGS-622). Preserved verbatim modulo
    // the new `flags` field defaulting to an empty map.
    // ---------------------------------------------------------------

    #[test]
    fn parses_bare_slash_command() {
        assert_eq!(
            parse("/fast"),
            Some(ParsedCommand {
                name: "fast".to_string(),
                args: vec![],
                flags: HashMap::new(),
            }),
        );
    }

    #[test]
    fn parses_command_with_single_arg() {
        assert_eq!(
            parse("/effort high"),
            Some(ParsedCommand {
                name: "effort".to_string(),
                args: vec!["high".to_string()],
                flags: HashMap::new(),
            }),
        );
    }

    #[test]
    fn parses_config_subcommand() {
        assert_eq!(
            parse("/config sources"),
            Some(ParsedCommand {
                name: "config".to_string(),
                args: vec!["sources".to_string()],
                flags: HashMap::new(),
            }),
        );
    }

    #[test]
    fn parses_quoted_argument_with_spaces() {
        assert_eq!(
            parse("/rules edit r1 \"some text with spaces\""),
            Some(ParsedCommand {
                name: "rules".to_string(),
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
    fn rejects_non_slash_input() {
        assert_eq!(parse("not a slash command"), None);
    }

    #[test]
    fn rejects_empty_input() {
        assert_eq!(parse(""), None);
    }

    #[test]
    fn rejects_bare_slash() {
        assert_eq!(parse("/"), None);
    }

    #[test]
    fn tolerates_extra_whitespace() {
        assert_eq!(
            parse("/effort   high  "),
            Some(ParsedCommand {
                name: "effort".to_string(),
                args: vec!["high".to_string()],
                flags: HashMap::new(),
            }),
        );
    }

    // ---------------------------------------------------------------
    // New tests for TASK-AGS-801 (G1-G7).
    // ---------------------------------------------------------------

    #[test]
    fn parses_flag_with_value() {
        let parsed = parse("/model claude-4.5 --temperature=0.2")
            .expect("parse should succeed");
        assert_eq!(parsed.name, "model");
        assert_eq!(parsed.args, vec!["claude-4.5".to_string()]);
        assert_eq!(parsed.flag("temperature"), Some("0.2"));
        assert!(parsed.has_flag("temperature"));
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
        let parsed = parse("/export --format=json \"my session\"")
            .expect("parse should succeed");
        assert_eq!(parsed.name, "export");
        assert_eq!(parsed.args, vec!["my session".to_string()]);
        assert_eq!(parsed.flag("format"), Some("json"));
    }

    #[test]
    fn parses_bare_flag_as_true() {
        let parsed = parse("/fork --detach").expect("parse should succeed");
        assert_eq!(parsed.name, "fork");
        assert!(parsed.args.is_empty());
        assert_eq!(parsed.flag("detach"), Some("true"));
        assert!(parsed.has_flag("detach"));
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
    fn parsedcommand_arg_helper_returns_positional() {
        let parsed = parse("/effort high --quiet").unwrap();
        assert_eq!(parsed.arg(0), Some("high"));
        assert_eq!(parsed.arg(1), None);
        assert!(parsed.has_flag("quiet"));
    }

    #[test]
    fn arg_newtype_parses_typed_value() {
        let a = Arg("42".to_string());
        assert_eq!(a.as_str(), "42");
        let n: i32 = a.parse().expect("should parse as i32");
        assert_eq!(n, 42);
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
            parse("/run foo --verbose --retries=3 bar").expect("parse should succeed");
        assert_eq!(parsed.name, "run");
        assert_eq!(parsed.args, vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(parsed.flag("verbose"), Some("true"));
        assert_eq!(parsed.flag("retries"), Some("3"));
    }
}
