// Orchestrated lifecycle (v3): one orchestrator decision loop per run.
// Each request reconstructs context from the authoritative task universe,
// deterministic orchestration ledger, and a bounded recent transcript tail.
// It is intentionally independent from main-agent compaction segments.

use super::*;

pub(super) use super::super::super::workflow_live_v3_orchestrator_actions as orchestrator_actions;
pub(super) use orchestrator_actions::{ActionOutcome, OrchestratorAction};

pub(super) const ORCHESTRATOR_TASK: &str = r#"You are the workflow orchestrator. You drive one decomposed task universe to completion using the authoritative task universe, deterministic ledger, and bounded recent outcomes supplied on each turn.

Rules:
- You DECIDE; the host ENFORCES. You cannot accept work the gates rejected, and dishonest acceptance attempts are refused with a typed reason.
- Work tasks in dependency order (each task lists dependency_ids).
- On a failed attempt, read the verbatim failure output in the outcome and give the NEXT attempt specific corrected instructions (exact commands, exact paths). Never re-issue an instruction that already failed unchanged.
- Test commands must match at least one test; zero-matched filters are never evidence.
- Artifact evidence must satisfy the declared deliverable contracts as they are checked TODAY; never edit an existing artifact instance to satisfy a check — produce a new one through the real pipeline.
- When a task is genuinely impossible (missing entitlement, unavailable provider, contradictory contract), block it honestly with the evidence. An honest block is success; fabrication is failure.
- Reply with EXACTLY ONE JSON envelope per turn: {"status":"accepted","summary":"<one line>","data":{"action":{...}}} where action is one of:
  {"action":"spawn_coder","task_id":"...","instructions":"...","target_files":["..."],"focused_tests":["..."]}
  {"action":"spawn_verifier","task_id":"...","instructions":"...","checks":["..."],"artifact_paths":["..."]}
  {"action":"spawn_explorer","question":"..."}
  {"action":"accept_task","task_id":"...","evidence_summary":"..."}
  {"action":"block_task","task_id":"...","reason":"..."}
  {"action":"final_report","narrative":"..."}"#;

pub(super) const MAX_CODER_ATTEMPTS_PER_TASK: usize = 4;
pub(super) const MAX_VERIFIER_ATTEMPTS_PER_TASK: usize = 4;
pub(super) const MAX_EXPLORER_CALLS: usize = 6;
pub(super) const TRANSCRIPT_TAIL: usize = 40;
pub(super) const OUTCOME_JSON_MAX_CHARS: usize = 6000;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub(super) struct OrchestratedTaskState {
    pub(super) coder_attempts: usize,
    pub(super) verifier_attempts: usize,
    pub(super) last_coder_status: Option<String>,
    pub(super) last_verifier_status: Option<String>,
    pub(super) status: OrchestratedTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) block_reason: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OrchestratedTaskStatus {
    #[default]
    Pending,
    Accepted,
    Blocked,
}

#[derive(Debug, Default)]
pub(super) struct OrchestrationLedger {
    pub(super) tasks: std::collections::BTreeMap<String, OrchestratedTaskState>,
    pub(super) explorer_calls: usize,
}

impl OrchestrationLedger {
    pub(super) fn for_universe(universe: &WorkflowV2TaskUniverse) -> Self {
        let mut ledger = Self::default();
        for task in &universe.tasks {
            ledger
                .tasks
                .entry(task.canonical_task_id.clone())
                .or_default();
        }
        ledger
    }

    pub(super) fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "tasks": self.tasks,
            "explorer_calls": self.explorer_calls,
        })
    }

    pub(super) fn accounting(&self) -> serde_json::Value {
        let mut accepted = Vec::new();
        let mut blocked = Vec::new();
        let mut pending = Vec::new();
        for (task_id, state) in &self.tasks {
            match state.status {
                OrchestratedTaskStatus::Accepted => accepted.push(task_id.clone()),
                OrchestratedTaskStatus::Blocked => blocked.push(serde_json::json!({
                    "task_id": task_id,
                    "reason": state.block_reason.clone().unwrap_or_default(),
                })),
                OrchestratedTaskStatus::Pending => pending.push(task_id.clone()),
            }
        }
        serde_json::json!({
            "accepted_task_ids": accepted,
            "blocked_tasks": blocked,
            "pending_task_ids": pending,
        })
    }
}

