//! Host-owned frontmatter stamping for one bound body landing.
//!
//! #428 follow-up: what the host cannot stamp because of the candidate itself
//! (no task file anchor, several task files of the bound subject, a
//! frontmatter the stamp cannot read) is the author's artifact being wrong.
//! It is refused back to the author with the body gate's shape refusal, the
//! same outcome the binding gives a body it cannot parse, so the author gets
//! its next attempt. Only the host's own state (the pin, the frozen skeleton,
//! the binding, the candidate stdin) fails the call.
//!
//! The binding refuses most such bodies first. What reaches here unbound is
//! a body the binding reads but the anchor does not: a frontmatter a JSON
//! mapping accepts and a YAML mapping refuses (a repeated key, the R8 shape),
//! or a task file inside another fence. A context bound in advance skips the
//! binding, so every case is refused here too.

use archon_workflow::{HostCommandRequest, WorkflowError, WorkflowResult};

use super::super::workflow_host_command_binding::{Unbound, UnparsedSubject};
use super::super::workflow_host_command_catalog::HostCommandResolutionContext;

/// Stamp the frozen frontmatter into a bound `land-task-body` candidate:
/// `Ok(Ok(notes))` names the fields replaced, `Ok(Err(_))` refuses the
/// candidate with a finding the author can act on, and `Err` is a host fault.
pub(super) fn stamp_request(
    request: &mut HostCommandRequest,
    context: &HostCommandResolutionContext,
) -> WorkflowResult<Result<Vec<String>, Unbound>> {
    if request.command_id != "land-task-body" {
        return Ok(Ok(Vec::new()));
    }
    let pin = super::super::workflow_host_command_postcondition::read_acceptance_pin(context)?;
    let skeleton = archon_workflow::task_skeleton::validate_full_chain(&context.task_root, &pin)
        .map_err(|error| WorkflowError::SpecInvalid(error.to_string()))?;
    let task_id = context.frozen_task_id.as_deref().ok_or_else(|| {
        WorkflowError::SpecInvalid("land-task-body has no host-bound frozen task id".into())
    })?;
    let frozen = skeleton
        .tasks
        .iter()
        .find(|task| task.task_id == task_id)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(format!(
                "bound task {task_id} is absent from frozen skeleton"
            ))
        })?;
    let candidate = request.stdin.as_deref().ok_or_else(|| {
        WorkflowError::SpecInvalid("land-task-body requires candidate stdin".into())
    })?;
    let path = context.frozen_task_file.as_ref().ok_or_else(|| {
        WorkflowError::SpecInvalid("land-task-body has no host-bound frozen task file".into())
    })?;
    let refused = |reason: String| {
        Ok(Err(Unbound::Unparsed(UnparsedSubject {
            task_file: path.clone(),
            reason,
        })))
    };
    let subjects = [(task_id, path.as_path())];
    let anchor = match crate::command::topology_lint::task_file_anchor(candidate, &subjects) {
        Ok(Some(anchor)) => anchor,
        Ok(None) => return refused(missing_anchor_finding(candidate, task_id)),
        Err(several) => {
            let ids: Vec<&str> = several.iter().map(|(id, _)| id.as_str()).collect();
            return refused(crate::command::topology_lint::several_task_files_finding(
                &ids,
            ));
        }
    };
    let (stamped, notes) =
        match super::super::workflow_host_command_binding::stamp_frozen_frontmatter(
            &candidate[anchor.opener..],
            frozen,
        ) {
            Ok(stamped) => stamped,
            // `stamp_frozen_frontmatter` reports the candidate's own faults
            // as `SpecInvalid`; anything else is the host's.
            Err(WorkflowError::SpecInvalid(cause)) => {
                return refused(format!(
                    "candidate TASK body for {task_id} does not parse: {cause}; return the whole task file"
                ));
            }
            Err(error) => return Err(error),
        };
    request.stdin = Some(format!("{}{}", &candidate[..anchor.opener], stamped));
    Ok(Ok(notes))
}

/// The finding for a bound candidate with no task file anchor: why the
/// ```` ```yaml ```` block that tries to be its frontmatter cannot open the
/// task file, and the form expected.
fn missing_anchor_finding(candidate: &str, task_id: &str) -> String {
    format!(
        "candidate TASK body for {task_id} has no task file frontmatter the host can anchor on: {}; return only the task file, starting with one top-level ```yaml frontmatter block that is a valid YAML mapping (each key once) with task_id: {task_id}",
        missing_anchor_cause(candidate, task_id)
    )
}

fn missing_anchor_cause(candidate: &str, task_id: &str) -> String {
    use crate::command::topology_lint::is_frontmatter_opener;
    let lines: Vec<&str> = candidate.lines().collect();
    let closes = |line: &&str| matches!(line.trim(), "```" | "---");
    let openers: Vec<usize> = (0..lines.len())
        .filter(|&index| is_frontmatter_opener(lines[index]))
        .collect();
    // The block that tries to be the frontmatter: the first naming a task_id
    // (a yaml example in chat before it is not it), else the first.
    let names_task_id = |&&opener: &&usize| {
        lines[opener + 1..]
            .iter()
            .take_while(|line| !closes(line))
            .any(|line| line.trim_start().starts_with("task_id:"))
    };
    let Some(&opener) = openers
        .iter()
        .find(names_task_id)
        .or_else(|| openers.first())
    else {
        return "the answer has no ```yaml frontmatter block".into();
    };
    let line = opener + 1;
    let Some(length) = lines[line..].iter().position(|line| closes(&line)) else {
        return format!("the ```yaml frontmatter block opened on line {line} is not closed");
    };
    // Padded so the YAML error's line numbers are the answer's own.
    let yaml = format!(
        "{}{}",
        "\n".repeat(line),
        lines[line..line + length].join("\n")
    );
    match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&yaml) {
        Err(error) => {
            format!(
                "the ```yaml frontmatter block opened on line {line} is not valid YAML: {error}"
            )
        }
        Ok(value) => match value.get("task_id").and_then(serde_yaml_ng::Value::as_str) {
            None => format!("the ```yaml frontmatter block opened on line {line} has no task_id"),
            Some(named) if named != task_id => {
                format!("the ```yaml frontmatter block opened on line {line} names task_id {named}")
            }
            Some(_) => format!(
                "the ```yaml frontmatter block opened on line {line} is inside another fenced block"
            ),
        },
    }
}

#[cfg(test)]
#[path = "workflow_host_command_exec_stamp_tests.rs"]
mod stamp_tests;
