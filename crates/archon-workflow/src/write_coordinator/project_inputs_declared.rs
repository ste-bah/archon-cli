//! Batch G2: a write branch's declared project artifacts land the way its
//! project data does.
//!
//! Issue-124 left one live path open from a write branch into the project
//! root: each file its call declared as a project artifact was stamped
//! writable where the host judges it, and the tripwire left a write there to
//! the call as "its delivery". A branch's shell could therefore rewrite a
//! live project file directly, unaudited, while Batch E already landed the
//! same kind of change -- the project's data -- from the branch's own copy,
//! with a baseline, a stale refusal, placement rules, a kept copy of what it
//! replaced and an append-only log (`patch_apply::project_inputs_apply`).
//!
//! So a declared project artifact no longer has a live grant. Where it lies
//! under the acceptance policy's inputs, Batch E seeds and lands it already.
//! Any other declared artifact under the project root is seeded, captured
//! and landed by the same code, one file each (`SeedRecord::declared`):
//!
//! - at the same relative path in the worktree when the worktree's git
//!   ignores and does not track it (as the inputs are);
//! - through the branch's patch, untouched here, when the project IS the
//!   repository and git carries the path;
//! - otherwise in a staging directory of the branch's own beside its
//!   worktree ([`staging_dir`]), since the path in the worktree is the
//!   repository's.
//!
//! The placement rule is [`ProjectInputPolicy::placed`] with `declared`:
//! the inputs' rule, for that exact declared file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{MAX_PROJECT_INPUT_BYTES, ProjectInputPolicy};

/// Where the branch's copy of one declared project artifact is, and the
/// project root's state of it when the branch was seeded: the baseline its
/// landing must still find there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredCopy {
    pub copy: PathBuf,
    pub baseline: String,
}

impl ProjectInputPolicy {
    /// What a landing of a branch's project data or declared artifacts is
    /// judged by: the run's recorded acceptance policy, inputs or not, or --
    /// for a run that recorded none -- the project root the run lives in,
    /// with no inputs. `None` when the run directory is not inside it.
    pub fn for_landing(run_root: &Path) -> Option<Self> {
        if let Some(policy) = Self::recorded(run_root) {
            return Some(policy);
        }
        let context = crate::v2::project_artifacts::project_artifact_context_from_v2_root(
            &run_root.join("v2"),
        );
        let project = PathBuf::from(context.project_root?)
            .canonicalize()
            .map(archon_shell::paths::plain)
            .ok()?;
        if !run_root
            .canonicalize()
            .map(archon_shell::paths::plain)
            .ok()?
            .starts_with(&project)
        {
            return None;
        }
        let task_root = recorded_task_root(run_root).unwrap_or_else(|| project.join("tasks"));
        Some(Self {
            task_root: task_root
                .canonicalize()
                .map(archon_shell::paths::plain)
                .unwrap_or(task_root),
            project,
            inputs: Vec::new(),
            excludes: Vec::new(),
            limit: MAX_PROJECT_INPUT_BYTES,
            combined: false,
            external: Default::default(),
        })
    }
}

