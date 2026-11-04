use super::names;

pub(super) fn absent_remedy(tool: &str, name: &str, requirements: &[String]) -> String {
    let remedy = unavailable(tool, name);
    if requirements
        .iter()
        .any(|text| names(text, &format!("{tool} {name}")) || asks_to_add(text, name))
    {
        format!("{remedy}. {}", guard(tool, name))
    } else {
        remedy
    }
}

pub(super) fn guard(tool: &str, name: &str) -> String {
    format!(
        "If the deliverable adds it (for example an alias in the tree's configuration), make the check show first that the tree provides it: `{tool} --list | grep -qw {name} && {tool} {name} ...`, so that it fails by its own assertion before the implementation"
    )
}

#[path = "workflow_acceptance_requirement_addition.rs"]
mod addition;
use addition::asks_to_add;

pub(super) fn unavailable(tool: &str, name: &str) -> String {
    format!(
        "replace `{tool} {name}` with what the site provides; neither the site nor the host has it; the plugin program `{tool}-{name}` is not on the site's PATH; the check must use an available program or the deliverable must add it"
    )
}