impl LifecycleDriver {
    pub(super) async fn run_orchestrated(&self) -> archon_workflow::WorkflowResult<()> {
        let mut ledger = OrchestrationLedger::for_universe(&self.universe);
        let mut transcript: Vec<serde_json::Value> = Vec::new();
        let budget = (self.universe.tasks.len() * 12).clamp(24, 240);
        for ordinal in 0..budget {
            let reply = self
                .orchestrator_reply(ordinal, &ledger, &transcript)
                .await?;
            let action = match orchestrator_actions::action_from_reply(&reply) {
                Ok(action) => action,
                Err(correction) => {
                    push_bounded_orchestrator_turn(
                        &mut transcript,
                        serde_json::json!({
                            "turn": ordinal,
                            "reply": bounded_json(&reply, OUTCOME_JSON_MAX_CHARS),
                            "outcome": { "status": "invalid_action", "correction": correction },
                        }),
                    );
                    continue;
                }
            };
            let finished = matches!(action, OrchestratorAction::FinalReport { .. });
            let outcome = self
                .dispatch_orchestrator_action(ordinal, &action, &mut ledger)
                .await?;
            push_bounded_orchestrator_turn(
                &mut transcript,
                serde_json::json!({
                    "turn": ordinal,
                    "decision": action,
                    "outcome": outcome_json(&outcome),
                }),
            );
            if finished {
                return Ok(());
            }
        }
        self.orchestrated_terminal_checkpoint(
            "orchestrated-budget-exhausted",
            "orchestrator action budget exhausted before final_report",
            &ledger,
            serde_json::Value::Null,
        )
        .await
    }

    pub(super) async fn orchestrator_reply(
        &self,
        ordinal: usize,
        ledger: &OrchestrationLedger,
        transcript: &[serde_json::Value],
    ) -> archon_workflow::WorkflowResult<serde_json::Value> {
        self.call(
            "reduce",
            &format!("orchestrator-turn-{ordinal}"),
            Some(serde_json::json!([
                { "mission": "Drive every task in the universe to honest completion or an honest block.",
                  "task_universe": self.task_universe },
                { "ledger": ledger.summary() },
                { "conversation": transcript },
            ])),
            serde_json::json!({ "tier": "reducer", "task": ORCHESTRATOR_TASK }),
        )
        .await
    }

