//! Binding a candidate TASK body to the one frozen subject it belongs to.

use std::path::{Path, PathBuf};

use archon_workflow::{HostCommandRequest, HostCommandResult, WorkflowError, WorkflowResult};

use super::workflow_host_command_catalog::HostCommandResolutionContext;
use super::workflow_host_command_decision::{
    candidate_refusal_envelope, shape_refusal_evaluation, unpublished,
};
use super::workflow_host_command_postcondition::read_acceptance_pin;

pub(crate) fn unbound_context(
    base: &HostCommandResolutionContext,
    run_root: &Path,
) -> HostCommandResolutionContext {
    let mut context = base.clone();
    context.run_staging_root = run_root.join("host-command-staging");
    context
}

/// Why a candidate body binds no single frozen subject.
#[derive(Debug)]
pub(crate) enum Unbound {
    /// It names a frozen subject and does not parse as that subject's file
    /// (the parse cause), or holds several frozen subjects' task files:
    /// refused as a body shape.
    Unparsed(UnparsedSubject),
    /// It binds zero (naming none) or several subjects.
    Binding(String),
}

impl Unbound {
    fn reason(&self) -> &str {
        match self {
            Self::Unparsed(subject) => &subject.reason,
            Self::Binding(reason) => reason,
        }
    }

    /// The refusal the author gets, as an unpublished outcome.
    pub(crate) fn refused(self, command_id: &str) -> WorkflowResult<HostCommandResult> {
        let envelope = match self {
            Self::Unparsed(subject) => subject.envelope()?,
            Self::Binding(reason) => candidate_refusal_envelope(command_id, &reason),
        };
        Ok(unpublished(
            Some(0),
            String::new(),
            String::new(),
            (0, 0),
            envelope,
            "candidate refused before staging",
        ))
    }
}

/// Which frozen subject a candidate body belongs to.
///
/// Exactly one frozen task must accept the candidate as its own file. Zero or
/// several is the author's artifact being wrong, reported as [`SpecInvalid`] so
/// the caller can refuse the candidate and give the author its next attempt.
pub(crate) fn context_for_request(
    base: &HostCommandResolutionContext,
    run_root: &Path,
    request: &HostCommandRequest,
) -> WorkflowResult<HostCommandResolutionContext> {
    bind_request(base, run_root, request)?
        .map_err(|unbound| WorkflowError::SpecInvalid(unbound.reason().to_string()))
}

/// [`context_for_request`], keeping why a candidate is unbound, so `execute`
/// can refuse it with the cause the author can act on.
pub(crate) fn bind_request(
    base: &HostCommandResolutionContext,
    run_root: &Path,
    request: &HostCommandRequest,
) -> WorkflowResult<Result<HostCommandResolutionContext, Unbound>> {
    let mut context = unbound_context(base, run_root);
    if request.command_id != "land-task-body"
        || (context.frozen_task_id.is_some() && context.frozen_task_file.is_some())
    {
        return Ok(Ok(context));
    }
    let candidate = request.stdin.as_deref().ok_or_else(|| {
        WorkflowError::SpecInvalid("land-task-body requires candidate stdin".to_string())
    })?;
    // Issue 294: the pin, skeleton and bodies are read as one version.
    let _read = crate::command::workflow_task_set::ChainRead::workflow(
        &context.project_root,
        &context.task_root,
    )?;
    let pin = read_acceptance_pin(&context)?;
    let skeleton = archon_workflow::task_skeleton::validate_full_chain(&context.task_root, &pin)
        .map_err(|error| WorkflowError::SpecInvalid(error.to_string()))?;
    // The task file opens where the body gate's packaging strip says it does,
    // so a yaml block in chat before it is never read as its frontmatter. The
    // anchor is the one frozen subject's task file in the answer; two or more
    // are ambiguous and refused, never picked.
    //
    // Limit: the request carries no target, so the host cannot check that
    // the subject bound here is the one THIS call authors. Follow-up issue:
    // "land-task-body cannot verify the call's target subject".
    let paths: Vec<_> = skeleton
        .tasks
        .iter()
        .map(|frozen| context.task_root.join(&frozen.file_name))
        .collect();
    let subjects: Vec<_> = skeleton
        .tasks
        .iter()
        .zip(&paths)
        .map(|(frozen, path)| (frozen.task_id.as_str(), path.as_path()))
        .collect();
    let anchor = match crate::command::topology_lint::task_file_anchor(candidate, &subjects) {
        Ok(anchor) => anchor,
        Err(several) => {
            let ids: Vec<&str> = several.iter().map(|(id, _)| id.as_str()).collect();
            return Ok(Err(Unbound::Unparsed(UnparsedSubject {
                task_file: paths[several[0].1].clone(),
                reason: crate::command::topology_lint::several_task_files_finding(&ids),
            })));
        }
    };
    let task_file = anchor
        .as_ref()
        .map_or(candidate, |anchor| &candidate[anchor.opener..]);
    let mut matches = Vec::new();
    for frozen in &skeleton.tasks {
        let path = context.task_root.join(&frozen.file_name);
        if archon_workflow::task_universe::parsing::parse_task_file(&path, task_file).is_ok() {
            matches.push((frozen.task_id.clone(), path));
        }
    }
    if matches.is_empty()
        && let Some(anchor) = &anchor
        && let Some(subject) =
            named_subject_refusal(&skeleton.tasks, &context.task_root, candidate, anchor)
    {
        return Ok(Err(Unbound::Unparsed(subject)));
    }
    if matches.len() != 1 {
        return Ok(Err(Unbound::Binding(format!(
            "candidate TASK body binds {} frozen subjects; return exactly one body preserving a frozen task_id and file_name",
            matches.len()
        ))));
    }
    let (task_id, task_file) = matches.pop().expect("one candidate subject");
    context.frozen_task_id = Some(task_id);
    context.frozen_task_file = Some(task_file);
    Ok(Ok(context))
}

