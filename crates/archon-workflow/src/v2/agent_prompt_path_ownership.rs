//! The `## Path Ownership` section of a verifier's prompt (Issue-85).
//!
//! Rendered from the stamp `verification::path_ownership` put on the item,
//! carrying the host's conclusion about who declares what and the rule that
//! follows from it: a defect confined to paths no task declares is a residual
//! gap, not a reason to withhold acceptance from the task under verification.
//! Empty for an item without a stamp, and with no stamp the exemption it
//! describes is simply unavailable.

use serde_json::Value;

use crate::v2::verification::path_ownership::stamped;

pub(crate) fn path_ownership_prompt_section(input: &Value) -> String {
    let Some(stamp) = stamped(input) else {
        return String::new();
    };
    let mut text = String::from(
        "## Path Ownership\n\
         Every path any task in this run declares as its own is listed below, so the lists are \
         complete: a repository path in NEITHER list is declared by no task at all. An entry \
         naming a directory covers every file beneath it, so read these by containment rather \
         than by exact match.\n",
    );
    if stamp.own_declared.is_empty() {
        text.push_str(
            "- Declared by this task (its writable scope): none recorded; treat every path as \
             outside its scope and rely on its deliverable contracts instead.\n",
        );
    } else {
        text.push_str(&format!(
            "- Declared by this task (its writable scope): {}\n",
            stamp.own_declared.join(", ")
        ));
    }
    if !stamp.declared_elsewhere.is_empty() {
        let listed: Vec<String> = stamp
            .declared_elsewhere
            .iter()
            .map(|entry| format!("{} (owned by {})", entry.path, entry.owner_task))
            .collect();
        text.push_str(&format!(
            "- Declared by another task: {}\n",
            listed.join(", ")
        ));
    }
    text.push_str(
        "Rule: a defect lying wholly outside this task's declared scope, in path(s) no task \
         declares, is a residual gap — record it naming the path and saying that no task \
         declares it — and NOT a reason to withhold acceptance from this task. No branch can \
         ever be dispatched to write a file no task owns, so a non-accepted verdict cannot \
         produce a fix and only repeats this cycle until the budget is gone. Judge this task on \
         what it owns: its declared deliverables, its declared tests, and the obligations it \
         implements. The exemption is narrow and never an escape hatch: it applies only when \
         the defect lies wholly outside this task's declared writable scope AND this task's own \
         deliverables and declared tests pass. A defect in a path listed above as this task's, \
         or any failure of its own declared tests, still fails the task exactly as before. A \
         defect in a path another task declares is that task's to fix: record it as a finding \
         naming that task, and never use this exemption for it.\n\n",
    );
    text
}

#[cfg(test)]
#[path = "agent_prompt_path_ownership_tests.rs"]
mod tests;
