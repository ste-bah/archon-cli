use crate::adapters::broker::{BrokerHealth, BrokerOrderStatus, BrokerResponse};
use crate::audit_ledger::OrderStatus;
use crate::order_intent::OrderIntent;
use serde::{Deserialize, Serialize};
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
    pub filled_quantity: Option<f64>,
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

impl ExecLedgerEntry {
    pub(super) fn new(
        client_order_id: &str,
        status: OrderStatus,
        intent: &OrderIntent,
        response: Option<BrokerResponse>,
        message: &str,
    ) -> Self {
        let broker_order_id = response.as_ref().map(|value| value.broker_order_id.clone());
        let filled_quantity = response.as_ref().map(|value| value.filled_quantity);
        Self {
            client_order_id: client_order_id.to_string(),
            timestamp_unix_ms: now_ms(),
            status,
            intent: intent.clone(),
            broker_order_id,
            broker_message: message.to_string(),
            filled_quantity,
            immutable_hash: String::new(),
        }
    }
}

impl TerminalHealthDecision {
    pub(super) fn halt(reason: String) -> Self {
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

pub(super) fn health_decision(health: BrokerHealth) -> TerminalHealthDecision {
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

pub(super) fn normalize_status(status: BrokerOrderStatus) -> OrderStatus {
    match status {
        BrokerOrderStatus::Requested => OrderStatus::Requested,
        BrokerOrderStatus::Accepted => OrderStatus::Accepted,
        BrokerOrderStatus::Partial => OrderStatus::Partial,
        BrokerOrderStatus::Filled => OrderStatus::Filled,
        BrokerOrderStatus::Rejected => OrderStatus::Rejected,
        BrokerOrderStatus::Cancelled => OrderStatus::Cancelled,
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}
