//! `/workflow-prd` — write a PRD in the layout the workflow engine consumes.
//!
//! This is the first half of the *workflow* PRD pipeline, which is deliberately
//! separate from the skills chain (`/to-prd` → `/prd-to-spec` → `/spec-to-tasks`
//! → `/archon-code`). Both write under the same two roots — `prds/` and
//! `tasks/` — in different subfolders, so the two pipelines coexist without
//! colliding:
//!
//! | | workflow path | skills chain |
//! |---|---|---|
//! | PRD | `prds/PRD-<NAME>/PRD-<NAME>.md` | `prds/<slug>/PRD.md` |
//! | tasks | `tasks/PRD-<NAME>/TASK-<DOMAIN>-<NNN>-<slug>.md` | `tasks/phase<N>/task<M>.md` |
//!
//! The workflow layout exists because its task files are found by *walking a
//! directory* rather than by being referenced, and because the requirement IDs
//! in the PRD are extracted by regex. Both make the document's shape part of
//! the contract rather than presentation — which is what `workflow-prd.md`
//! teaches and what this skill points the model at.

use super::{Skill, SkillContext, SkillOutput, templates};

/// Root under which every generated PRD lives, for both pipelines.
pub const PRD_ROOT: &str = "prds";

/// Directory a workflow PRD is written into, relative to the working
/// directory. `{name}` is the PRD id — SCREAMING-KEBAB-CASE, no `PRD-` prefix
/// duplication.
pub fn workflow_prd_path(name: &str) -> String {
    format!("{PRD_ROOT}/PRD-{name}/PRD-{name}.md")
}

pub struct WorkflowPrdSkill;

impl Skill for WorkflowPrdSkill {
    fn name(&self) -> &str {
        "workflow-prd"
    }

    fn description(&self) -> &str {
        "Write a PRD in the workflow-engine format — numbered sections, \
         regex-extractable REQ IDs, per-requirement severity. Writes to \
         prds/PRD-<NAME>/PRD-<NAME>.md."
    }

    fn aliases(&self) -> Vec<&str> {
        vec!["wf-prd"]
    }

    fn execute(&self, args: &[String], ctx: &SkillContext) -> SkillOutput {
        let (template, source) = templates::resolve_template("workflow-prd", &ctx.working_dir);
        if matches!(source, templates::TemplateSource::Missing) {
            return SkillOutput::Error(
                "workflow-prd template not found (embedded fallback missing)".to_string(),
            );
        }
        let extra = if args.is_empty() {
            String::new()
        } else {
            format!("\n\nAdditional input from the user: {}", args.join(" "))
        };
        let path_instruction = match explicit_workflow_prd_path(args) {
            Some(path) => format!(
                "Use this exact output path: `{path}`. Do not choose a \
                 different PRD id or path."
            ),
            None => format!(
                "Choose a PRD id in SCREAMING-KEBAB-CASE ending in a \
                 three-digit sequence, derived from the product area — for \
                 example `TRADING-DATA-LAKE-AHDM-001`. Write the PRD to \
                 `{}`. The same id names the task directory \
                 `tasks/PRD-<NAME>/` that `/workflow-prd-spec` will create, \
                 so pick it once and reuse it verbatim.",
                workflow_prd_path("<NAME>")
            ),
        };
        SkillOutput::Prompt(format!(
            "{template}\n\n---USER REQUEST---\n\n{}",
            user_block(&path_instruction, &extra)
        ))
    }
}

