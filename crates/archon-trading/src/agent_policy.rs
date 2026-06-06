use crate::TradingError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Persona {
    Per01HumanGovernor,
    Per02ResearchAnalyst,
    Per03CodeBuilder,
    Per04RiskOfficer,
    Per05ExecutionAgent,
    Per06PostmortemAnalyst,
    Per07Observer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AutonomyLevel {
    L1ReadOnly,
    L2Draft,
    L3Propose,
    L4ExecuteWithApproval,
    L5Autonomous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradingPhase {
    P1Idea,
    P2Research,
    P3Backtest,
    P4Paper,
    P5LivePilot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ToolAction {
    ReadKb,
    WriteKb,
    DraftSpec,
    WriteRiskPolicy,
    GenerateCode,
    CompilePine,
    McpRead,
    McpWrite,
    SubmitPaperOrder,
    SubmitLiveOrder,
    PromoteStrategy,
    ChangeLiveLimit,
    BrokerCredentialWrite,
    TriggerKillSwitch,
    ReadLedger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelClass {
    General,
    Code,
    Reasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EscalationTrigger {
    LicensingDenial,
    ContradictionDetected,
    BrokerHealthHalt,
    GovernorHalt,
    UncertifiedMcpWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentContext {
    pub persona: Persona,
    pub requested_level: AutonomyLevel,
    pub phase: TradingPhase,
    pub model_class: ModelClass,
    pub maker_checker_approved: bool,
    pub certified_mcp_write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub effective_level: AutonomyLevel,
    pub reason: &'static str,
    pub escalate_to: Option<Persona>,
}

impl PolicyDecision {
    pub const fn allow(level: AutonomyLevel, reason: &'static str) -> Self {
        Self {
            allowed: true,
            effective_level: level,
            reason,
            escalate_to: None,
        }
    }

    pub const fn deny(level: AutonomyLevel, reason: &'static str) -> Self {
        Self {
            allowed: false,
            effective_level: level,
            reason,
            escalate_to: None,
        }
    }

    pub const fn escalate(level: AutonomyLevel, reason: &'static str) -> Self {
        Self {
            allowed: false,
            effective_level: level,
            reason,
            escalate_to: Some(Persona::Per01HumanGovernor),
        }
    }

    pub fn result(&self) -> Result<(), TradingError> {
        self.allowed.then_some(()).ok_or(TradingError::PolicyDenied)
    }
}

pub fn evaluate_access(context: &AgentContext, action: ToolAction) -> PolicyDecision {
    let effective_level = effective_level(context.persona, context.phase, context.requested_level);

    if context.requested_level > ceiling_for(context.persona, context.phase) {
        return PolicyDecision::deny(effective_level, "autonomy ceiling exceeded");
    }

    if !model_allowed(context.persona, context.model_class) {
        return PolicyDecision::deny(effective_level, "model class not permitted for persona");
    }

    if matches!(context.persona, Persona::Per07Observer) && !is_read_only(action) {
        return PolicyDecision::deny(effective_level, "PER-07 is read-only");
    }

    if matches!(context.persona, Persona::Per05ExecutionAgent)
        && matches!(action, ToolAction::WriteKb | ToolAction::WriteRiskPolicy)
    {
        return PolicyDecision::deny(effective_level, "PER-05 cannot write KB or risk policy");
    }

    if requires_maker_checker(action) && !context.maker_checker_approved {
        return PolicyDecision::escalate(effective_level, "maker-checker approval required");
    }

    if matches!(action, ToolAction::McpWrite) && !context.certified_mcp_write {
        return PolicyDecision::escalate(effective_level, "uncertified MCP write requires PER-01");
    }

    if is_granted(context.persona, action) && level_satisfies(effective_level, action) {
        PolicyDecision::allow(effective_level, "granted")
    } else {
        PolicyDecision::deny(effective_level, "not granted by fail-closed matrix")
    }
}

pub fn escalation_for(trigger: EscalationTrigger) -> PolicyDecision {
    match trigger {
        EscalationTrigger::LicensingDenial => PolicyDecision::escalate(
            AutonomyLevel::L1ReadOnly,
            "licensing denial requires human governor",
        ),
        EscalationTrigger::ContradictionDetected => PolicyDecision::escalate(
            AutonomyLevel::L1ReadOnly,
            "contradiction requires human governor",
        ),
        EscalationTrigger::BrokerHealthHalt => PolicyDecision::escalate(
            AutonomyLevel::L1ReadOnly,
            "broker health halt requires human governor",
        ),
        EscalationTrigger::GovernorHalt => PolicyDecision::escalate(
            AutonomyLevel::L1ReadOnly,
            "risk governor halt requires human governor",
        ),
        EscalationTrigger::UncertifiedMcpWrite => PolicyDecision::escalate(
            AutonomyLevel::L1ReadOnly,
            "uncertified MCP write requires human governor",
        ),
    }
}

pub const fn sla_reference(action: ToolAction) -> Option<&'static str> {
    match action {
        ToolAction::SubmitPaperOrder | ToolAction::SubmitLiveOrder => {
            Some("NFR-001 governor <=50ms")
        }
        ToolAction::CompilePine => Some("Pine compile <=30s"),
        _ => None,
    }
}

pub const fn postmortem_sla() -> &'static str {
    "postmortem ready <=1h after session close"
}

pub fn effective_level(
    persona: Persona,
    phase: TradingPhase,
    requested: AutonomyLevel,
) -> AutonomyLevel {
    requested.min(ceiling_for(persona, phase))
}

pub const fn ceiling_for(persona: Persona, phase: TradingPhase) -> AutonomyLevel {
    match persona {
        Persona::Per01HumanGovernor => AutonomyLevel::L5Autonomous,
        Persona::Per02ResearchAnalyst => AutonomyLevel::L3Propose,
        Persona::Per03CodeBuilder => AutonomyLevel::L3Propose,
        Persona::Per04RiskOfficer => AutonomyLevel::L4ExecuteWithApproval,
        Persona::Per05ExecutionAgent => match phase {
            TradingPhase::P3Backtest | TradingPhase::P4Paper => AutonomyLevel::L1ReadOnly,
            TradingPhase::P5LivePilot => AutonomyLevel::L4ExecuteWithApproval,
            _ => AutonomyLevel::L2Draft,
        },
        Persona::Per06PostmortemAnalyst => AutonomyLevel::L2Draft,
        Persona::Per07Observer => AutonomyLevel::L1ReadOnly,
    }
}

const fn is_read_only(action: ToolAction) -> bool {
    matches!(
        action,
        ToolAction::ReadKb | ToolAction::McpRead | ToolAction::ReadLedger
    )
}

const fn requires_maker_checker(action: ToolAction) -> bool {
    matches!(
        action,
        ToolAction::WriteKb
            | ToolAction::WriteRiskPolicy
            | ToolAction::McpWrite
            | ToolAction::SubmitPaperOrder
            | ToolAction::SubmitLiveOrder
            | ToolAction::PromoteStrategy
            | ToolAction::ChangeLiveLimit
            | ToolAction::BrokerCredentialWrite
            | ToolAction::TriggerKillSwitch
    )
}

const fn level_satisfies(level: AutonomyLevel, action: ToolAction) -> bool {
    match action {
        ToolAction::ReadKb | ToolAction::McpRead | ToolAction::ReadLedger => true,
        ToolAction::DraftSpec | ToolAction::GenerateCode | ToolAction::CompilePine => matches!(
            level,
            AutonomyLevel::L2Draft
                | AutonomyLevel::L3Propose
                | AutonomyLevel::L4ExecuteWithApproval
                | AutonomyLevel::L5Autonomous
        ),
        ToolAction::WriteKb
        | ToolAction::WriteRiskPolicy
        | ToolAction::McpWrite
        | ToolAction::SubmitPaperOrder
        | ToolAction::SubmitLiveOrder
        | ToolAction::PromoteStrategy
        | ToolAction::ChangeLiveLimit
        | ToolAction::BrokerCredentialWrite
        | ToolAction::TriggerKillSwitch => matches!(
            level,
            AutonomyLevel::L4ExecuteWithApproval | AutonomyLevel::L5Autonomous
        ),
    }
}

const fn model_allowed(persona: Persona, model_class: ModelClass) -> bool {
    match persona {
        Persona::Per02ResearchAnalyst | Persona::Per04RiskOfficer => {
            matches!(model_class, ModelClass::Reasoning)
        }
        Persona::Per03CodeBuilder => {
            matches!(model_class, ModelClass::Code | ModelClass::Reasoning)
        }
        _ => true,
    }
}

const fn is_granted(persona: Persona, action: ToolAction) -> bool {
    match persona {
        Persona::Per01HumanGovernor => true,
        Persona::Per02ResearchAnalyst => matches!(
            action,
            ToolAction::ReadKb
                | ToolAction::DraftSpec
                | ToolAction::McpRead
                | ToolAction::ReadLedger
        ),
        Persona::Per03CodeBuilder => matches!(
            action,
            ToolAction::ReadKb
                | ToolAction::GenerateCode
                | ToolAction::CompilePine
                | ToolAction::McpRead
        ),
        Persona::Per04RiskOfficer => matches!(
            action,
            ToolAction::ReadLedger
                | ToolAction::WriteRiskPolicy
                | ToolAction::PromoteStrategy
                | ToolAction::TriggerKillSwitch
        ),
        Persona::Per05ExecutionAgent => matches!(
            action,
            ToolAction::ReadLedger
                | ToolAction::SubmitPaperOrder
                | ToolAction::SubmitLiveOrder
                | ToolAction::McpRead
        ),
        Persona::Per06PostmortemAnalyst => matches!(
            action,
            ToolAction::ReadLedger | ToolAction::ReadKb | ToolAction::DraftSpec
        ),
        Persona::Per07Observer => is_read_only(action),
    }
}

#[cfg(test)]
#[path = "agent_policy_tests.rs"]
mod tests;
