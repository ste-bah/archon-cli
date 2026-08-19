//! `/branch` — fork a session from an earlier message (#192).
//!
//! `/fork` copies the whole log: "carry on from here in a separate session".
//! This is the other question — "go back to before that and try something
//! else" — which had no implementation at all, which is why the branch picker
//! built for it was never wired: it had nothing to call.
//!
//! With no argument it lists the branch points and opens the picker; with an
//! index it forks through that message, inclusive. The original session is
//! untouched: branching is not rewinding.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

const USAGE: &str = "Usage: /branch <message-number> [name]";

/// How much of a message to show in the list.
///
/// Long enough to recognise which turn it was, short enough that the picker
/// stays one row per message.
const SUMMARY_CHARS: usize = 72;

pub(crate) struct BranchHandler;

impl CommandHandler for BranchHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let (Some(store), Some(session_id)) = (ctx.session_store.clone(), ctx.session_id.clone())
        else {
            ctx.emit(TuiEvent::Error(
                "Branching is unavailable: no session store is attached.".to_string(),
            ));
            return Ok(());
        };

        let messages = match store.load_messages(&session_id) {
            Ok(messages) => messages,
            Err(error) => {
                ctx.emit(TuiEvent::Error(format!(
                    "Could not read the session: {error}"
                )));
                return Ok(());
            }
        };

        let points = branch_points(&messages);
        match args.first().map(String::as_str) {
            None | Some("") => {
                ctx.emit(TuiEvent::TextDelta(list_text(&points)));
                // Additive, like every other picker here: the text above is what
                // a print-mode run keeps, and the overlay is dropped there.
                ctx.emit(TuiEvent::ShowBranchPicker(points));
            }
            Some(raw) => {
                let Ok(index) = raw.parse::<usize>() else {
                    ctx.emit(TuiEvent::Error(format!(
                        "Not a message number: {raw}. {USAGE}"
                    )));
                    return Ok(());
                };
                if index >= messages.len() {
                    ctx.emit(TuiEvent::Error(format!(
                        "This session has {} messages, so {index} is not one of them. {USAGE}",
                        messages.len()
                    )));
                    return Ok(());
                }
                let name = args.get(1..).map(|rest| rest.join(" ")).unwrap_or_default();
                match archon_session::fork::fork_session_at(
                    &store,
                    &session_id,
                    index,
                    (!name.is_empty()).then_some(name.as_str()),
                ) {
                    Ok(new_id) => ctx.emit(TuiEvent::TextDelta(format!(
                        "\nBranched at message {index} as: {new_id}\n\
                         Resume with: archon --resume {new_id}\n\
                         Original session: {session_id}\n"
                    ))),
                    Err(error) => {
                        ctx.emit(TuiEvent::Error(format!("Branch failed: {error}")));
                    }
                }
            }
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Fork the session from an earlier message"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

/// The messages worth offering as branch points, as `(index, role, summary)`.
///
/// A message that is not a JSON object with a role is skipped: it is not a turn
/// anyone would recognise in a list, and branching at it would be branching at
/// something the reader cannot see.
fn branch_points(messages: &[String]) -> Vec<(usize, String, String)> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(index, raw)| {
            let value: serde_json::Value = serde_json::from_str(raw).ok()?;
            let role = value.get("role")?.as_str()?.to_string();
            Some((index, role, summarise(&value)))
        })
        .collect()
}

/// One line of a message.
///
/// Content is either a string or a list of blocks; a tool-use block has no text
/// to show, so it is named instead of skipped — "which turn was that" is easier
/// to answer from "used Bash" than from a blank row.
fn summarise(message: &serde_json::Value) -> String {
    let text = match message.get("content") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| {
                if let Some(text) = block.get("text").and_then(serde_json::Value::as_str) {
                    return Some(text.to_string());
                }
                block
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(|name| format!("used {name}"))
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };

    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= SUMMARY_CHARS {
        return collapsed;
    }
    // Truncate on a char boundary — a session in any non-ASCII language would
    // otherwise panic here rather than show a shorter line.
    let mut short: String = collapsed.chars().take(SUMMARY_CHARS - 1).collect();
    short.push('…');
    short
}

/// The text listing, which a print-mode run keeps.
fn list_text(points: &[(usize, String, String)]) -> String {
    if points.is_empty() {
        return "\nThis session has no messages to branch from.\n".to_string();
    }
    let mut out = format!("\nBranch points ({}):\n", points.len());
    for (index, role, summary) in points {
        out.push_str(&format!("  [{index}] {role}: {summary}\n"));
    }
    out.push_str(&format!("{USAGE}\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: serde_json::Value) -> String {
        serde_json::json!({ "role": role, "content": content }).to_string()
    }

    #[test]
    fn every_message_with_a_role_is_a_branch_point() {
        let messages = vec![
            message("user", serde_json::json!("add the parser")),
            message("assistant", serde_json::json!("here it is")),
        ];

        let points = branch_points(&messages);

        assert_eq!(points.len(), 2);
        assert_eq!(points[0], (0, "user".to_string(), "add the parser".into()));
        assert_eq!(points[1].0, 1);
    }

    /// Branching at something the reader cannot see is not a choice worth
    /// offering.
    #[test]
    fn messages_that_are_not_turns_are_skipped_and_the_indices_still_line_up() {
        let messages = vec![
            "not json".to_string(),
            message("user", serde_json::json!("real one")),
        ];

        let points = branch_points(&messages);

        assert_eq!(points.len(), 1);
        assert_eq!(
            points[0].0, 1,
            "the index must stay the position in the log, or the fork keeps the wrong messages"
        );
    }

    /// A tool call has no text. Naming the tool beats a blank row.
    #[test]
    fn a_tool_call_is_summarised_by_its_tool_name() {
        let summary = summarise(&serde_json::json!({
            "content": [{"type": "tool_use", "name": "Bash", "input": {}}]
        }));
        assert_eq!(summary, "used Bash");
    }

    #[test]
    fn a_long_message_is_shortened_on_a_character_boundary() {
        let summary = summarise(&serde_json::json!({ "content": "é".repeat(500) }));
        assert_eq!(summary.chars().count(), SUMMARY_CHARS);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn whitespace_is_collapsed_so_one_message_is_one_row() {
        let summary = summarise(&serde_json::json!({ "content": "line one\n\n   line two" }));
        assert_eq!(summary, "line one line two");
    }

    #[test]
    fn an_empty_session_is_reported_rather_than_listed_as_nothing() {
        assert!(list_text(&[]).contains("no messages to branch from"));
    }

    #[test]
    fn the_listing_names_the_index_to_pass_back() {
        let text = list_text(&[(3, "user".into(), "do the thing".into())]);
        assert!(text.contains("[3] user: do the thing"), "{text}");
        assert!(text.contains("Usage: /branch"), "{text}");
    }
}
