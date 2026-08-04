//! `/workflow-prd-spec` — decompose a workflow PRD into its task directory.
//!
//! Second half of the workflow PRD pipeline. Writes a *flat* directory of
//! `TASK-<DOMAIN>-<NNN>-<slug>.md` files under `tasks/PRD-<NAME>/`.
//!
//! # Why the layout is not negotiable
//!
//! The workflow engine discovers these files with a **single non-recursive
//! read** of the directory for names matching `TASK-*.md`. A task one level
//! deeper is not found at all — not warned about, not partially loaded. The
//! canonical task id is the first three dash-separated parts of the filename
//! stem and must equal the file's own `task_id:`; a mismatch is refused naming
//! both. Ten YAML keys are required by presence, and a file missing any of them
//! is refused naming the file and the keys.
//!
//! Because discovery reads a named directory rather than a path relative to the
//! PRD, the task directory does not need to sit beside the PRD, which is what
//! lets both pipelines share the fixed `prds/` and `tasks/` roots.

use super::{Skill, SkillContext, SkillOutput, templates};

/// Root under which every decomposed task set lives, for both pipelines.
pub const TASK_ROOT: &str = "tasks";

/// Directory the workflow task files for `name` are written into, relative to
/// the working directory.
///
/// Pinned by a test in the binary crate that builds this exact path on disk and
/// asserts the workflow engine's directory walk finds the tasks in it — so the
/// skill's output location and the engine's discovery cannot drift apart
/// without a test failing.
pub fn workflow_task_dir(name: &str) -> String {
    format!("{TASK_ROOT}/PRD-{name}")
}

/// The PRD id embedded in a workflow PRD path.
///
/// `prds/PRD-ALERT-002/PRD-ALERT-002.md` → `ALERT-002`. Returns `None` for a
/// path whose file stem does not start with `PRD-`, which includes the skills
/// chain's `prds/<slug>/PRD.md` — that pipeline is not decomposed by this skill
/// and silently accepting its path would write a task set neither pipeline
/// reads.
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
        "Decompose a workflow PRD into a flat TASK-<DOMAIN>-<NNN>-<slug>.md \
         directory the workflow engine can plan and execute. Writes to \
         tasks/PRD-<NAME>/."
    }

    fn aliases(&self) -> Vec<&str> {
        vec!["wf-prd-spec"]
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        let (template, source) =
            templates::resolve_template("workflow-prdtospec", &ctx.working_dir);
        if matches!(source, templates::TemplateSource::Missing) {
            return SkillOutput::Error(
                "workflow-prdtospec template not found (embedded fallback missing)".to_string(),
            );
        }
        let Some(prd_path) = args.first() else {
            return SkillOutput::Error(
                "Usage: /workflow-prd-spec <path/to/prds/PRD-<NAME>/PRD-<NAME>.md>".to_string(),
            );
        };
        let (task_dir, id_note) = match prd_id_from_path(prd_path) {
            Some(id) => (workflow_task_dir(&id), String::new()),
            None => (
                workflow_task_dir("<NAME>"),
                format!(
                    "\n\nNOTE: no PRD id could be read from `{prd_path}` — a workflow \
                     PRD filename is `PRD-<NAME>.md`. Read the PRD's title to \
                     recover the id, and if the document is not a workflow PRD \
                     (for example it is the skills chain's `prds/<slug>/PRD.md`), \
                     stop and say so rather than decomposing it into a layout \
                     nothing reads."
                ),
            ),
        };
        SkillOutput::Prompt(format!(
            "{template}\n\n---USER REQUEST---\n\n{}",
            user_block(prd_path, &task_dir, &id_note)
        ))
    }
}