    pub(super) async fn dispatch_orchestrator_action(
        &self,
        ordinal: usize,
        action: &OrchestratorAction,
        ledger: &mut OrchestrationLedger,
    ) -> archon_workflow::WorkflowResult<ActionOutcome> {
        match action {
            OrchestratorAction::SpawnCoder {
                task_id,
                instructions,
                target_files,
                focused_tests,
            } => {
                let state = ledger.tasks.entry(task_id.clone()).or_default();
                if state.coder_attempts >= MAX_CODER_ATTEMPTS_PER_TASK {
                    return Ok(refusal(
                        ordinal,
                        "spawn_coder",
                        "coder attempt budget for this task is exhausted; block the task honestly or accept it on existing evidence",
                    ));
                }
                state.coder_attempts += 1;
                let item = serde_json::json!({
                    "item_id": format!("orch-{ordinal}-code"),
                    "canonical_task_ids": [task_id],
                    "task": instructions,
                    "instructions": instructions,
                    "target_files": target_files,
                    "focused_verification": focused_tests,
                    "work_type": "implementation",
                });
                let result = self
                    .write_fanout(
                        &format!("orchestrator-{ordinal}-coder"),
                        serde_json::json!([item]),
                        instructions,
                    )
                    .await;
                Ok(self.wave_outcome(ordinal, "spawn_coder", result, |status| {
                    ledger
                        .tasks
                        .entry(task_id.clone())
                        .or_default()
                        .last_coder_status = Some(status);
                }))
            }
            OrchestratorAction::SpawnVerifier {
                task_id,
                instructions,
                checks,
                artifact_paths,
            } => {
                let state = ledger.tasks.entry(task_id.clone()).or_default();
                if state.verifier_attempts >= MAX_VERIFIER_ATTEMPTS_PER_TASK {
                    return Ok(refusal(
                        ordinal,
                        "spawn_verifier",
                        "verifier attempt budget for this task is exhausted",
                    ));
                }
                state.verifier_attempts += 1;
                let item = serde_json::json!({
                    "item_id": format!("orch-{ordinal}-verify"),
                    "canonical_task_ids": [task_id],
                    "focused_verification": checks,
                    "instructions": instructions,
                    "artifact_requirements": artifact_paths,
                });
                let items = workflow_live_v2_lifecycle_verify_options::prepare_verification_items(
                    vec![item],
                    self.project_artifact_root.as_deref(),
                    &[],
                    &self.task_universe,
                );
                let options = workflow_live_v2_lifecycle_verify_options::verification_options(
                    &items,
                    instructions,
                    true,
                );
                let result = self
                    .parallel(
                        &format!("orchestrator-{ordinal}-verifier"),
                        serde_json::json!(items),
                        options,
                    )
                    .await;
                Ok(
                    self.wave_outcome(ordinal, "spawn_verifier", result, |status| {
                        ledger
                            .tasks
                            .entry(task_id.clone())
                            .or_default()
                            .last_verifier_status = Some(status);
                    }),
                )
            }
            OrchestratorAction::SpawnExplorer { question } => {
                if ledger.explorer_calls >= MAX_EXPLORER_CALLS {
                    return Ok(refusal(
                        ordinal,
                        "spawn_explorer",
                        "explorer budget exhausted",
                    ));
                }
                ledger.explorer_calls += 1;
                let result = self
                    .parallel(
                        &format!("orchestrator-{ordinal}-explorer"),
                        serde_json::json!([{ "id": format!("orch-{ordinal}-explore"), "question": question }]),
                        serde_json::json!({ "tier": "analysis", "task": question }),
                    )
                    .await;
                Ok(self.wave_outcome(ordinal, "spawn_explorer", result, |_| {}))
            }
            OrchestratorAction::AcceptTask {
                task_id,
                evidence_summary,
            } => {
                let state = ledger.tasks.entry(task_id.clone()).or_default();
                let coder_ok = matches!(
                    state.last_coder_status.as_deref(),
                    Some("accepted" | "noop")
                );
                let verifier_ok = state.last_verifier_status.as_deref() == Some("accepted");
                if !coder_ok || !verifier_ok {
                    return Ok(refusal(
                        ordinal,
                        "accept_task",
                        &format!(
                            "host refuses acceptance: latest coder status {:?}, latest verifier status {:?}; both must be gate-accepted first",
                            state.last_coder_status, state.last_verifier_status
                        ),
                    ));
                }
                state.status = OrchestratedTaskStatus::Accepted;
                Ok(ActionOutcome {
                    action_ordinal: ordinal,
                    tool: "accept_task".to_string(),
                    status: "ok".to_string(),
                    report: serde_json::json!({
                        "task_id": task_id,
                        "accepted": true,
                        "evidence_summary": evidence_summary,
                    }),
                })
            }
            OrchestratorAction::BlockTask { task_id, reason } => {
                let state = ledger.tasks.entry(task_id.clone()).or_default();
                state.status = OrchestratedTaskStatus::Blocked;
                state.block_reason = Some(reason.clone());
                Ok(ActionOutcome {
                    action_ordinal: ordinal,
                    tool: "block_task".to_string(),
                    status: "ok".to_string(),
                    report: serde_json::json!({ "task_id": task_id, "blocked": true }),
                })
            }
            OrchestratorAction::FinalReport { narrative } => {
                self.orchestrated_terminal_checkpoint(
                    "orchestrated-final-report",
                    "orchestrated run final report",
                    ledger,
                    serde_json::json!(narrative),
                )
                .await?;
                Ok(ActionOutcome {
                    action_ordinal: ordinal,
                    tool: "final_report".to_string(),
                    status: "ok".to_string(),
                    report: ledger.accounting(),
                })
            }
        }
    }

