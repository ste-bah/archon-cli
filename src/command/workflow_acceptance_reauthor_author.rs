//! The re-author agent call: one entry, read-only, told every finding.

use std::time::Duration;

use archon_core::agents::harness::ACCEPTANCE_REAUTHOR_AGENT;
use archon_workflow::llm_client_port::{
    WorkflowAgentCall, WorkflowAgentSpec, WorkflowAgentToolAccess,
};

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::*;
pub(super) fn author_prompt(
    scope: &AuthorScope,
    frozen: &AcceptanceCriterion,
    notes: &[String],
    attempt: usize,
) -> String {
    let baseline_pass = notes.iter().any(|note| {
        note.contains("criterion already holds on the pre-implementation tree")
            || note.contains(super::executability::CANNOT_FAIL)
    });
    let current = if baseline_pass {
        serde_json::json!({
            "id": frozen.id,
            "criterion": frozen.criterion,
            "covers": frozen.covers,
            "gap_permitted": frozen.gap_permitted,
        })
    } else {
        serde_json::json!({
            "id": frozen.id,
            "criterion": frozen.criterion,
            "check": frozen.check,
            "gap_permitted": frozen.gap_permitted,
        })
    };
    let mut prompt = Vec::new();
    if baseline_pass {
        prompt.extend([
            "The criterion already holds on the pre-implementation tree. write a regression guard. Run the existing test or implementation file that decides the criterion, and name that file relative to the chosen working directory. The check must fail on its own assertion when that file is moved aside. Do not generate substitute data.".to_string(),
            "Template: choose the root containing the named file; run the existing test or implementation that decides the criterion; assert the criterion's result; the check fails when the named file is moved aside.".to_string(),
        ]);
    }
    prompt.push(format!(
        "Re-author exactly one acceptance entry of a frozen acceptance contract: {}. The current entry cannot be used as it is: the host judge did not accept it, or it crashed in its own code when the host ran it (the findings below say which). Author ONLY this entry, not the whole contract.",
        frozen.id
    ));
    prompt.extend([
        format!("Read the PRD at {}.", scope.prd_path.display()),
        format!(
            "The code repository is {}. It is the ONLY place to verify source paths, test names, module layout and whether a file exists: a repository path you name must be one you observed there. Never descend into dependency, build-output or earlier-run directories.",
            scope.repository_root.display()
        ),
        format!(
            "{} is the project root. It holds the PRD and the task root; read source under the repository root alone.",
            scope.project_root.display()
        ),
        // Issue 366: the decomposition author's own text.
        live_root::check_path_rule(
            &scope.repository_root.display().to_string(),
            &scope.project_root.display().to_string(),
        ),
        "Return one JSON object with id, criterion, check, gap_permitted, judgment. The two examples below are ENTRIES showing the two check shapes; your reply is one such entry and nothing around it.".to_string(),
        ENTRY_SHAPES.to_string(),
        "For a command check, choose EXACTLY ONE command form. Block form: the JSON check object holds kind, cwd and \"command_block\": true, with NO \"command\" field, followed by exactly one non-empty fence whose info string is exactly 'check'. Inline form: the JSON check object holds kind, cwd and \"command\" only; command is one short line without quotes or backslashes. Include no command_block and no fence.".to_string(),
        "A check must exercise the deliverable and fail when its criterion is false, not merely match usage text or assert that a file exists. The host judges the entry adversarially against the criterion, then runs it once against the current tree: a check whose own script crashes (a syntax error, an undefined name, a call that does not match a helper it defines) is returned to you. Repairing a crash never licenses weakening the check: keep every assertion, fix only the script defect.".to_string(),
        "The host also runs the check on the tree before any implementation, where it must fail. If it passes there, data the check writes itself is never evidence; movable inputs must exist under the check's selected root before the run. To guard behaviour implemented in code, select the root that contains the existing implementation or test dependency and name it relative to that root. The host moves those inputs aside, never the check's own script, a program it runs, a directory it changes into or a build manifest; the check must still fail on its own assertion, not by crashing. Read the files that decide the criterion by those paths.".to_string(),
        format!(
            "Use the exact id {}. Criterion and judgment are host-owned placeholders; gap_permitted stays {}.",
            frozen.id, frozen.gap_permitted
        ),
        format!("The entry being replaced: {current}"),
    ]);
    if baseline_pass {
        prompt.extend([
            format!(
                "Do not repeat the refused approach: {}",
                refused_reason(notes)
            ),
            format!(
                "candidates to inspect; use only one that decides the criterion: {}",
                candidate_files(scope, frozen)
            ),
        ]);
    }
    prompt.extend([
        format!(
            "Attempt {attempt}; the host stops after {REAUTHOR_ATTEMPTS} consecutive attempts that repair no check. Findings to fix, oldest first:\n- {}",
            notes.join("\n- ")
        ),
        "Return exactly the entry: for inline form, output only the JSON entry (raw or in one `json` fence); for block form, output the JSON entry (raw or in one `json` fence) followed by exactly one non-empty fence labelled `check`. Emit no prose, explanations, headings or other fences.".to_string(),
        "Do not run commands or write files.".to_string(),
    ]);
    prompt.join("\n")
}

