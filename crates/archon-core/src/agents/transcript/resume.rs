//! A stopped agent is never resumed by message, and a workflow's validation
//! repair continues only the effective context this process stored for it
//! (#241). Transcripts and their sidecars are history, never authority.

/// The refusal for a repair of an agent whose context this process does not
/// hold: another process started it, or its entry was collected.
pub(crate) fn unknown_context(agent_id: &str) -> String {
    format!(
        "cannot continue agent '{agent_id}': its confinement is only known to the process that started it; start a new agent"
    )
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
