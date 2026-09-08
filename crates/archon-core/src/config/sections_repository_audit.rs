//! Operator-selected audit allowances. Unlimited is explicit, never a zero sentinel.
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuditLimit {
    Finite(u64),
    #[default]
    Unlimited,
}
impl AuditLimit {
    pub fn finite(self) -> Option<u64> {
        match self { Self::Finite(n) => Some(n), Self::Unlimited => None }
    }
}
impl Serialize for AuditLimit {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Finite(n) => serializer.serialize_u64(*n),
            Self::Unlimited => serializer.serialize_str("unlimited"),
        }
    }
}
impl<'de> Deserialize<'de> for AuditLimit {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = AuditLimit;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a positive integer representable in milliseconds or the string \"unlimited\"")
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<Self::Value, E> {
                // Persisted elapsed time uses signed milliseconds. This is an
                // arithmetic representation bound, not an operational budget.
                if n == 0 || n > (i64::MAX as u64) / 1000 {
                    return Err(E::custom("audit limit must be positive and representable in milliseconds"));
                }
                Ok(AuditLimit::Finite(n))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<Self::Value, E> {
                self.visit_u64(u64::try_from(n).map_err(|_| E::custom("audit limit must be positive"))?)
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "unlimited" => Ok(AuditLimit::Unlimited),
                    _ => Err(E::custom("audit limit string must be exactly \"unlimited\"")),
                }
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RepositoryAuditConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempt_timeout_secs: Option<AuditLimit>,
    pub total_time_secs: AuditLimit,
    pub unexpected_change_refreshes: AuditLimit,
}
impl Default for RepositoryAuditConfig {
    fn default() -> Self {
        Self {
            attempt_timeout_secs: None,
            total_time_secs: AuditLimit::Unlimited,
            unexpected_change_refreshes: AuditLimit::Finite(3),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolvedRepositoryAuditConfig {
    pub attempt_timeout_secs: AuditLimit,
    pub total_time_secs: AuditLimit,
    pub unexpected_change_refreshes: AuditLimit,
    pub attempt_timeout_source: String,
}
impl RepositoryAuditConfig {
    pub fn resolve(&self, host_call_timeout_secs: u32) -> ResolvedRepositoryAuditConfig {
        ResolvedRepositoryAuditConfig {
            attempt_timeout_secs: self.attempt_timeout_secs.unwrap_or(AuditLimit::Finite(u64::from(host_call_timeout_secs))),
            total_time_secs: self.total_time_secs,
            unexpected_change_refreshes: self.unexpected_change_refreshes,
            attempt_timeout_source: if self.attempt_timeout_secs.is_some() {
                "workflow.repository_audit.attempt_timeout_secs"
            } else { "workflow.generated.host_call_timeout_secs" }.into(),
        }
    }
}