/// A candidate that binds no frozen subject but names one by its `task_id`
/// line: that subject's file, and why the candidate does not parse as it.
#[derive(Debug)]
pub(crate) struct UnparsedSubject {
    pub(crate) task_file: PathBuf,
    pub(crate) reason: String,
}

impl UnparsedSubject {
    /// The shape refusal the child would stage, built by the host: one
    /// `Body` finding with the parse cause.
    fn envelope(self) -> WorkflowResult<archon_workflow::GateEnvelopeV1> {
        shape_refusal_evaluation(&self.task_file, self.reason)
            .into_envelope()
            .map_err(|error| WorkflowError::StateCorrupt(error.to_string()))
    }
}

/// The subject the task file's frontmatter names ([`task_file_anchor`]),
/// when it is a frozen one the task file fails to parse as.
///
/// [`task_file_anchor`]: crate::command::topology_lint::task_file_anchor
fn named_subject_refusal(
    tasks: &[archon_workflow::task_skeleton::FrozenTask],
    task_root: &Path,
    candidate: &str,
    anchor: &crate::command::topology_lint::TaskFileAnchor,
) -> Option<UnparsedSubject> {
    let named = anchor.task_id.as_str();
    let frozen = tasks.iter().find(|frozen| frozen.task_id == named)?;
    let task_file = task_root.join(&frozen.file_name);
    let block = &candidate[anchor.opener..];
    let error =
        archon_workflow::task_universe::parsing::parse_task_file(&task_file, block).err()?;
    let cause = unclosed_frontmatter(block, candidate).unwrap_or_else(|| match error {
        WorkflowError::SpecInvalid(text) => text,
        other => other.to_string(),
    });
    Some(UnparsedSubject {
        reason: format!(
            "candidate TASK body for {named} does not parse: {cause}; return the whole task file"
        ),
        task_file,
    })
}

/// The cause for a candidate whose frontmatter `block` (from its
/// ```` ```yaml ```` line) never closes (the parser reads that as no block at
/// all): where the answer ends, and its last 40 characters.
fn unclosed_frontmatter(block: &str, candidate: &str) -> Option<String> {
    if block
        .lines()
        .skip(1)
        .any(|line| matches!(line.trim(), "```" | "---"))
    {
        return None;
    }
    let ends = candidate.chars().count();
    let tail: String = candidate
        .chars()
        .skip(ends.saturating_sub(40))
        .collect::<String>()
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    Some(format!(
        "the ```yaml frontmatter block is not closed; the answer ends at char {ends} with '{tail}'"
    ))
}
