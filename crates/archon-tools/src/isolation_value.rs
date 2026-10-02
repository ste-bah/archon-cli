//! Every value an `isolation` field may carry, parsed in one place (#236).
//!
//! The value used to be a free string read in three places that disagreed: the
//! workflow adapter sent `"workspace-boundary"`, the tier parser knew only the
//! ladder and returned `None` for it, and the spawn then fell back to the
//! automatic decision and recorded the tier it reached in place of the request.
//! The executor's later test for `"workspace-boundary"` could never match, so
//! the boundary it named was never applied, and nothing said so. One enum and
//! one parser close that: whoever sends a value and whoever reads it go through
//! [`Isolation`], and a value it does not know is an error naming the value and
//! where it came from.

use super::IsolationTier;

/// What a spawn asked for in its `isolation` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    /// A rung of the isolation ladder.
    Tier(IsolationTier),
    /// Confine the agent to the directory it was given.
    ///
    /// The agent inherits none of its parent's other directories. It reads
    /// only its working directory and the read roots its caller named, and it
    /// writes only its working directory and its declared write roots. This is
    /// a different control from the ladder: it creates no checkout and costs no
    /// disk, so `isolation_max_tier` never clamps it, and which rung the agent
    /// is on is still decided as if it had named none.
    WorkspaceBoundary,
}

/// The spelling of [`IsolationTier::Shared`] that older callers send. It is
/// accepted on input and never produced.
const SHARED_ALIAS: &str = "shared";

impl Isolation {
    /// Every value that is accepted, in the order a tool schema lists them.
    pub const ALL: [Isolation; 4] = [
        Isolation::Tier(IsolationTier::Shared),
        Isolation::Tier(IsolationTier::Worktree),
        Isolation::Tier(IsolationTier::WorktreeWithBuilds),
        Isolation::WorkspaceBoundary,
    ];

    /// The string a caller sends to ask for this value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tier(tier) => tier.as_str(),
            Self::WorkspaceBoundary => "workspace-boundary",
        }
    }

    /// Parse `raw`. `source` names where it came from, so that the error
    /// tells the operator which input to correct.
    pub fn parse(raw: &str, source: &str) -> Result<Self, IsolationParseError> {
        let value = raw.trim();
        if value == SHARED_ALIAS {
            return Ok(Self::Tier(IsolationTier::Shared));
        }
        Self::ALL
            .into_iter()
            .find(|isolation| isolation.as_str() == value)
            .ok_or_else(|| IsolationParseError {
                value: raw.to_string(),
                source: source.to_string(),
            })
    }

    /// The rung this value names, if it names one.
    pub fn tier(self) -> Option<IsolationTier> {
        match self {
            Self::Tier(tier) => Some(tier),
            Self::WorkspaceBoundary => None,
        }
    }

    /// Every accepted string, for a tool schema or an error message.
    pub fn accepted() -> Vec<&'static str> {
        Self::ALL.into_iter().map(Self::as_str).collect()
    }
}

/// An `isolation` value that no variant of [`Isolation`] spells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolationParseError {
    /// The value as it was received.
    pub value: String,
    /// Where it came from.
    pub source: String,
}

impl std::fmt::Display for IsolationParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown isolation value '{}' in {}; accepted values are {} ('{SHARED_ALIAS}' is \
             also read as '{}')",
            self.value,
            self.source,
            Isolation::accepted().join(", "),
            IsolationTier::Shared.as_str(),
        )
    }
}

impl std::error::Error for IsolationParseError {}