fn user_block(path_instruction: &str, extra: &str) -> String {
    format!(
        "Use the workflow-prd framework above to write a PRD for the Archon \
         workflow engine.\n\
         \n\
         Source material: always use the current conversation context as \
             background.{extra}\n\
         \n\
         SOURCE GROUNDING CONTRACT:\n\
         1. Classify the request as `source-grounded` or `conversation-only`.\n\
         2. Treat it as `source-grounded` if the user names or implies \
            required sources: file paths, HLDs, PDFs, research packs, \
            ingested docs, URLs, or exact document titles.\n\
         3. For `source-grounded` requests, do NOT write the PRD until every \
            required source has been read or searched with the available \
            tools. If one cannot be accessed, stop and report the missing \
            source instead of drafting from memory.\n\
         4. For `source-grounded` requests, include a `## Source Coverage` \
            section listing each source, whether it was found, and any gaps.\n\
         5. For `conversation-only` requests, label the basis as \
            `Sources used: conversation only`, include assumptions, include \
            open questions, and do not claim validated architecture, \
            algorithms, or compliance facts the user did not supply.\n\
         \n\
         WORKFLOW-FORMAT REQUIREMENTS — each of these is checked mechanically \
         later, so a violation is a later failure, not a style note:\n\
         1. Number every section (`## 6. Coverage Matrix`, `### 6.1 ...`). \
            Task files cite these numbers in `source_sections:`.\n\
         2. Write every requirement as its own line starting at column 0 with \
            `- REQ-<AREA>-<NNN>: `, where `<AREA>` is uppercase LETTERS only \
            and `<NNN>` is exactly three digits. One per line, the ID first on \
            the line, never inside a fenced code block. An ID buried \
            mid-paragraph is never extracted and silently fails to exist.\n\
         3. Group the requirement bullets under the numbered section they \
            belong to.\n\
         4. End every requirement with a severity clause. Use \
            ``Violation severity: `error` — <what fails closed>.`` when a \
            violation must fail the run; the backticked `error` and the \
            phrase `fails closed` are what the classifier recognises. Use \
            ``Violation severity: `warn` — <what is reported>.`` otherwise, \
            and note that `warn` is currently recorded as unclassified rather \
            than being promoted to error.\n\
         5. Include a `## Hard Rules` section. Every non-blank line in it is \
            harvested verbatim into every agent prompt for the run, so put \
            only rules there — no commentary.\n\
         6. Include a numbered decomposition section listing the intended \
            tasks in dependency order with the requirement IDs each will \
            claim, and a traceability table of requirement ID to task.\n\
         7. Every `TASK-<DOMAIN>-<NNN>` id you name in the PRD must be one the \
            decomposition will actually create. A generated run harvests task \
            ids from the PRD and refuses to plan when one has no matching \
            `TASK-*.md` file.\n\
         8. Where the PRD specifies an artifact family with a templated path \
            (`data/<dataset-id>/summary.json`), state which collection \
            enumerates the instances and the minimum count. A minimum of zero \
            is a gate that zero files satisfy.\n\
         9. Where two tasks must append to one file, carry the \
            single-writer/atomicity rule as its own normative requirement. \
            The task-side `shared_append_target_files:` declaration removes \
            the scheduler's serialisation; it does not implement locking.\n\
         10. No \"should work\", \"probably\", \"later\", \"TBD\", or \
            \"best effort\". State what fails, and which way it fails.\n\
         \n\
         OUTPUT REQUIREMENTS:\n\
         1. {path_instruction}\n\
         2. Write the PRD to disk with complete, valid tool JSON. The Write \
            tool input MUST be a JSON object with string fields named exactly \
            `file_path` and `content`.\n\
         3. For long PRDs, do NOT put the whole document in one Write call. \
            Keep each tool-call payload below 8,000 characters: create the \
            parent directory, Write the title and first sections, then append \
            later sections with separate Bash heredoc chunks.\n\
         4. Create parent directories as needed before appending chunks.\n\
         5. After writing, print the path you wrote to and tell the user the \
            next command: `/workflow-prd-spec <that path>`.\n\
         6. Do NOT print the PRD body into the conversation."
    )
}

