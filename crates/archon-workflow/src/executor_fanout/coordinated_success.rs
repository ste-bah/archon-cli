pub(super) fn coordinated_target_files(
    outcome: &crate::write_coordinator::CoordinatedOutcome,
    item_id: &str,
) -> Vec<String> {
    outcome
        .plans
        .iter()
        .find(|plan| plan.item_id == item_id)
        .map(|plan| {
            plan.target_files
                .iter()
                .map(|path| path.as_str().to_string())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn coordinated_accepted_item_body(
    outcome: &crate::write_coordinator::CoordinatedOutcome,
    item_id: &str,
) -> String {
    let changed_files = outcome
        .plans
        .iter()
        .find(|plan| plan.item_id == item_id)
        .map(|plan| plan.changed_files.clone())
        .unwrap_or_default();
    let target_files = coordinated_target_files(outcome, item_id);
    let target_lines = markdown_list(&target_files);
    let changed_lines = markdown_list(&changed_files);

    format!(
        "# Coordinated item `{item_id}`\n\n\
status: accepted\n\
target_files:\n{target_lines}\
changed_files:\n{changed_lines}\
acceptance_checks:\n\
  - write coordinator applied the item patch or verified idempotent completion\n\
  - declared target files were tracked in the patch manifest\n\
commands_run:\n\
  - command: write-coordinator patch capture and apply validation\n\
    exit_status: 0\n\
residual_gaps: []\n"
    )
}

fn markdown_list(values: &[String]) -> String {
    if values.is_empty() {
        return "  - none\n".to_string();
    }
    values
        .iter()
        .map(|value| format!("  - {value}\n"))
        .collect::<String>()
}
