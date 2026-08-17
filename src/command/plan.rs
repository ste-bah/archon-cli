//! `/plan` slash-command handler — typed Plan Mode lifecycle and plan-file surface.
//!
//! The synchronous handler only reads its owned [`PlanSnapshot`] and emits an
//! owned [`CommandEffect`]. `apply_effect` records the shared lifecycle state
//! before it changes the lock-protected permission mode.

use archon_permissions::mode::PermissionMode;
use archon_session::plan::{PlanDocument, PlanStep, PlanStepStatus, PlanStore};
use archon_tui::app::TuiEvent;

use crate::command::plan_file;
use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};

#[derive(Clone, Debug)]
pub(crate) struct PlanSnapshot {
    pub(crate) current_mode: PermissionMode,
    pub(crate) active_plan_id: Option<String>,
}

/// `/plan` handler — displays the plan file and changes mode through effects.
pub(crate) struct PlanHandler;

impl PlanHandler {
    fn working_dir(ctx: &CommandContext) -> std::path::PathBuf {
        ctx.working_dir.clone().unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        })
    }

    fn active_plan_id(ctx: &CommandContext) -> Option<String> {
        ctx.plan_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.active_plan_id.clone())
    }

    fn resolve_plan_path(
        ctx: &CommandContext,
        plan_id: &str,
    ) -> std::io::Result<std::path::PathBuf> {
        plan_file::plan_document_path(&Self::working_dir(ctx), plan_id)
    }

    fn draft_plan_id() -> String {
        format!("plan-{}", uuid::Uuid::new_v4())
    }

    fn load_plan(ctx: &CommandContext, plan_id: &str) -> Result<Option<PlanDocument>, String> {
        let session_id = ctx
            .session_id
            .as_deref()
            .ok_or_else(|| "Cannot open a plan without a session ID.".to_string())?;
        let db = ctx
            .cozo_db
            .as_deref()
            .ok_or_else(|| "Cannot open a plan without the session database.".to_string())?;
        PlanStore::new(db)
            .map_err(|error| format!("Failed to open plan store: {error}"))?
            .load_plan(session_id, plan_id)
            .map_err(|error| format!("Failed to load plan {plan_id}: {error}"))
    }

    fn save_plan(ctx: &CommandContext, plan: &PlanDocument) -> Result<(), String> {
        let session_id = ctx
            .session_id
            .as_deref()
            .ok_or_else(|| "Cannot save a plan without a session ID.".to_string())?;
        let db = ctx
            .cozo_db
            .as_deref()
            .ok_or_else(|| "Cannot save a plan without the session database.".to_string())?;
        PlanStore::new(db)
            .map_err(|error| format!("Failed to open plan store: {error}"))?
            .save_plan(session_id, plan)
            .map_err(|error| format!("Failed to save plan {}: {error}", plan.id))
    }

    fn parse_edited_document(text: &str, prior: &PlanDocument) -> Result<PlanDocument, String> {
        enum Section {
            None,
            Steps,
            Risks,
            Questions,
        }

        let mut plan = prior.clone();
        let mut section = Section::None;
        let mut steps = Vec::new();
        let mut risks = Vec::new();
        let mut questions = Vec::new();
        let mut saw_title = false;
        let mut saw_steps = false;

        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(title) = trimmed.strip_prefix("# Plan:") {
                let title = title.trim();
                if title.is_empty() {
                    return Err("Plan title cannot be empty.".to_string());
                }
                plan.title = title.to_string();
                saw_title = true;
                continue;
            }
            match trimmed {
                "## Steps" => {
                    saw_steps = true;
                    section = Section::Steps;
                    continue;
                }
                "## Risks" => {
                    section = Section::Risks;
                    continue;
                }
                "## Questions" | "## Open Questions" => {
                    section = Section::Questions;
                    continue;
                }
                _ if trimmed.starts_with('#') => {
                    section = Section::None;
                    continue;
                }
                _ => {}
            }
            if trimmed.is_empty() {
                continue;
            }
            match section {
                Section::Steps => {
                    let Some((number, description)) = trimmed.split_once('.') else {
                        continue;
                    };
                    let number = number
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| format!("Invalid plan step: {trimmed}"))?;
                    let description = description.trim();
                    if description.is_empty() {
                        return Err(format!("Plan step {number} has no description."));
                    }
                    let fresh_step = PlanStep {
                        number,
                        description: description.to_string(),
                        affected_files: Vec::new(),
                        status: PlanStepStatus::Pending,
                        blocked_by: Vec::new(),
                        required_evidence: Vec::new(),
                        task_id: None,
                    };
                    let index = steps.len();
                    let step = prior.steps.get(index).filter(|previous| {
                        previous.number == fresh_step.number
                            && previous.description == fresh_step.description
                    });
                    steps.push(step.cloned().unwrap_or(fresh_step));
                }
                Section::Risks => risks.push(trimmed.trim_start_matches("- ").to_string()),
                Section::Questions => questions.push(trimmed.trim_start_matches("- ").to_string()),
                Section::None => {}
            }
        }

        if !saw_title || !saw_steps {
            return Err("Plan document must contain '# Plan: <title>' and '## Steps'.".to_string());
        }
        plan.steps = steps;
        plan.risks = risks;
        plan.questions = questions;
        plan.user_edited = true;
        Ok(plan)
    }

    fn enter_or_show(ctx: &mut CommandContext) -> anyhow::Result<()> {
        let current_mode = ctx
            .plan_snapshot
            .as_ref()
            .map(|snapshot| snapshot.current_mode)
            .unwrap_or_default();
        if current_mode == PermissionMode::Plan {
            ctx.emit(TuiEvent::TextDelta(
                "Already in plan mode — /plan off to exit, or tell the agent to proceed."
                    .to_string(),
            ));
            return Ok(());
        }

        let plan_body = match Self::active_plan_id(ctx) {
            Some(plan_id) => {
                let path = match Self::resolve_plan_path(ctx, &plan_id) {
                    Ok(path) => path,
                    Err(error) => {
                        ctx.emit(TuiEvent::Error(format!(
                            "Invalid active plan ID {plan_id:?}: {error}"
                        )));
                        return Ok(());
                    }
                };
                match plan_file::read_plan_document(&path) {
                    Ok(Some(content)) if !content.trim().is_empty() => {
                        format!("\nCurrent plan ({}):\n\n{}\n", path.display(), content)
                    }
                    Ok(_) => format!(
                        "\nNo editable document exists for active plan {plan_id} at {}.\n",
                        path.display()
                    ),
                    Err(error) => {
                        ctx.emit(TuiEvent::Error(format!(
                            "Failed to read plan document {}: {}",
                            path.display(),
                            error
                        )));
                        String::new()
                    }
                }
            }
            None => {
                "\nNo active plan yet. Use /plan open to create an editable draft.\n".to_string()
            }
        };
        ctx.emit(TuiEvent::TextDelta(format!(
            "{plan_body}\nPlan mode enabled. Use /plan off to exit.\n"
        )));
        ctx.pending_effect = Some(CommandEffect::EnterPlanMode {
            previous_mode: current_mode,
        });
        Ok(())
    }

    fn open_document(ctx: &mut CommandContext) -> anyhow::Result<()> {
        let plan_id = Self::active_plan_id(ctx).unwrap_or_else(Self::draft_plan_id);
        let path = match Self::resolve_plan_path(ctx, &plan_id) {
            Ok(path) => path,
            Err(error) => {
                ctx.emit(TuiEvent::Error(format!(
                    "Invalid plan ID {plan_id:?}: {error}"
                )));
                return Ok(());
            }
        };
        let prior = match Self::load_plan(ctx, &plan_id) {
            Ok(Some(plan)) => plan,
            Ok(None) => {
                let mut plan = PlanDocument::new(&plan_id, "Untitled Plan");
                plan.session_id = ctx.session_id.clone();
                plan
            }
            Err(error) => {
                ctx.emit(TuiEvent::Error(error));
                return Ok(());
            }
        };

        if !path.exists()
            && let Err(error) = plan_file::write_plan_document(&path, &prior)
        {
            ctx.emit(TuiEvent::Error(format!(
                "Failed to write plan document {}: {}",
                path.display(),
                error
            )));
            return Ok(());
        }

        if let Err(error) = plan_file::open_plan_in_editor(&path) {
            ctx.emit(TuiEvent::Error(format!(
                "Failed to open plan document {}: {}",
                path.display(),
                error
            )));
            return Ok(());
        }

        let edited = match plan_file::read_plan_document(&path) {
            Ok(Some(document)) => document,
            Ok(None) => {
                ctx.emit(TuiEvent::Error(format!(
                    "Plan document disappeared while editing: {}",
                    path.display()
                )));
                return Ok(());
            }
            Err(error) => {
                ctx.emit(TuiEvent::Error(format!(
                    "Failed to reread plan document {}: {}",
                    path.display(),
                    error
                )));
                return Ok(());
            }
        };
        let edited_plan = match Self::parse_edited_document(&edited, &prior) {
            Ok(plan) => plan,
            Err(error) => {
                ctx.emit(TuiEvent::Error(format!(
                    "Plan document was not saved: {error}"
                )));
                return Ok(());
            }
        };
        if let Err(error) = Self::save_plan(ctx, &edited_plan) {
            ctx.emit(TuiEvent::Error(error));
            return Ok(());
        }

        ctx.pending_effect = Some(CommandEffect::SetActivePlanId(plan_id));
        ctx.emit(TuiEvent::TextDelta(format!(
            "\nOpened and saved plan document: {}\n",
            path.display()
        )));
        Ok(())
    }

    fn exit_plan(ctx: &mut CommandContext) -> anyhow::Result<()> {
        ctx.emit(TuiEvent::TextDelta("Plan mode disabled.".to_string()));
        ctx.pending_effect = Some(CommandEffect::SetPermissionMode("default".to_string()));
        Ok(())
    }

    fn emit_valid_forms_error(ctx: &mut CommandContext) -> anyhow::Result<()> {
        ctx.emit(TuiEvent::Error(
            "Valid /plan forms: show, open, off, exit, done.".to_string(),
        ));
        Ok(())
    }
}

impl CommandHandler for PlanHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let arg = args.join(" ").trim().to_ascii_lowercase();
        match arg.as_str() {
            "" | "show" => Self::enter_or_show(ctx),
            "open" => Self::open_document(ctx),
            "off" | "exit" | "done" => Self::exit_plan(ctx),
            _ => Self::emit_valid_forms_error(ctx),
        }
    }

    fn description(&self) -> &str {
        "Enable Plan Mode (approve each tool call individually)"
    }
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
