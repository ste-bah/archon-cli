use crate::TradingError;
use crate::adapters::broker::{
    BrokerAdapter, BrokerError, BrokerHealth, BrokerOrderStatus, BrokerResponse,
};
use crate::audit_ledger::{AuditLedger, NewLedgerRecord, OrderStatus, TaxFields};
use crate::order_intent::{GatedOrderIntent, OrderIntent, TradingMode, pre_trade_gate};
use crate::risk_governor::{
    AccountState, MarketState, RiskDecisionStatus, RiskGovernor, RiskGovernorError,
};
use crate::risk_policy::RiskPolicy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

const HEALTH_TIMEOUT_SECONDS: u64 = 3;
const HALT_DEADLINE_MS: u128 = 1_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecLedgerEntry {
    pub client_order_id: String,
    pub timestamp_unix_ms: u128,
    pub status: OrderStatus,
    pub intent: OrderIntent,
    pub broker_order_id: Option<String>,
    pub broker_message: String,
    pub immutable_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalHealthDecision {
    pub healthy: bool,
    pub halt_required: bool,
    pub auto_resume_allowed: bool,
    pub poll_interval_ms: u64,
    pub halt_deadline_ms: u128,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveTerminalError {
    NonLiveIntent,
    Risk(String),
    RiskRejected(Option<&'static str>),
    UnsupportedOrderType,
    BrokerReject(String),
    Audit(String),
    HealthHalt(String),
}

pub struct LiveTerminal<A: BrokerAdapter> {
    adapter: A,
    governor: RiskGovernor,
    policy: RiskPolicy,
    audit: Option<AuditLedger>,
    ledger: Vec<ExecLedgerEntry>,
    next_order: u64,
}

impl<A: BrokerAdapter> LiveTerminal<A> {
    pub fn new(adapter: A, governor: RiskGovernor, policy: RiskPolicy) -> Self {
        Self {
            adapter,
            governor,
            policy,
            audit: None,
            ledger: Vec::new(),
            next_order: 1,
        }
    }

    pub fn with_audit(mut self, audit: AuditLedger) -> Self {
        self.audit = Some(audit);
        self
    }

    pub fn submit_order(
        &mut self,
        intent: OrderIntent,
        account: &AccountState,
        market: &MarketState,
    ) -> Result<GatedOrderIntent, LiveTerminalError> {
        if intent.mode != TradingMode::LivePilot {
            return Err(LiveTerminalError::NonLiveIntent);
        }
        self.adapter
            .capability_manifest()
            .require_supported(&intent)
            .map_err(map_support_error)?;
        let client_order_id = self.allocate_order_id();
        self.record_status(
            &client_order_id,
            OrderStatus::Requested,
            &intent,
            None,
            "requested",
        )?;
        let gated =
            pre_trade_gate(&self.governor, intent, account, market).map_err(risk_error_message)?;
        if gated.decision.status != RiskDecisionStatus::Approved {
            self.record_status(
                &client_order_id,
                OrderStatus::Rejected,
                &gated.intent,
                None,
                "risk rejected",
            )?;
            return Err(LiveTerminalError::RiskRejected(gated.decision.control_id));
        }
        let response = self.submit_once(&gated.intent, &client_order_id)?;
        self.record_broker_response(&client_order_id, &gated.intent, response)?;
        Ok(gated)
    }

    pub fn cancel_order(
        &mut self,
        broker_order_id: &str,
        intent: &OrderIntent,
    ) -> Result<(), LiveTerminalError> {
        let response = self
            .adapter
            .cancel(broker_order_id)
            .map_err(map_broker_reject)?;
        self.record_broker_response(broker_order_id, intent, response)
    }

    pub fn replace_order(
        &mut self,
        broker_order_id: &str,
        replacement: &OrderIntent,
    ) -> Result<(), LiveTerminalError> {
        self.adapter
            .capability_manifest()
            .require_supported(replacement)
            .map_err(map_support_error)?;
        let response = self
            .adapter
            .replace(broker_order_id, replacement)
            .map_err(map_broker_reject)?;
        self.record_broker_response(broker_order_id, replacement, response)
    }

    pub fn poll_health(&self) -> TerminalHealthDecision {
        match self.adapter.health() {
            Ok(health) => health_decision(health),
            Err(error) => TerminalHealthDecision::halt(error.to_string()),
        }
    }

    pub fn ledger(&self) -> &[ExecLedgerEntry] {
        &self.ledger
    }

    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    fn submit_once(
        &mut self,
        intent: &OrderIntent,
        client_order_id: &str,
    ) -> Result<BrokerResponse, LiveTerminalError> {
        self.adapter.submit(intent).map_err(|error| {
            let _ = self.record_status(
                client_order_id,
                OrderStatus::Rejected,
                intent,
                None,
                error.code(),
            );
            map_broker_reject(error)
        })
    }

    fn allocate_order_id(&mut self) -> String {
        let order_id = format!("live-{}", self.next_order);
        self.next_order += 1;
        order_id
    }

    fn record_broker_response(
        &mut self,
        client_order_id: &str,
        intent: &OrderIntent,
        response: BrokerResponse,
    ) -> Result<(), LiveTerminalError> {
        let status = normalize_status(response.status.clone());
        self.record_status(
            client_order_id,
            status,
            intent,
            Some(response.clone()),
            &response.message,
        )
    }

    fn record_status(
        &mut self,
        client_order_id: &str,
        status: OrderStatus,
        intent: &OrderIntent,
        response: Option<BrokerResponse>,
        message: &str,
    ) -> Result<(), LiveTerminalError> {
        let mut entry = ExecLedgerEntry::new(client_order_id, status, intent, response, message);
        entry.immutable_hash = entry_hash(&entry);
        if let Some(audit) = &self.audit {
            audit
                .append_status(self.audit_record(&entry))
                .map_err(|error| LiveTerminalError::Audit(error.code().to_string()))?;
        }
        self.ledger.push(entry);
        Ok(())
    }

    fn audit_record(&self, entry: &ExecLedgerEntry) -> NewLedgerRecord {
        NewLedgerRecord {
            actor: "live-terminal".to_string(),
            strategy_id: entry.intent.strategy_id.clone(),
            policy_version: self.policy.version_hash.clone(),
            status: entry.status,
            risk_decision: json!({"live_terminal": true}),
            order_intent: json!(&entry.intent),
            broker_response: json!({"broker_order_id": entry.broker_order_id, "message": entry.broker_message}),
            account: json!({"mode": "live", "adapter": self.adapter.name()}),
            tax: TaxFields::default(),
            artefacts: vec![entry.immutable_hash.as_bytes().to_vec()],
            maker_checker: None,
        }
    }
}

impl ExecLedgerEntry {
    fn new(
        client_order_id: &str,
        status: OrderStatus,
        intent: &OrderIntent,
        response: Option<BrokerResponse>,
        message: &str,
    ) -> Self {
        Self {
            client_order_id: client_order_id.to_string(),
            timestamp_unix_ms: now_ms(),
            status,
            intent: intent.clone(),
            broker_order_id: response.map(|value| value.broker_order_id),
            broker_message: message.to_string(),
            immutable_hash: String::new(),
        }
    }
}

impl TerminalHealthDecision {
    fn halt(reason: String) -> Self {
        Self {
            healthy: false,
            halt_required: true,
            auto_resume_allowed: false,
            poll_interval_ms: 1_000,
            halt_deadline_ms: HALT_DEADLINE_MS,
            reason,
        }
    }
}

fn health_decision(health: BrokerHealth) -> TerminalHealthDecision {
    if !health.healthy || health.last_seen_seconds > HEALTH_TIMEOUT_SECONDS {
        return TerminalHealthDecision::halt(health.message);
    }
    TerminalHealthDecision {
        healthy: true,
        halt_required: false,
        auto_resume_allowed: false,
        poll_interval_ms: 1_000,
        halt_deadline_ms: HALT_DEADLINE_MS,
        reason: health.message,
    }
}

fn normalize_status(status: BrokerOrderStatus) -> OrderStatus {
    match status {
        BrokerOrderStatus::Requested => OrderStatus::Requested,
        BrokerOrderStatus::Accepted => OrderStatus::Accepted,
        BrokerOrderStatus::Partial => OrderStatus::Partial,
        BrokerOrderStatus::Filled => OrderStatus::Filled,
        BrokerOrderStatus::Rejected => OrderStatus::Rejected,
        BrokerOrderStatus::Cancelled => OrderStatus::Cancelled,
    }
}

fn map_support_error(error: TradingError) -> LiveTerminalError {
    match error {
        TradingError::OrderTypeUnsupported => LiveTerminalError::UnsupportedOrderType,
        other => LiveTerminalError::BrokerReject(other.code().to_string()),
    }
}

fn map_broker_reject(error: BrokerError) -> LiveTerminalError {
    match error {
        BrokerError::UnsupportedOrderType => LiveTerminalError::UnsupportedOrderType,
        other => LiveTerminalError::BrokerReject(other.code().to_string()),
    }
}

fn risk_error_message(error: RiskGovernorError) -> LiveTerminalError {
    LiveTerminalError::Risk(format!("{error:?}"))
}

fn entry_hash(entry: &ExecLedgerEntry) -> String {
    let encoded = serde_json::to_vec(entry).unwrap_or_default();
    blake3::hash(&encoded).to_hex().to_string()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[cfg(test)]
#[path = "live_terminal_tests.rs"]
mod tests;