fn refused_reason(notes: &[String]) -> &'static str {
    if notes
        .iter()
        .any(|note| note.contains("data the check writes itself"))
    {
        "it relied on data created during the check"
    } else {
        "it did not prove the criterion using an existing deciding file"
    }
}

pub(crate) fn candidate_files(scope: &AuthorScope, frozen: &AcceptanceCriterion) -> String {
    let project_input_excludes = match crate::command::acceptance_scratch_policy::capture(
        &scope.project_root,
        &scope.task_root,
    ) {
        Ok(Some(binding)) => binding.policy.project_input_excludes,
        Ok(None) => Vec::new(),
        Err(error) => {
            eprintln!(
                "acceptance re-author candidate discovery: project input copy policy could not be loaded: {error}"
            );
            return "none could be verified".into();
        }
    };
    let mut paths = BTreeSet::new();
    for task in &scope.task_universe.tasks {
        if !task.implements.iter().any(|id| frozen.covers.contains(id)) {
            continue;
        }
        let mut declared = task
            .files_expected_to_change
            .iter()
            .filter_map(|item| item.split_whitespace().next().map(str::to_string))
            .collect::<Vec<_>>();
        declared.extend(
            task.declared_focused_test_commands()
                .into_iter()
                .flat_map(|command| {
                    command
                        .split_whitespace()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                }),
        );
        for raw in declared {
            let candidate =
                raw.trim_matches(|c: char| !c.is_ascii_alphanumeric() && !"._/-\\".contains(c));
            if candidate.is_empty() {
                continue;
            }
            let declared_path = Path::new(candidate);
            let roots = if declared_path.is_absolute() {
                let canonical_candidate = declared_path.canonicalize().ok();
                [
                    ("repository root", &scope.repository_root),
                    ("project root", &scope.project_root),
                ]
                .into_iter()
                .filter_map(|(label, root)| {
                    let canonical_root = root.canonicalize().ok()?;
                    let canonical_candidate = canonical_candidate.as_ref()?;
                    let relative = canonical_candidate.strip_prefix(canonical_root).ok()?;
                    Some((label, root, relative.to_path_buf()))
                })
                .collect::<Vec<_>>()
            } else {
                vec![
                    (
                        "repository root",
                        &scope.repository_root,
                        declared_path.to_path_buf(),
                    ),
                    (
                        "project root",
                        &scope.project_root,
                        declared_path.to_path_buf(),
                    ),
                ]
            };
            for (label, root, relative) in roots {
                let Some((relative, relative_text)) = normalized_relative_path(&relative) else {
                    continue;
                };
                let inside_tree = if label == "repository root" {
                    scope.baseline.as_ref().is_some_and(|baseline| {
                        crate::command::workflow_task_set::executability::repository_tree_contains(
                            &baseline.repository,
                            &baseline.commit,
                            &relative_text,
                        )
                    })
                } else {
                    let project_relative_task_root =
                        scope.task_root.strip_prefix(&scope.project_root).ok();
                    let nested_repository = archon_workflow::acceptance_scratch::nested_repository(
                        &scope.project_root,
                        &scope.repository_root,
                    );
                    let excluded = archon_workflow::acceptance_scratch::acceptance_input_excluded(
                        &relative,
                        &project_input_excludes,
                        project_relative_task_root,
                    ) || nested_repository.as_ref().is_some_and(|nested| {
                        archon_workflow::acceptance_scratch::nested_repository_skipped(
                            &relative,
                            Some(nested.as_path()),
                        )
                    });
                    !excluded && project_candidate_is_copied_file(root, &relative)
                };
                if inside_tree {
                    paths.insert(format!("{label}: {relative_text}"));
                }
            }
        }
    }
    if paths.is_empty() {
        "none found".into()
    } else {
        paths.into_iter().collect::<Vec<_>>().join(", ")
    }
}

