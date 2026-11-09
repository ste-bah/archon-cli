use archon_workflow::requirement_state::CLASSIFICATION_PREFIX as PREFIX;
use std::collections::BTreeMap;
pub(super) fn body(body: &str, states: &BTreeMap<String, String>) -> String {
    let newline = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let clean = body
        .split_inclusive('\n')
        .filter(|line| !line.trim_start().starts_with(PREFIX))
        .collect::<String>();
    if states.is_empty() {
        return if body.lines().any(|l| l.trim_start().starts_with(PREFIX)) {
            clean
        } else {
            body.into()
        };
    }
    let ids = |state: &str| {
        let ids: Vec<_> = states
            .iter()
            .filter(|(_, s)| s.as_str() == state)
            .map(|(id, _)| id.as_str())
            .collect();
        if ids.is_empty() {
            "none".into()
        } else {
            ids.join(", ")
        }
    };
    format!(
        "{newline}{PREFIX} done: {}; to implement: {}; kept, not rebuilt: {}; inspection: {}.{newline}{}",
        ids("done"),
        ids("missing"),
        ids("undecided"),
        ids("inspection"),
        clean.trim_start_matches(['\r', '\n'])
    )
}

#[cfg(test)]
#[path = "workflow_host_command_binding_states_line_tests.rs"]
mod tests;
