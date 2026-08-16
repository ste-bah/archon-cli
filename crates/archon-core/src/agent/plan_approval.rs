use archon_tools::plan_tasks::{
    build_plan_task_infos, persisted_records, reject_plan_task_collisions,
};
use archon_tools::task_manager::TASK_MANAGER;

use archon_permissions::mode::PermissionMode;
use archon_session::plan::{
    PlanApproval, PlanApprovalDecision, PlanApprovalRecord, PlanApprovalSource, PlanDocument,
    PlanStatus,
};

use super::{
    Agent, AgentEvent, AgentMode, AskUserPromptKind, ToolResult, parse_plan_from_text,
    plan_mode_state,
};

/// Policy used when an agent has no interactive approval channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonInteractivePlanApproval {
    Approve,
    Reject,
}

impl NonInteractivePlanApproval {
    fn from_config(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("reject") => Self::Reject,
            _ => Self::Approve,
        }
    }
}

/// Render the durable plan, rather than assistant text, before accepting a decision.
pub fn render_plan_approval(plan: &PlanDocument) -> String {
    let mut rendered = format!("Plan approval: {}\n\nSteps:\n", plan.title);
    let mut steps = plan.steps.iter().collect::<Vec<_>>();
    steps.sort_by_key(|step| step.number);
    for step in steps {
        rendered.push_str(&format!("{}. {}\n", step.number, step.description));
    }
    rendered.push_str("\nChoose one: approve | approve-edits | edit | reject: <reason>");
    rendered
}

/// Parse only the explicit approval protocol tokens.
pub fn parse_plan_approval_response(input: &str) -> Result<PlanApprovalDecision, String> {
    let input = input.trim();
    match input {
        "approve" => Ok(PlanApprovalDecision::Approve),
        "approve-edits" => Ok(PlanApprovalDecision::ApproveAcceptEdits),
        "edit" => Ok(PlanApprovalDecision::Edit),
        _ => {
            let Some(reason) = input.strip_prefix("reject:") else {
                return Err("Enter approve, approve-edits, edit, or reject: <reason>.".into());
            };
            let reason = reason.trim();
            if reason.is_empty() {
                return Err("A rejection reason is required.".into());
            }
            Ok(PlanApprovalDecision::Reject {
                reason: reason.to_string(),
            })
        }
    }
}

pub fn noninteractive_decision(value: Option<&str>) -> PlanApprovalDecision {
    match NonInteractivePlanApproval::from_config(value) {
        NonInteractivePlanApproval::Approve => PlanApprovalDecision::Approve,
        NonInteractivePlanApproval::Reject => PlanApprovalDecision::Reject {
            reason: "noninteractive plan approval rejected by policy".into(),
        },
    }
}

impl Agent {
    pub(super) async fn handle_exit_plan_mode_approval(
        &mut self,
        result: ToolResult,
    ) -> ToolResult {
        let Some(mut plan) = self.persist_draft_before_approval() else {
            return ToolResult::error(
                "Plan approval requires a persisted structured draft before exiting Plan Mode.",
            );
        };

        let source = if self.ask_user_response_rx.is_some() {
            PlanApprovalSource::Interactive
        } else {
            PlanApprovalSource::NonInteractive
        };
        let decision = self.request_plan_approval(&plan).await;
        let approval = PlanApproval {
            user_edited: matches!(
                decision,
                PlanApprovalDecision::ApproveAcceptEdits | PlanApprovalDecision::Edit
            ),
            decision: decision.clone(),
            source,
            decided_at: chrono::Utc::now().to_rfc3339(),
        };
        plan.approval = Some(approval.clone());
        plan.status = match decision {
            PlanApprovalDecision::Approve | PlanApprovalDecision::ApproveAcceptEdits => {
                PlanStatus::Approved
            }
            PlanApprovalDecision::Reject { .. } => PlanStatus::Abandoned,
            PlanApprovalDecision::Edit => PlanStatus::Draft,
        };

        let approved = matches!(
            decision,
            PlanApprovalDecision::Approve | PlanApprovalDecision::ApproveAcceptEdits
        );
        let task_infos = if approved {
            match build_plan_task_infos(&self.config.session_id, &mut plan) {
                Ok(infos) => Some(infos),
                Err(error) => {
                    return ToolResult::error(format!(
                        "Failed to materialize approved plan tasks: {error}"
                    ));
                }
            }
        } else {
            None
        };

        {
            let prepared_installation = if let Some(task_infos) = task_infos.clone() {
                let Some(store) = self.plan_store.as_ref() else {
                    return ToolResult::error(
                        "Failed to prepare approved plan tasks: plan store is not configured",
                    );
                };
                if let Err(error) = reject_plan_task_collisions(
                    &TASK_MANAGER,
                    store,
                    &self.config.session_id,
                    &task_infos,
                ) {
                    return ToolResult::error(format!(
                        "Failed to prepare approved plan tasks: {error}"
                    ));
                }
                let Some(authority) = self.plan_approval_authority.as_ref() else {
                    return ToolResult::error(
                        "Failed to prepare approved plan tasks: plan approval authority is not configured",
                    );
                };
                match TASK_MANAGER.prepare_plan_task_installation(
                    authority,
                    &self.config.session_id,
                    store.clone(),
                    task_infos,
                ) {
                    Ok(prepared) => Some(prepared),
                    Err(error) => {
                        return ToolResult::error(format!(
                            "Failed to prepare approved plan tasks: {error}"
                        ));
                    }
                }
            } else {
                None
            };

            if let Err(error) = self.persist_approval(&plan, approval, task_infos.as_deref()) {
                return ToolResult::error(format!("Failed to persist plan approval: {error}"));
            }
            if let Some(prepared) = prepared_installation {
                prepared.install();
            }
        }

        match decision {
            PlanApprovalDecision::Approve => {
                self.restore_mode_after_approved_plan(None).await;
                result
            }
            PlanApprovalDecision::ApproveAcceptEdits => {
                self.restore_mode_after_approved_plan(Some(PermissionMode::AcceptEdits))
                    .await;
                result
            }
            PlanApprovalDecision::Reject { reason } => {
                ToolResult::error(format!("Plan rejected: {reason}"))
            }
            PlanApprovalDecision::Edit => {
                ToolResult::error("Plan requires edits before execution.")
            }
        }
    }