    /// Convert a wave result (or error) into a bounded, verbatim outcome for
    /// the conversation, recording the first branch status via `record`.
    pub(super) fn wave_outcome(
        &self,
        ordinal: usize,
        tool: &str,
        result: archon_workflow::WorkflowResult<serde_json::Value>,
        record: impl FnOnce(String),
    ) -> ActionOutcome {
        match result {
            Ok(value) => {
                let outcomes = support::outcomes_of(&value);
                let status = outcomes
                    .first()
                    .and_then(|outcome| outcome.get("status").and_then(serde_json::Value::as_str))
                    .or_else(|| value.get("status").and_then(serde_json::Value::as_str))
                    .unwrap_or("unknown")
                    .to_string();
                record(status.clone());
                ActionOutcome {
                    action_ordinal: ordinal,
                    tool: tool.to_string(),
                    status: if status == "accepted" || status == "noop" {
                        "ok".to_string()
                    } else {
                        "gate_rejected".to_string()
                    },
                    report: bounded_json(&value, OUTCOME_JSON_MAX_CHARS),
                }
            }
            Err(error) => {
                record("error".to_string());
                ActionOutcome {
                    action_ordinal: ordinal,
                    tool: tool.to_string(),
                    status: "error".to_string(),
                    report: serde_json::json!({ "error": error.to_string() }),
                }
            }
        }
    }

    pub(super) async fn orchestrated_terminal_checkpoint(
        &self,
        id: &str,
        summary: &str,
        ledger: &OrchestrationLedger,
        narrative: serde_json::Value,
    ) -> archon_workflow::WorkflowResult<()> {
        self.call(
            "checkpoint",
            id,
            Some(serde_json::json!({
                "summary": summary,
                "accounting": ledger.accounting(),
                "narrative": narrative,
            })),
            serde_json::json!({ "task": "Record the orchestrated-lifecycle terminal accounting." }),
        )
        .await
        .map(|_| ())
    }
}

pub(super) fn refusal(ordinal: usize, tool: &str, reason: &str) -> ActionOutcome {
    ActionOutcome {
        action_ordinal: ordinal,
        tool: tool.to_string(),
        status: "refused".to_string(),
        report: serde_json::json!({ "reason": reason }),
    }
}

pub(super) fn outcome_json(outcome: &ActionOutcome) -> serde_json::Value {
    serde_json::json!({
        "tool": outcome.tool,
        "status": outcome.status,
        "report": outcome.report,
    })
}

/// Verbatim-but-bounded: serialize and truncate at a character budget without
/// reshaping any field the model needs to read.
pub(super) fn push_bounded_orchestrator_turn(
    transcript: &mut Vec<serde_json::Value>,
    turn: serde_json::Value,
) {
    transcript.push(turn);
    if transcript.len() > TRANSCRIPT_TAIL {
        let drop = transcript.len() - TRANSCRIPT_TAIL;
        transcript.drain(..drop);
    }
}

pub(super) fn bounded_json(value: &serde_json::Value, max_chars: usize) -> serde_json::Value {
    let text = value.to_string();
    if text.chars().count() <= max_chars {
        return value.clone();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    serde_json::json!({
        "truncated": true,
        "prefix": truncated,
    })
}
