//! `/workflow-prd-spec` — typed delegation to fixed host decomposition.
//!
//! The skill does not author prompts, run agents, or construct commands. It
//! derives the canonical task-root layout from the PRD filename and returns a
//! host-only [`SkillOutput::WorkflowDecompose`] request. The interactive host
//! owns provider selection, script structure, command capabilities, writes,
//! persistence, resume, and progress.

use std::path::PathBuf;

use super::{Skill, SkillContext, SkillOutput, WorkflowDecomposeRequest};

pub const TASK_ROOT: &str = "tasks";

pub fn workflow_task_dir(name: &str) -> String {
    format!("{TASK_ROOT}/PRD-{name}")
}

pub fn prd_id_from_path(path: &str) -> Option<String> {
    let stem = path
        .rsplit(['/', '\\'])
        .next()?
        .strip_suffix(".md")
        .or_else(|| path.rsplit(['/', '\\']).next())?;
    let id = stem.strip_prefix("PRD-")?.trim();
    (!id.is_empty()).then(|| id.to_string())
}

pub struct WorkflowPrdSpecSkill;

impl Skill for WorkflowPrdSpecSkill {
    fn name(&self) -> &str {
        "workflow-prd-spec"
    }

    fn description(&self) -> &str {
        "Delegate a workflow PRD to the fixed engine-native decomposition host."
    }

    fn aliases(&self) -> Vec<&str> {
        vec!["wf-prd-spec"]
    }

    fn execute(&self, args: &[String], _ctx: &SkillContext) -> SkillOutput {
        let Some(prd_path) = args.first() else {
            return SkillOutput::Error(
                "Usage: /workflow-prd-spec <path/to/PRD-<NAME>.md>".to_string(),
            );
        };
        if args.len() != 1 {
            return SkillOutput::Error(
                "/workflow-prd-spec accepts exactly one PRD path; the host derives the task root"
                    .to_string(),
            );
        }
        let Some(id) = prd_id_from_path(prd_path) else {
            return SkillOutput::Error(format!(
                "workflow PRD filename must be PRD-<NAME>.md, found {prd_path}"
            ));
        };
        SkillOutput::WorkflowDecompose(WorkflowDecomposeRequest {
            prd_path: PathBuf::from(prd_path),
            task_root: PathBuf::from(workflow_task_dir(&id)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> SkillContext {
        SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
            session_store: None,
        }
    }

    #[test]
    fn workflow_prd_spec_returns_typed_host_delegation() {
        let output = WorkflowPrdSpecSkill.execute(&["prds/PRD-ALERT-002.md".to_string()], &ctx());
        let SkillOutput::WorkflowDecompose(request) = output else {
            panic!("expected typed host delegation")
        };
        assert_eq!(request.prd_path, PathBuf::from("prds/PRD-ALERT-002.md"));
        assert_eq!(request.task_root, PathBuf::from("tasks/PRD-ALERT-002"));
    }

    #[test]
    fn workflow_prd_spec_never_returns_prompt_or_command_text() {
        let output = WorkflowPrdSpecSkill.execute(&["PRD-X-001.md".to_string()], &ctx());
        assert!(matches!(output, SkillOutput::WorkflowDecompose(_)));
        let source = include_str!("workflow_prd_spec.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        assert!(!production.contains("TaskCreate"));
        assert!(!production.contains("run Bash"));
        assert!(!production.contains("archon workflow decompose --"));
    }

    #[test]
    fn workflow_prd_spec_refuses_noncanonical_or_extra_arguments() {
        assert!(matches!(
            WorkflowPrdSpecSkill.execute(&["prds/PRD.md".into()], &ctx()),
            SkillOutput::Error(_)
        ));
        assert!(matches!(
            WorkflowPrdSpecSkill.execute(&["PRD-X-001.md".into(), "extra".into()], &ctx()),
            SkillOutput::Error(_)
        ));
    }
}
