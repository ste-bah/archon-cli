//! Slash command parser.
//!
//! Pure function: takes raw user input (e.g. `"/effort high"`) and emits
//! a [`ParsedCommand`] describing the command name and its arguments.
//! No I/O, no async, no app state. Dispatch and registry live in
//! separate modules (TASK-AGS-622 / TASK-AGS-623).
//!
//! Gate 1 ships only the type definition and a stub [`parse`] that
//! returns `None` for every input. Gate 2 lands the real implementation.

/// A parsed slash command: the command name and its positional
/// arguments, in declaration order.
///
/// The `name` is the token immediately following the leading `/`, with
/// no case normalization applied — the dispatcher is responsible for
/// case-folding when looking the command up in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedCommand {
    pub name: String,
    pub args: Vec<String>,
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

    let tokens = tokenize(body);
    let mut iter = tokens.into_iter();
    let name = iter.next()?;
    if name.is_empty() {
        return None;
    }
    let args: Vec<String> = iter.collect();

    Some(ParsedCommand { name, args })
}

/// Split `body` into tokens using a quote-aware scanner.
///
/// - `"` toggles an in-quotes state and is itself discarded.
/// - Whitespace outside quotes delimits tokens; runs of whitespace
///   collapse.
/// - Whitespace inside quotes is preserved verbatim.
/// - A trailing non-empty buffer at EOF is pushed as a final token.
fn tokenize(body: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in body.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }
        if ch.is_whitespace() && !in_quotes {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_slash_command() {
        assert_eq!(
            parse("/fast"),
            Some(ParsedCommand {
                name: "fast".to_string(),
                args: vec![],
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
            }),
        );
    }
}
