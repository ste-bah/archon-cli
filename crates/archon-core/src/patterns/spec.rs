//! TASK-AGS-500: Shared pattern specification types.
//!
//! PatternSpec, CircuitBreakerConfig, FanOutConfig, BrokerConfig,
//! RemoteAgentConfig, and supporting enums.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::PatternKind;

/// Top-level specification for any pattern instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternSpec {
    /// Schema version for backward-compat (N and N-1 accepted).
    pub pattern_version: u32,
    /// Which pattern this spec targets.
    pub kind: PatternKind,
    /// Pattern-specific configuration (parsed by each concrete pattern).
    pub config: serde_json::Value,
    /// Per-invocation timeout. Default: 30 minutes (REQ-ARCH-006).
    #[serde(with = "duration_secs")]
    pub timeout: Duration,
    /// Circuit breaker settings applied to this pattern.
    pub circuit_breaker: CircuitBreakerConfig,
}

impl Default for PatternSpec {
    fn default() -> Self {
        Self {
            pattern_version: 1,
            kind: PatternKind::Pipeline,
            config: serde_json::Value::Null,
            timeout: Duration::from_secs(30 * 60), // 30 minutes per REQ-ARCH-006
            circuit_breaker: CircuitBreakerConfig::default(),
        }
    }
}

impl PatternSpec {
    /// Returns true if this spec's version is compatible with `current`.
    /// Accepts N and N-1 (NFR-ARCH-002).
    pub fn is_supported(&self, current: u32) -> bool {
        self.pattern_version == current
            || (current > 0 && self.pattern_version == current - 1)
    }
}

/// Circuit breaker configuration (TECH-AGS-PATTERNS lines 767-771).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures before tripping. Default: 3.
    pub failure_threshold: u32,
    /// Time to wait in Open state before transitioning to HalfOpen.
    #[serde(with = "duration_secs")]
    pub reset_after: Duration,
    /// Number of probe calls allowed in HalfOpen state. Default: 1.
    pub half_open_probes: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            reset_after: Duration::from_secs(60),
            half_open_probes: 1,
        }
    }
}

/// Fan-out/fan-in configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanOutConfig {
    /// Agent names to distribute work to.
    pub workers: Vec<String>,
    /// Agent name that merges results.
    pub aggregator: String,
    /// Optional CEL expression for input partitioning.
    pub partition_fn: Option<String>,
}

/// Broker configuration for capability-based selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerConfig {
    /// Candidate agent names.
    pub candidates: Vec<String>,
    /// Selection strategy.
    pub selector: BrokerSelector,
}

/// Broker selection strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BrokerSelector {
    RoundRobin,
    Capability,
    Cost,
    Custom(String),
}

/// Remote agent invocation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteAgentConfig {
    /// Transport protocol.
    pub transport: RemoteTransport,
    /// Target endpoint URL.
    pub endpoint: url::Url,
    /// Authentication configuration.
    pub auth: AuthConfig,
    /// Optional service discovery backend.
    pub service_discovery: Option<DiscoveryBackend>,
}

/// Transport protocol for remote agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RemoteTransport {
    Http,
    Grpc,
}

/// Authentication configuration for remote agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthConfig {
    /// No authentication.
    None,
    /// Bearer token.
    Bearer(String),
    /// Mutual TLS (placeholder — cert paths).
    Mtls {
        cert_path: String,
        key_path: String,
    },
}

/// Service discovery backend (placeholder).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiscoveryBackend {
    Consul { address: String },
    Etcd { endpoints: Vec<String> },
    DnsSrv { domain: String },
}

/// Serde helper: serialize Duration as seconds (u64).
mod duration_secs {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(Duration::from_secs(secs))
    }
}
