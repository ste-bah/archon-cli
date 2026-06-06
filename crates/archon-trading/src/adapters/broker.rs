use crate::TradingError;
use crate::order_intent::{OrderIntent, OrderType};
use crate::risk_governor::AccountState;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrokerOrderType {
    Market,
    Limit,
    Stop,
    StopLimit,
    Bracket,
    Oco,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub market: bool,
    pub limit: bool,
    pub stop: bool,
    pub stop_limit: bool,
    pub bracket: bool,
    pub oco: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrokerPosition {
    pub instrument: String,
    pub quantity: f64,
    pub average_price: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrokerOrderStatus {
    Requested,
    Accepted,
    Partial,
    Filled,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrokerOrder {
    pub broker_order_id: String,
    pub client_order_id: String,
    pub instrument: String,
    pub status: BrokerOrderStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrokerFill {
    pub broker_order_id: String,
    pub quantity: f64,
    pub price: f64,
    pub timestamp_unix_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrokerResponse {
    pub broker_order_id: String,
    pub status: BrokerOrderStatus,
    pub message: String,
    pub filled_quantity: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerHealth {
    pub healthy: bool,
    pub last_seen_seconds: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerError {
    UnsupportedOrderType,
    Reject(String),
    Timeout(String),
    Unavailable(String),
}

pub trait BrokerAdapter {
    fn name(&self) -> &str;
    fn capability_manifest(&self) -> &CapabilityManifest;
    fn account_state(&self) -> Result<AccountState, BrokerError>;
    fn positions(&self) -> Result<Vec<BrokerPosition>, BrokerError>;
    fn open_orders(&self) -> Result<Vec<BrokerOrder>, BrokerError>;
    fn submit(&mut self, intent: &OrderIntent) -> Result<BrokerResponse, BrokerError>;
    fn cancel(&mut self, broker_order_id: &str) -> Result<BrokerResponse, BrokerError>;
    fn replace(
        &mut self,
        broker_order_id: &str,
        replacement: &OrderIntent,
    ) -> Result<BrokerResponse, BrokerError>;
    fn fills(&self, broker_order_id: &str) -> Result<Vec<BrokerFill>, BrokerError>;
    fn health(&self) -> Result<BrokerHealth, BrokerError>;
}

impl CapabilityManifest {
    pub const fn all_disabled() -> Self {
        Self {
            market: false,
            limit: false,
            stop: false,
            stop_limit: false,
            bracket: false,
            oco: false,
        }
    }

    pub const fn supports(&self, order_type: BrokerOrderType) -> bool {
        match order_type {
            BrokerOrderType::Market => self.market,
            BrokerOrderType::Limit => self.limit,
            BrokerOrderType::Stop => self.stop,
            BrokerOrderType::StopLimit => self.stop_limit,
            BrokerOrderType::Bracket => self.bracket,
            BrokerOrderType::Oco => self.oco,
        }
    }

    pub fn require_supported(&self, intent: &OrderIntent) -> Result<(), TradingError> {
        let order_type = BrokerOrderType::from(intent.order_type);
        self.supports(order_type)
            .then_some(())
            .ok_or(TradingError::OrderTypeUnsupported)
    }
}

impl Default for CapabilityManifest {
    fn default() -> Self {
        Self {
            market: true,
            limit: true,
            stop: true,
            stop_limit: true,
            bracket: false,
            oco: false,
        }
    }
}

impl From<OrderType> for BrokerOrderType {
    fn from(value: OrderType) -> Self {
        match value {
            OrderType::Market => Self::Market,
            OrderType::Limit => Self::Limit,
            OrderType::Stop => Self::Stop,
            OrderType::StopLimit => Self::StopLimit,
        }
    }
}

impl BrokerError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedOrderType => "ORDER_TYPE_UNSUPPORTED",
            Self::Reject(_) | Self::Timeout(_) | Self::Unavailable(_) => "BROKER_REJECT",
        }
    }
}

impl std::fmt::Display for BrokerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedOrderType => formatter.write_str("ORDER_TYPE_UNSUPPORTED"),
            Self::Reject(message) => write!(formatter, "broker rejected order: {message}"),
            Self::Timeout(message) => write!(formatter, "broker timed out: {message}"),
            Self::Unavailable(message) => write!(formatter, "broker unavailable: {message}"),
        }
    }
}

impl std::error::Error for BrokerError {}