    /// Request a decision through the existing AskUser channel, or apply the
    /// explicit noninteractive policy when no interactive frontend exists.
    pub async fn request_plan_approval(&mut self, plan: &PlanDocument) -> PlanApprovalDecision {
        if self.ask_user_response_rx.is_none() {
            return noninteractive_decision(Some(
                &self.config.context.noninteractive_plan_approval,
            ));
        }
        let prompt = render_plan_approval(plan);
        self.send_event(AgentEvent::AskUser {
            question: prompt.clone(),
            kind: AskUserPromptKind::PlanApproval,
        })
        .await;
        loop {
            let response = match self.ask_user_response_rx.as_ref() {
                Some(rx) => rx.lock().await.recv().await,
                None => {
                    return noninteractive_decision(Some(
                        &self.config.context.noninteractive_plan_approval,
                    ));
                }
            };
            let Some(response) = response else {
                return PlanApprovalDecision::Reject {
                    reason: "User input channel closed unexpectedly.".into(),
                };
            };
            match parse_plan_approval_response(&response) {
                Ok(decision) => return decision,
                Err(reason) => {
                    self.send_event(AgentEvent::AskUser {
                        question: format!("{reason}\n\n{prompt}"),
                        kind: AskUserPromptKind::PlanApproval,
                    })
                    .await;
                }
            }
        }
    }

    fn persist_draft_before_approval(&self) -> Option<PlanDocument> {
        let plan_store = self.plan_store.as_ref()?;
        let plan_text = self.latest_assistant_text();
        if plan_text.is_empty() {
            return None;
        }
        let mut draft = parse_plan_from_text(&plan_text);
        draft.status = PlanStatus::Draft;
        let session_id = &self.config.session_id;
        if let Err(error) = plan_store.save_plan(session_id, &draft) {
            tracing::warn!("failed to save plan draft before approval: {error}");
            return None;
        }
        match plan_store.load_plan(session_id, &draft.id) {
            Ok(Some(plan)) => Some(plan),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!("failed to reload plan draft before approval: {error}");
                None
            }
        }
    }

    fn persist_approval(
        &self,
        plan: &PlanDocument,
        approval: PlanApproval,
        task_infos: Option<&[archon_tools::task_manager::TaskInfo]>,
    ) -> Result<(), std::io::Error> {
        let plan_store = self
            .plan_store
            .as_ref()
            .ok_or_else(|| std::io::Error::other("plan store is not configured"))?;
        let authority = self
            .plan_approval_authority
            .as_ref()
            .ok_or_else(|| std::io::Error::other("plan approval authority is not configured"))?;
        let record = PlanApprovalRecord {
            plan_id: plan.id.clone(),
            session_id: self.config.session_id.clone(),
            approval,
        };
        if let Some(task_infos) = task_infos {
            let task_records = persisted_records(task_infos).map_err(std::io::Error::other)?;
            plan_store.save_terminal_plan_with_approval_and_tasks(
                authority,
                &self.config.session_id,
                plan,
                &record,
                &task_records,
            )
        } else {
            plan_store.save_terminal_plan_with_approval(
                authority,
                &self.config.session_id,
                plan,
                &record,
            )
        }
    }

    async fn restore_mode_after_approved_plan(&mut self, override_mode: Option<PermissionMode>) {
        let mut state = self.plan_mode_state.lock().await;
        let restored = override_mode.unwrap_or_else(|| {
            plan_mode_state::safe_restore_mode(state.previous_permission_mode.take(), false)
        });
        if override_mode.is_some() {
            state.previous_permission_mode.take();
        }
        state.active_plan_id = None;
        state.entered_via = None;
        drop(state);
        *self.config.permission_mode.lock().await = restored.to_string();
        self.state.mode = AgentMode::Normal;
    }
}