/// The task set root the run's launch snapshot names, when it names one.
fn recorded_task_root(run_root: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(run_root.join("v2/generated-metadata.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .pointer("/observer_snapshot/canonical_task_root_identity")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// The branch's own staging directory for declared project artifacts the
/// worktree cannot hold (their path there is the repository's): beside its
/// worktree, where the host plants agent-writable trees and never keeps its
/// own records (the run-store guard's `v2/worktrees` subtree).
pub fn staging_dir(worktree: &Path) -> PathBuf {
    let name = worktree
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "worktree".into());
    worktree.with_file_name(format!("{name}.project-artifacts"))
}

/// The project artifacts a call declares (its `required_artifacts` and its
/// input's artifact requirements, host-resolved exactly as its prompt and
/// its completion check resolve them), relative to the project root of
/// `context`, sorted.
///
/// Issue-226: when the run's policy lists external data directories
/// (`context.external_roots`), every declaration of an absolute path
/// outside the project root is kept too, as that absolute path: one the
/// policy admits lands there, and any other is refused by name when the
/// branch is seeded. With none listed, an artifact outside the project root
/// is not the project's to land, as before.
pub fn declared_rel_paths(
    input: &serde_json::Value,
    required: &[crate::v2::WorkflowV2ArtifactRequirement],
    context: &crate::v2::WorkflowV2ProjectArtifactContext,
) -> Vec<String> {
    let Some(project_root) = context.project_root.as_deref() else {
        return Vec::new();
    };
    let declared =
        crate::v2::project_artifact_prompt::declared_project_artifacts(input, required, context);
    let mut rels: Vec<String> = declared
        .entries
        .into_iter()
        .filter_map(|(_, absolute)| {
            let Ok(rel) = Path::new(&absolute).strip_prefix(project_root) else {
                return Some(absolute);
            };
            let rel = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            (!rel.is_empty()).then_some(rel)
        })
        .collect();
    if !context.external_roots.is_empty() {
        // The ones the policy does not admit, so the seed names each refusal.
        let external =
            super::ExternalRoots::from_allowed(context.external_roots.iter().map(PathBuf::from));
        let raw = (required.iter().map(|requirement| requirement.path.clone()))
            .chain(crate::v2::project_artifact_contract::artifact_requirement_paths(input));
        rels.extend(raw.map(|raw| raw.trim().to_string()).filter(|raw| {
            let path = Path::new(raw);
            path.is_absolute() && !path.starts_with(project_root) && external.admit(path).is_err()
        }));
    }
    rels.sort();
    rels.dedup();
    rels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(project: &Path) -> ProjectInputPolicy {
        ProjectInputPolicy {
            project: project.to_path_buf(),
            inputs: vec![PathBuf::from("data")],
            excludes: Vec::new(),
            task_root: project.join("tasks"),
            limit: MAX_PROJECT_INPUT_BYTES,
            combined: true,
            external: Default::default(),
        }
    }

    #[test]
    fn a_declared_file_outside_the_inputs_is_placed_and_every_other_rule_holds() {
        let project = Path::new("/p");
        let policy = policy(project);
        assert_eq!(
            policy.placed("docs/reports/audit.md", true),
            Ok(project.join("docs/reports/audit.md"))
        );
        assert_eq!(
            policy.placed("reports/summary.json", true),
            Ok(project.join("reports/summary.json"))
        );
        assert!(policy.placed(".archon/lab/out.json", true).is_ok());
        // Not declared: outside the inputs is refused, as before.
        assert!(policy.destination("docs/reports/audit.md").is_err());
        for refused in [
            "tasks/TASK-1.md",
            "prds/prd.md",
            "config/app.toml",
            ".github/workflows/ci.yml",
            ".claude/settings.json",
            ".archon/config.toml",
            ".archon/workflows/wf/state.json",
            ".archon/agents/x/y.md",
            ".archon/docs/engine.md",
            "reports/.env",
            "config.toml",
            "../escape.md",
            "/abs/path.md",
            "a/.git/config",
        ] {
            assert!(policy.placed(refused, true).is_err(), "{refused}");
        }
    }

    #[test]
    fn declared_paths_are_the_project_relative_resolved_declarations() {
        // Absolute on either platform: `/elsewhere` is not absolute on Windows,
        // so it would not be recognised as the outside path it stands for.
        let (root, outside) = if cfg!(windows) {
            (r"C:\p", "C:/elsewhere/out.json")
        } else {
            ("/p", "/elsewhere/out.json")
        };
        let input = serde_json::json!({"item": {"artifact_requirements": [
            "docs/audit.md", outside, "{{project_root}}/reports/a.json"
        ]}});
        let context = crate::v2::WorkflowV2ProjectArtifactContext {
            project_root: Some(root.into()),
            ..Default::default()
        };
        let rels = declared_rel_paths(&input, &[], &context);
        assert!(rels.contains(&"docs/audit.md".to_string()), "{rels:?}");
        assert!(
            !rels.iter().any(|rel| rel.contains("elsewhere")),
            "{rels:?}"
        );
    }
}
