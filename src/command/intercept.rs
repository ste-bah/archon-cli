//! Recognising an upstream-intercepted command under every spelling it has.
//!
//! A few slash commands cannot run from `CommandHandler::execute`. Their
//! bodies need an `Arc<Mutex<Agent>>`, the full `SlashCommandContext`, or an
//! `.await` — none of which the synchronous handler signature can reach. Those
//! commands are intercepted upstream instead: `/clear` in
//! `session_loop::slash_dispatch`, `/config` in `command::slash`. Their
//! registered handlers exist so the registry can name and describe them, not
//! to do the work.
//!
//! Each interception used to be a literal comparison against the primary
//! name. The aliases the handler publishes appeared nowhere in it, so `/cls`
//! and `/settings` walked straight past the interception and into a handler
//! body that was `Ok(())` — no clear, no config, no error, no output at all.
//! For `/clear` that is not a missing convenience: clearing purges the log,
//! the compaction segments, their verbatim bodies, the compaction ledger and
//! every cached projection, so the silent spelling left a user believing a
//! conversation was gone while all of it stayed readable.
//!
//! [`command_args`] resolves the same set of spellings the registry resolves:
//! the primary plus `CommandHandler::aliases()`, which is the only source
//! `RegistryBuilder::build` reads when it indexes aliases. An alias added to a
//! handler is therefore intercepted by construction, not by someone
//! remembering to extend a match arm.

/// The argument text following `input`'s command name, if that name is one of
/// `spellings`.
///
/// Returns `Some("")` for a bare command and `Some(rest)` when arguments
/// follow, so a caller can tell "this command, no arguments" apart from "not
/// this command" — the distinction the old `trimmed == "/clear"` comparison
/// could not make, which is why `/clear` with a stray argument was as silent
/// as `/cls` was.
///
/// The name is the first whitespace-delimited token after the leading `/`,
/// which is the token `CommandParser::parse` hands the registry for lookup.
pub(crate) fn command_args<'a>(input: &'a str, spellings: &[&str]) -> Option<&'a str> {
    let trimmed = input.trim();
    let rest = trimmed.strip_prefix('/')?;
    let name_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (name, args) = rest.split_at(name_end);
    if spellings.contains(&name) {
        Some(args.trim())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::command_args;

    #[test]
    fn the_primary_name_matches_with_and_without_arguments() {
        assert_eq!(command_args("/clear", &["clear", "cls"]), Some(""));
        assert_eq!(command_args("  /clear  ", &["clear", "cls"]), Some(""));
        assert_eq!(
            command_args("/clear everything", &["clear", "cls"]),
            Some("everything")
        );
    }

    #[test]
    fn every_alias_matches_the_same_way_the_primary_does() {
        assert_eq!(command_args("/cls", &["clear", "cls"]), Some(""));
        assert_eq!(command_args("/cls now", &["clear", "cls"]), Some("now"));
        assert_eq!(
            command_args("/prefs model", &["config", "settings", "prefs"]),
            Some("model")
        );
    }

    #[test]
    fn a_name_that_merely_starts_with_the_command_is_not_the_command() {
        // `/clearing` is its own command name, not `/clear` with an argument.
        assert_eq!(command_args("/clearing", &["clear", "cls"]), None);
        assert_eq!(command_args("/close", &["clear", "cls"]), None);
        assert_eq!(command_args("clear", &["clear", "cls"]), None);
        assert_eq!(command_args("/", &["clear", "cls"]), None);
        assert_eq!(command_args("", &["clear", "cls"]), None);
    }
}