/// An explicit `prds/.../PRD-*.md` path named in the user's arguments.
fn explicit_workflow_prd_path(args: &[String]) -> Option<String> {
    args.iter().find_map(|arg| {
        let start = arg.find(&format!("{PRD_ROOT}/"))?;
        let cleaned = arg[start..].trim_matches(|c: char| {
            matches!(c, '`' | '"' | '\'' | ',' | '.' | ';' | ':' | '(' | ')')
        });
        (cleaned.ends_with(".md") && cleaned.contains("/PRD-")).then(|| cleaned.to_string())
    })
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
        match WorkflowPrdSkill.execute(args, &ctx()) {
            SkillOutput::Prompt(s) => s,
            other => panic!("expected Prompt, got {other:?}"),
        }
    }

    #[test]
    fn workflow_prd_name_description_and_alias() {
        assert_eq!(WorkflowPrdSkill.name(), "workflow-prd");
        assert!(WorkflowPrdSkill.aliases().contains(&"wf-prd"));
        assert!(
            WorkflowPrdSkill
                .description()
                .contains("prds/PRD-<NAME>/PRD-<NAME>.md")
        );
    }

    #[test]
    fn workflow_prd_path_is_under_the_prds_root() {
        assert_eq!(
            workflow_prd_path("TRADING-DATA-LAKE-AHDM-001"),
            "prds/PRD-TRADING-DATA-LAKE-AHDM-001/PRD-TRADING-DATA-LAKE-AHDM-001.md"
        );
        assert!(workflow_prd_path("X-001").starts_with("prds/"));
    }

    #[test]
    fn workflow_prd_emits_its_own_template_not_the_skills_chain_one() {
        let out = prompt(&[]);
        assert!(out.starts_with(templates::WORKFLOW_PRD));
        assert!(!out.starts_with(templates::AI_AGENT_PRD));
        assert!(out.contains("---USER REQUEST---"));
    }

    #[test]
    fn workflow_prd_defaults_to_the_two_root_layout() {
        let out = prompt(&[]);
        assert!(out.contains("prds/PRD-<NAME>/PRD-<NAME>.md"));
        assert!(out.contains("tasks/PRD-<NAME>/"));
    }

    #[test]
    fn workflow_prd_requires_numbered_sections_and_bullet_requirement_ids() {
        let out = prompt(&[]);
        assert!(out.contains("Number every section"));
        assert!(out.contains("source_sections:"));
        assert!(out.contains("- REQ-<AREA>-<NNN>: "));
        assert!(out.contains("uppercase LETTERS only"));
        assert!(out.contains("never inside a fenced code block"));
    }

    #[test]
    fn workflow_prd_requires_per_requirement_severity() {
        let out = prompt(&[]);
        assert!(out.contains("Violation severity: `error`"));
        assert!(out.contains("Violation severity: `warn`"));
        assert!(out.contains("fails closed"));
    }

    #[test]
    fn workflow_prd_carries_the_workflow_only_rules() {
        let out = prompt(&[]);
        assert!(out.contains("## Hard Rules"));
        assert!(out.contains("shared_append_target_files"));
        assert!(out.contains("minimum of zero"));
        assert!(out.contains("no matching `TASK-*.md` file"));
    }

    #[test]
    fn workflow_prd_forbids_hedging_without_a_failure_mode() {
        let out = prompt(&[]);
        assert!(out.contains("best effort"));
        assert!(out.contains("which way it fails"));
    }

    #[test]
    fn workflow_prd_with_args_includes_extra_block() {
        let args: Vec<String> = vec!["focus".into(), "on".into(), "coverage".into()];
        assert!(prompt(&args).contains("focus on coverage"));
    }

    #[test]
    fn workflow_prd_no_args_omits_extra_block() {
        assert!(!prompt(&[]).contains("Additional input from the user"));
    }

    #[test]
    fn workflow_prd_with_explicit_path_pins_it() {
        let args: Vec<String> =
            vec!["Write it to prds/PRD-ALERT-002/PRD-ALERT-002.md.".to_string()];
        let out = prompt(&args);
        assert!(out.contains("Use this exact output path: `prds/PRD-ALERT-002/PRD-ALERT-002.md`"));
    }

    #[test]
    fn explicit_workflow_prd_path_ignores_non_prd_paths() {
        assert_eq!(
            explicit_workflow_prd_path(&["docs/example.md".to_string()]),
            None
        );
        assert_eq!(
            explicit_workflow_prd_path(&["prds/oauth/PRD.md".to_string()]),
            None
        );
    }
}