/// Drop harmless `.` components and reject any path that could escape or
/// select a different root. Keep a platform path for filesystem access and
/// its shared, forward-slashed spelling for Git and prompt text.
fn normalized_relative_path(path: &Path) -> Option<(PathBuf, String)> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    if normalized.as_os_str().is_empty() {
        return None;
    }
    let text = archon_shell::paths::stored_text(&normalized);
    Some((normalized, text))
}

/// The project input copier skips symlink entries. A declared path must walk
/// through ordinary directories and end at an ordinary file so the scratch
/// copy contains the path the prompt names.
fn project_candidate_is_copied_file(project_root: &Path, relative: &Path) -> bool {
    let mut current = project_root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        current.push(name);
        let Ok(metadata) = current.symlink_metadata() else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            return false;
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return false;
            }
        } else if !metadata.is_file() {
            return false;
        }
    }
    !relative.as_os_str().is_empty()
}

/// The live root forms a re-authored check must not name (Issue 366): the
/// roots the prompt names, absolute, and every root the probe refuses.
pub(super) fn refused_forms(
    scope: &AuthorScope,
    probe: &dyn super::executability::ExecutabilityProbe,
) -> Vec<PathBuf> {
    let absolute = |root: &PathBuf| std::path::absolute(root).unwrap_or_else(|_| root.clone());
    let mut roots = vec![
        absolute(&scope.repository_root),
        absolute(&scope.project_root),
    ];
    roots.extend(probe.refused_roots());
    live_root::root_forms(roots.iter().map(PathBuf::as_path))
}

/// A candidate whose check names a live root by its absolute path never
/// reaches the judge: it goes back to its author in the freeze probe's own
/// words, as the decomposition author step refuses it.
pub(super) fn live_root_refusal(
    candidate: AcceptanceCriterion,
    forms: &[PathBuf],
) -> std::result::Result<AcceptanceCriterion, String> {
    let named = super::executability::executed_text(&candidate)
        .and_then(|(_, text)| live_root::named_root(text, forms));
    match named {
        Some(root) => Err(live_root::live_root_finding(&candidate.id, root)),
        None => Ok(candidate),
    }
}

pub(super) async fn author_entry(
    client: &dyn WorkflowLlmClient,
    scope: &AuthorScope,
    frozen: &AcceptanceCriterion,
    notes: &[String],
    attempt: usize,
) -> Result<String> {
    let read_roots = scope.read_roots().map_err(|error| anyhow!(error))?;
    let prompt = author_prompt(scope, frozen, notes, attempt);
    let call = WorkflowAgentCall {
        session_id: format!(
            "{ACCEPTANCE_REAUTHOR_AGENT}-{}-{}",
            frozen.id,
            uuid::Uuid::new_v4()
        ),
        task: prompt.clone(),
        cwd: Some(scope.repository_root.clone()),
        ordinal: 0,
        attempt,
        agent: WorkflowAgentSpec {
            // A host agent registered in every project: a key only a project's
            // `.archon/agents` defines fails to launch everywhere else.
            key: ACCEPTANCE_REAUTHOR_AGENT.into(),
            display_name: "acceptance reauthor".into(),
            model: "sonnet".into(),
            phase: 0,
            critical: false,
            parallelizable: false,
            quality_threshold: 0.5,
            tool_access: WorkflowAgentToolAccess::ReadOnly,
        },
        messages: vec![serde_json::json!({ "role": "user", "content": prompt })],
        system: Vec::new(),
        tools: Vec::new(),
        allowed_tools: std::iter::once(EXACT_TOOL_POLICY_MARKER)
            .chain(AUTHOR_TOOLS)
            .map(str::to_string)
            .collect(),
        timeout_secs: Some(judge::JUDGE_TIMEOUT_SECS),
        disable_auto_background: true,
        // The author runs in the repository and is confined to it, so every
        // other place its prompt names is passed by name (#236).
        read_roots,
        write_roots: Vec::new(),
        provider_env: None,
    };
    let outcome = tokio::time::timeout(
        Duration::from_secs(judge::JUDGE_TIMEOUT_SECS),
        client.run_agent(call),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "acceptance re-author for '{}' timed out after {}s",
            frozen.id,
            judge::JUDGE_TIMEOUT_SECS
        )
    })?
    .map_err(anyhow::Error::new)
    .with_context(|| format!("re-authoring acceptance check '{}'", frozen.id))?;
    // An incomplete reply is not an entry: it is fed back like one that does
    // not parse, and costs the attempt.
    if !judge::reply_is_complete(&outcome) || outcome.content.is_empty() {
        return Ok(String::new());
    }
    Ok(outcome.content)
}