fn user_block(prd_path: &str, task_dir: &str, id_note: &str) -> String {
    format!(
        "Use the workflow-prdtospec framework above to decompose the PRD at \
         `{prd_path}` into a workflow task directory.{id_note}\n\
         \n\
         OUTPUT LAYOUT:\n\
         1. Read the PRD with the Read tool before writing anything.\n\
         2. Write every task file FLAT into `{task_dir}/`, named \
            `TASK-<DOMAIN>-<NNN>-<slug>.md`. Discovery is a single \
            non-recursive read of that directory: a task file in a \
            subdirectory is not found at all.\n\
         3. `<DOMAIN>` is uppercase letters and digits with NO internal \
            hyphen, `<NNN>` is exactly three digits. Number in tens so a task \
            can be inserted later without renumbering.\n\
         4. The `task_id:` inside each file must equal the id read from its \
            filename. A mismatch is refused naming both.\n\
         \n\
         REQUIRED TASK YAML — the first fenced ```yaml block in the file, \
         immediately after the `# ` title. Not `---` front matter. These ten \
         keys must be PRESENT in every file, `[]` included, or the run is \
         refused naming the file and the missing keys: `task_id`, `title`, \
         `complexity`, `status`, `depends_on`, `blocks`, `implements`, \
         `required_env_keys`, `required_tools`, `deliverable_contracts`. Also \
         declare `prd`, `domain`, `workstream`, `source_sections`, and \
         `shared_append_target_files` where they apply.\n\
         \n\
         THE RULES THAT ACTUALLY FAIL:\n\
         1. `## Focused Tests` MUST list runnable commands, not descriptions. \
            A bullet counts only when it contains a backticked span whose \
            first token is one of: archon, bash, cargo, deno, go, gradle, \
            just, make, mvn, node, npm, pnpm, pytest, python, python3, sh, \
            tox, yarn. `cargo test -p archon-trading registry_migration` \
            proves something; \"Registry schema migration test.\" cannot. On \
            the reference corpus every entry was prose and the trace reported \
            0 of 93 requirements satisfied despite exact coverage both ways.\n\
         2. `## Files Expected to Change` MUST list real backtick-quoted \
            paths. Backticked spans are what gets lifted; a section with none \
            yields no anchors and the task's requirements cannot promote. Do \
            not write \"Likely anchors: ...\" hedges and do not repeat the \
            same list across tasks — identical lists make every task appear \
            to write every file.\n\
         3. `implements: [REQ-...]` is always declared, as a single-line flow \
            sequence. Use `implements: []` for an audit or review task — that \
            is a claim, and omitting the key is refused. Every cited ID must \
            exist in the PRD, and every PRD requirement must be claimed by at \
            least one task.\n\
         4. A templated `artifact_path` containing `<...>` needs an instance \
            binding: `instance_source_path`, `instance_source_records_field`, \
            `instance_artifact_field`, and a `min_instances` floor. \
            `min_instances: 0` is vacuous — zero matches satisfy it. A typed \
            verifier takes one concrete path and cannot be combined with a \
            template.\n\
         5. Use a distinct `kind` for create versus append on the same path — \
            `x_registry` creates, `x_registry_entry` appends.\n\
         6. Declare `shared_append_target_files` only for a file another task \
            writes concurrently. It asserts the write is coordinated and \
            atomic; it does not make it so, and the PRD must carry that as a \
            normative requirement separately.\n\
         7. `depends_on` and `blocks` are both parsed and reconciled into one \
            graph. Self-blocking, a pair declaring both directions, and \
            mutual blocking are each refused by name. An ordering-only \
            dependency — the upstream task produces no artifact this one \
            consumes — is legitimate and is reported as such. Do NOT fabricate \
            a deliverable contract to silence it.\n\
         \n\
         AFTER WRITING:\n\
         1. Run `archon workflow lint --tasks {task_dir}/`.\n\
         2. Run `archon requirements trace --prd {prd_path} --tasks \
            {task_dir}/`. This is the authoritative coverage check. The \
            lint's own `## requirement coverage` section looks for the PRD as \
            a SIBLING of the task directory and will report \
            `no PRD found beside {task_dir}/; skipped` because the PRD lives \
            under `prds/`. That is expected, not an error.\n\
         3. Print the paths created plus: task count, requirements claimed \
            against requirements the PRD defines, any unclaimed requirement, \
            any cited ID the PRD does not define, and every focused-test entry \
            that classified as prose.\n\
         4. Keep each tool-call payload under 8,000 characters — one task file \
            per Write call. Do NOT print full task bodies into the \
            conversation."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::Skill;

    fn ctx() -> SkillContext {
        SkillContext {
            session_id: "test".into(),
            working_dir: std::env::temp_dir(),
            model: "test".into(),
            agent_registry: None,
            session_store: None,
        }
    }

    fn prompt(args: &[String]) -> String {
        match WorkflowPrdSpecSkill.execute(args, &ctx()) {
            SkillOutput::Prompt(s) => s,
            other => panic!("expected Prompt, got {other:?}"),
        }
    }

    #[test]
    fn workflow_prd_spec_name_description_and_alias() {
        assert_eq!(WorkflowPrdSpecSkill.name(), "workflow-prd-spec");
        assert!(WorkflowPrdSpecSkill.aliases().contains(&"wf-prd-spec"));
        assert!(
            WorkflowPrdSpecSkill
                .description()
                .contains("tasks/PRD-<NAME>/")
        );
    }

    #[test]
    fn workflow_prd_spec_requires_a_path_arg() {
        match WorkflowPrdSpecSkill.execute(&[], &ctx()) {
            SkillOutput::Error(s) => assert!(s.contains("Usage:")),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn workflow_task_dir_is_under_the_tasks_root() {
        assert_eq!(
            workflow_task_dir("TRADING-DATA-LAKE-AHDM-001"),
            "tasks/PRD-TRADING-DATA-LAKE-AHDM-001"
        );
        assert!(workflow_task_dir("X-001").starts_with("tasks/"));
    }

    #[test]
    fn prd_id_is_read_from_a_workflow_prd_path() {
        assert_eq!(
            prd_id_from_path("prds/PRD-ALERT-002/PRD-ALERT-002.md").as_deref(),
            Some("ALERT-002")
        );
        assert_eq!(
            prd_id_from_path(r"F:\work\prds\PRD-X-001\PRD-X-001.md").as_deref(),
            Some("X-001")
        );
    }

    #[test]
    fn prd_id_rejects_the_skills_chain_layout() {
        assert_eq!(prd_id_from_path("prds/oauth-refresh/PRD.md"), None);
        assert_eq!(prd_id_from_path("docs/notes.md"), None);
    }

    #[test]
    fn workflow_prd_spec_emits_its_own_template_not_the_skills_chain_one() {
        let out = prompt(&["prds/PRD-X-001/PRD-X-001.md".to_string()]);
        assert!(out.starts_with(templates::WORKFLOW_PRD_TO_SPEC));
        assert!(!out.starts_with(templates::PRD_TO_SPEC));
    }

    #[test]
    fn workflow_prd_spec_targets_the_task_dir_derived_from_the_prd_id() {
        let out = prompt(&["prds/PRD-ALERT-002/PRD-ALERT-002.md".to_string()]);
        assert!(out.contains("tasks/PRD-ALERT-002/"));
        assert!(out.contains("TASK-<DOMAIN>-<NNN>-<slug>.md"));
        assert!(out.contains("non-recursive read"));
    }

    #[test]
    fn workflow_prd_spec_flags_a_path_with_no_recoverable_id() {
        let out = prompt(&["prds/oauth-refresh/PRD.md".to_string()]);
        assert!(out.contains("no PRD id could be read"));
        assert!(out.contains("tasks/PRD-<NAME>"));
    }

    #[test]
    fn workflow_prd_spec_lists_every_required_yaml_key() {
        let out = prompt(&["prds/PRD-X-001/PRD-X-001.md".to_string()]);
        for key in [
            "`task_id`",
            "`title`",
            "`complexity`",
            "`status`",
            "`depends_on`",
            "`blocks`",
            "`implements`",
            "`required_env_keys`",
            "`required_tools`",
            "`deliverable_contracts`",
        ] {
            assert!(out.contains(key), "missing required key {key} in prompt");
        }
        assert!(out.contains("source_sections"));
        assert!(out.contains("shared_append_target_files"));
    }

    #[test]
    fn workflow_prd_spec_demands_runnable_focused_tests() {
        let out = prompt(&["prds/PRD-X-001/PRD-X-001.md".to_string()]);
        assert!(out.contains("cargo test -p archon-trading registry_migration"));
        assert!(out.contains("Registry schema migration test."));
        assert!(out.contains("0 of 93 requirements satisfied"));
        assert!(out.contains("pytest"));
    }

    #[test]
    fn workflow_prd_spec_demands_real_paths_not_prose() {
        let out = prompt(&["prds/PRD-X-001/PRD-X-001.md".to_string()]);
        assert!(out.contains("backtick-quoted"));
        assert!(out.contains("Likely anchors"));
    }

    #[test]
    fn workflow_prd_spec_carries_the_contract_and_graph_rules() {
        let out = prompt(&["prds/PRD-X-001/PRD-X-001.md".to_string()]);
        assert!(out.contains("`min_instances: 0` is vacuous"));
        assert!(out.contains("cannot be combined with a template"));
        assert!(out.contains("x_registry_entry"));
        assert!(out.contains("ordering-only"));
        assert!(out.contains("Do NOT fabricate"));
        assert!(out.contains("implements: []"));
    }

    #[test]
    fn workflow_prd_spec_states_the_expected_lint_skip() {
        let out = prompt(&["prds/PRD-ALERT-002/PRD-ALERT-002.md".to_string()]);
        assert!(out.contains("archon requirements trace --prd"));
        assert!(out.contains("no PRD found beside tasks/PRD-ALERT-002/; skipped"));
        assert!(out.contains("That is expected, not an error."));
    }
}
