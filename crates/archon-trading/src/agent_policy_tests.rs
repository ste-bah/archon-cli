use super::*;

fn context(persona: Persona) -> AgentContext {
    AgentContext {
        persona,
        requested_level: AutonomyLevel::L4ExecuteWithApproval,
        phase: TradingPhase::P5LivePilot,
        model_class: ModelClass::Reasoning,
        maker_checker_approved: true,
        certified_mcp_write: true,
    }
}

#[test]
fn t_agent_01_per05_phase_ceiling_is_enforced() {
    let mut ctx = context(Persona::Per05ExecutionAgent);
    ctx.phase = TradingPhase::P4Paper;
    let decision = evaluate_access(&ctx, ToolAction::SubmitPaperOrder);
    assert!(!decision.allowed);
    assert_eq!(decision.effective_level, AutonomyLevel::L1ReadOnly);
}

#[test]
fn t_agent_02_fail_closed_denies_ungranted_writes() {
    let ctx = context(Persona::Per02ResearchAnalyst);
    let decision = evaluate_access(&ctx, ToolAction::WriteKb);
    assert!(!decision.allowed);
    assert_eq!(decision.result().unwrap_err().code(), "ERR-POLICY-DENIED");
}

#[test]
fn t_agent_03_per05_fence_and_model_gate_are_normative() {
    let ctx = context(Persona::Per05ExecutionAgent);
    assert!(!evaluate_access(&ctx, ToolAction::WriteKb).allowed);
    assert!(!evaluate_access(&ctx, ToolAction::WriteRiskPolicy).allowed);
    let mut risk = context(Persona::Per04RiskOfficer);
    risk.model_class = ModelClass::Code;
    let decision = evaluate_access(&risk, ToolAction::PromoteStrategy);
    assert!(!decision.allowed);
    assert_eq!(decision.reason, "model class not permitted for persona");
}

#[test]
fn write_and_broker_actions_need_maker_checker() {
    let mut ctx = context(Persona::Per04RiskOfficer);
    ctx.maker_checker_approved = false;
    for action in [
        ToolAction::WriteRiskPolicy,
        ToolAction::PromoteStrategy,
        ToolAction::ChangeLiveLimit,
        ToolAction::TriggerKillSwitch,
    ] {
        let decision = evaluate_access(&ctx, action);
        assert!(!decision.allowed, "{action:?}");
        assert_eq!(decision.escalate_to, Some(Persona::Per01HumanGovernor));
    }
}

#[test]
fn a_agent_01_escalation_targets_human_governor_only() {
    let decision = escalation_for(EscalationTrigger::GovernorHalt);
    assert!(!decision.allowed);
    assert_eq!(decision.escalate_to, Some(Persona::Per01HumanGovernor));
}

#[test]
fn a_agent_02_maker_checker_blocks_live_limit_change() {
    let mut ctx = context(Persona::Per04RiskOfficer);
    ctx.maker_checker_approved = false;
    let decision = evaluate_access(&ctx, ToolAction::ChangeLiveLimit);
    assert!(!decision.allowed);
    assert_eq!(decision.escalate_to, Some(Persona::Per01HumanGovernor));
}

#[test]
fn per07_is_read_only_and_cannot_self_escalate() {
    let mut ctx = context(Persona::Per07Observer);
    ctx.requested_level = AutonomyLevel::L1ReadOnly;
    assert!(evaluate_access(&ctx, ToolAction::ReadLedger).allowed);
    assert!(!evaluate_access(&ctx, ToolAction::DraftSpec).allowed);
    ctx.requested_level = AutonomyLevel::L4ExecuteWithApproval;
    assert!(!evaluate_access(&ctx, ToolAction::ReadLedger).allowed);
}
