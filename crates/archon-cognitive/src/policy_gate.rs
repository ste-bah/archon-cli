use archon_policy::CognitivePolicy;
use serde::{Deserialize, Serialize};

use crate::{Candidate, CandidateActionKind, RiskLevel};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DenyReason {
    pub candidate_id: String,
    pub reason: String,
    pub rule_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalDenyReason {
    pub proposal_id: String,
    pub reason: String,
    pub policy_rule: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyVerdict {
    pub allowed: bool,
    pub reason: String,
    pub denied_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalCheck {
    pub proposal_id: String,
    pub touched_paths: Vec<String>,
    pub risk_level: RiskLevel,
    pub evidence_count: usize,
    pub recent_incidents: usize,
    pub rollback_available: bool,
}

#[derive(Debug, Clone)]
pub struct PolicyGate {
    policy: CognitivePolicy,
    policy_available: bool,
    min_evidence: usize,
    max_recent_incidents: usize,
}

impl PolicyGate {
    pub fn new(policy: Option<CognitivePolicy>) -> Self {
        let policy_available = policy.is_some();
        Self {
            policy: policy.unwrap_or_default(),
            policy_available,
            min_evidence: 3,
            max_recent_incidents: 0,
        }
    }

    pub fn filter(&self, candidates: Vec<Candidate>) -> (Vec<Candidate>, Vec<DenyReason>) {
        let mut allowed = Vec::new();
        let mut denied = Vec::new();
        for candidate in candidates {
            if let Some(reason) = self.deny_candidate(&candidate) {
                denied.push(reason);
            } else {
                allowed.push(candidate);
            }
        }
        (allowed, denied)
    }

    pub fn allow_autonomous_apply(&self, risk: RiskLevel) -> bool {
        self.policy_available
            && self.policy.can_auto_apply()
            && risk <= max_policy_risk(&self.policy)
            && risk < RiskLevel::High
    }

    pub fn deny_proposal(&self, proposal: &ProposalCheck) -> Option<ProposalDenyReason> {
        if let Some((rule, reason)) = forbidden_path_rule(&proposal.touched_paths) {
            return Some(proposal_deny(proposal, rule, reason));
        }
        if !self.allow_autonomous_apply(proposal.risk_level) {
            return Some(proposal_deny(
                proposal,
                "risk_exceeds_autonomous_policy",
                "proposal risk exceeds autonomous apply policy",
            ));
        }
        if proposal.evidence_count < self.min_evidence {
            return Some(proposal_deny(
                proposal,
                "insufficient_evidence",
                "proposal does not have enough supporting evidence",
            ));
        }
        if proposal.recent_incidents > self.max_recent_incidents {
            return Some(proposal_deny(
                proposal,
                "recent_incident_threshold_exceeded",
                "recent incidents require human review",
            ));
        }
        if !proposal.rollback_available {
            return Some(proposal_deny(
                proposal,
                "rollback_unavailable",
                "autonomous apply requires a rollback path",
            ));
        }
        None
    }

    pub fn verdict(&self, denied: &[DenyReason]) -> PolicyVerdict {
        PolicyVerdict {
            allowed: denied.is_empty(),
            reason: if denied.is_empty() {
                "all candidates passed policy".into()
            } else {
                "one or more candidates denied by policy".into()
            },
            denied_actions: denied
                .iter()
                .map(|reason| reason.candidate_id.clone())
                .collect(),
        }
    }

    fn deny_candidate(&self, candidate: &Candidate) -> Option<DenyReason> {
        if !self.policy_available {
            return conservative_candidate_rule(candidate);
        }
        if !self.policy.enabled {
            return None;
        }
        if let Some((rule, reason)) = forbidden_candidate_rule(candidate) {
            return Some(candidate_deny(candidate, rule, reason));
        }
        if candidate.risk_class >= RiskLevel::High {
            return Some(candidate_deny(
                candidate,
                "high_risk_requires_human",
                "high or critical risk action requires human approval",
            ));
        }
        if candidate.risk_class > max_policy_risk(&self.policy) {
            return Some(candidate_deny(
                candidate,
                "risk_exceeds_policy",
                "candidate risk exceeds configured policy maximum",
            ));
        }
        None
    }
}

fn conservative_candidate_rule(candidate: &Candidate) -> Option<DenyReason> {
    match candidate.action_kind {
        CandidateActionKind::AnswerDirectly | CandidateActionKind::AskClarification => None,
        _ => Some(candidate_deny(
            candidate,
            "policy_unavailable",
            "policy unavailable; only direct answer or clarification is allowed",
        )),
    }
}

fn forbidden_candidate_rule(candidate: &Candidate) -> Option<(&'static str, &'static str)> {
    let haystack = format!(
        "{} {} {}",
        candidate.expected_evidence,
        candidate.expected_user_output,
        candidate.tool_name.as_deref().unwrap_or("")
    )
    .to_ascii_lowercase();
    forbidden_text_rule(&haystack)
}

fn forbidden_path_rule(paths: &[String]) -> Option<(&'static str, &'static str)> {
    let haystack = paths.join(" ").to_ascii_lowercase();
    forbidden_text_rule(&haystack)
}

fn forbidden_text_rule(value: &str) -> Option<(&'static str, &'static str)> {
    for (needle, rule, reason) in FORBIDDEN_TOUCHES {
        if value.contains(needle) {
            return Some((*rule, *reason));
        }
    }
    None
}

fn max_policy_risk(policy: &CognitivePolicy) -> RiskLevel {
    match policy.max_autonomous_risk.as_str() {
        "Medium" | "medium" => RiskLevel::Medium,
        _ => RiskLevel::Low,
    }
}

fn candidate_deny(candidate: &Candidate, rule_name: &str, reason: &str) -> DenyReason {
    DenyReason {
        candidate_id: candidate.id.clone(),
        reason: reason.into(),
        rule_name: rule_name.into(),
    }
}

fn proposal_deny(proposal: &ProposalCheck, rule: &str, reason: &str) -> ProposalDenyReason {
    ProposalDenyReason {
        proposal_id: proposal.proposal_id.clone(),
        reason: reason.into(),
        policy_rule: rule.into(),
    }
}

const FORBIDDEN_TOUCHES: &[(&str, &str, &str)] = &[
    (
        "prompt",
        "prompt_mutation_forbidden",
        "prompt changes require human review",
    ),
    (
        "policy",
        "policy_mutation_forbidden",
        "policy changes require human review",
    ),
    (
        "network",
        "network_mutation_forbidden",
        "network changes require human review",
    ),
    (
        "blocking-gate",
        "blocking_gate_mutation_forbidden",
        "blocking gate changes require human review",
    ),
];
